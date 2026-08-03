// Core game plugin: startup logic and all gameplay systems. Separated from main.rs so
// integration tests can build the game without a window.
use avian2d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::{NavMeshHandle, prelude::*};
use geo::BoundingRect;
use rand::SeedableRng;

use rogue_angles::{
    AGENT_RADIUS, GameLayer, LevelEntity, WorldBounds,
    dungeon::level_generation::{LevelPlan, RoomRegistry},
    dungeon::terrain::{
        DungeonCollider, DungeonState, DungeonVisuals, GlassWallsMarker, SlowZoneMarker,
        TerrainMarker, geometry_to_collider, geometry_to_mesh, glass_wall_border,
        sync_dungeon_to_entities, sync_glass_walls_to_entities, update_slow_zone_multipliers,
    },
    effects::crumble_terrain::{Fragile, RubbleMaterial},
    fov::{self as fov, MOVABLE_Z, OpaqueVertices, TERRAIN_Z},
    nav::{self as nav, DungeonNavMesh, NavMeshIslandMarker, SLOW_ZONE_NAV_COST, playable_area_to_nav_mesh},
    status_effects::StatusEffectsAppExt,
    util::safegeo::SafeMultiPolygon,
    visuals::indicator::update_hit_flash,
};

use crate::{
    AmuletOfWinning, DungeonDepth, GameState, Staircase, WORLD_HEIGHT, WORLD_WIDTH,
    dig_shot::{DigShotAssets, fire_dig_shot, init_dig_shot_assets, update_dig_shots},
    monster,
    player::{
        Player, Stats, advance_exploration, check_player_death, move_player,
        rotate_player_to_velocity, set_target_on_click, tick_fire_cooldown,
    },
    populate_level,
    status_effect::{StatusEffect, sync_movement_modifiers},
    ui::UiPlugin,
};

/// The dungeon seed for the current game session. Set fresh (from OS randomness) each time
/// `GameState::Restart` is entered, unless a `DungeonSeedOverride` resource is present.
#[derive(Resource)]
pub struct DungeonSeed(pub u64);

/// Optional resource: forces every restart to (re)use this fixed seed instead of a random
/// one. Insert this in tests that need a deterministic dungeon.
#[derive(Resource)]
pub struct DungeonSeedOverride(pub u64);

/// Player HP carried across a descend (there's no inventory to carry, unlike vector-arena).
#[derive(Resource, Default)]
pub struct SavedPlayer(pub Option<(f32, f32)>);

fn despawn_level_entities(mut commands: Commands, query: Query<Entity, With<LevelEntity>>) {
    for entity in query.iter() {
        commands.entity(entity).try_despawn();
    }
}

fn on_enter_restart(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut nav_meshes: ResMut<Assets<NavMesh2d>>,
    mut depth: ResMut<DungeonDepth>,
    mut message_log: ResMut<rogue_angles::hud::MessageLog>,
    mut next_state: ResMut<NextState<GameState>>,
    seed_override: Option<Res<DungeonSeedOverride>>,
) {
    let seed_val = match seed_override.as_deref() {
        Some(s) => s.0,
        None => rand::random::<u64>(),
    };
    commands.insert_resource(DungeonSeed(seed_val));
    message_log.clear();
    depth.0 = 1;
    spawn_game_world(&mut commands, &mut meshes, &mut materials, &mut nav_meshes, 1, None, seed_val);
    next_state.set(GameState::InLevel);
}

fn on_enter_descend(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut nav_meshes: ResMut<Assets<NavMesh2d>>,
    mut depth: ResMut<DungeonDepth>,
    mut message_log: ResMut<rogue_angles::hud::MessageLog>,
    saved_player: Res<SavedPlayer>,
    mut next_state: ResMut<NextState<GameState>>,
    seed: Res<DungeonSeed>,
) {
    depth.0 += 1;
    message_log.push(format!("You descend to depth {}.", depth.0));
    spawn_game_world(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut nav_meshes,
        depth.0,
        saved_player.0,
        seed.0,
    );
    next_state.set(GameState::InLevel);
}

fn save_player_on_exit(player_query: Query<&Stats, With<Player>>, mut saved: ResMut<SavedPlayer>) {
    saved.0 = player_query.single().ok().map(|s| (s.hp, s.max_hp));
}

/// Touching the staircase descends; touching the amulet (depth `FINAL_DEPTH` only) wins.
/// Both trigger automatically on proximity — there's no inventory/menu step in the way.
const EXIT_TOUCH_RANGE: f32 = 20.0;

fn check_level_exit(
    player_query: Query<&Transform, With<Player>>,
    staircase_query: Query<&Transform, With<Staircase>>,
    amulet_query: Query<&Transform, With<AmuletOfWinning>>,
    mut next_state: ResMut<NextState<GameState>>,
) {
    let Ok(player_tf) = player_query.single() else { return };
    let player_pos = player_tf.translation.truncate();

    if let Ok(stair_tf) = staircase_query.single()
        && player_pos.distance(stair_tf.translation.truncate()) < EXIT_TOUCH_RANGE
    {
        next_state.set(GameState::Descend);
    }
    if let Ok(amulet_tf) = amulet_query.single()
        && player_pos.distance(amulet_tf.translation.truncate()) < EXIT_TOUCH_RANGE
    {
        next_state.set(GameState::Won);
    }
}

pub fn spawn_game_world(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    nav_meshes: &mut Assets<NavMesh2d>,
    depth: u32,
    saved_player_hp: Option<(f32, f32)>,
    seed: u64,
) {
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed + depth as u64);

    let mut archipelago = Archipelago2d::new(ArchipelagoOptions::from_agent_radius(AGENT_RADIUS));
    archipelago.set_type_index_cost(1, SLOW_ZONE_NAV_COST).expect("slow zone nav cost is positive");
    let archipelago_id = commands.spawn((LevelEntity, archipelago)).id();

    let registry = RoomRegistry::stock();
    let terrain_geometry = LevelPlan::new_seeded(WORLD_WIDTH, WORLD_HEIGHT, &registry, &mut rng);
    let empty_polys: Vec<rogue_angles::util::safegeo::SafePolygon> = Vec::new();
    let slow_zones = terrain_geometry.regions.get("slow").unwrap_or(&empty_polys);

    let terrain_mesh = geometry_to_mesh(&terrain_geometry.solid_rock);
    let terrain_collider = geometry_to_collider(&terrain_geometry.solid_rock);
    let valid_nav_mesh = playable_area_to_nav_mesh(&terrain_geometry.playable_area, slow_zones);

    let terrain_mesh_handle = meshes.add(terrain_mesh);
    let nav_mesh_handle = nav_meshes.add(NavMesh2d { nav_mesh: valid_nav_mesh });

    let glass_border = glass_wall_border(&terrain_geometry.glass_walls);
    let glass_border_mesh_handle = meshes.add(geometry_to_mesh(&glass_border));
    let glass_collider = geometry_to_collider(&terrain_geometry.glass_walls);

    let dungeon_state = DungeonState {
        solid_rock: terrain_geometry.solid_rock.clone(),
        playable_area: terrain_geometry.playable_area.clone(),
        glass_walls: terrain_geometry.glass_walls.clone(),
        slow_zones: slow_zones.clone(),
    };
    commands.insert_resource(dungeon_state);
    commands.insert_resource(DungeonVisuals(terrain_mesh_handle.clone()));
    commands.insert_resource(DungeonCollider(terrain_collider.clone()));
    commands.insert_resource(DungeonNavMesh(nav_mesh_handle.clone()));

    let terrain_entity = commands
        .spawn((
            LevelEntity,
            TerrainMarker,
            Mesh2d(terrain_mesh_handle),
            MeshMaterial2d(materials.add(ColorMaterial::from(Color::srgb(0.2, 0.2, 0.2)))),
            Transform::from_translation(Vec3::new(0.0, 0.0, TERRAIN_Z)),
            terrain_collider,
            RigidBody::Static,
            CollisionLayers::new(GameLayer::Wall, [
                GameLayer::Wall,
                GameLayer::Dynamic,
                GameLayer::Missile,
            ]),
        ))
        .id();

    commands.spawn((
        LevelEntity,
        GlassWallsMarker,
        Mesh2d(glass_border_mesh_handle),
        MeshMaterial2d(materials.add(ColorMaterial::from(Color::srgba(0.5, 0.85, 0.9, 1.0)))),
        Transform::from_translation(Vec3::new(0.0, 0.0, TERRAIN_Z + 0.1)),
        glass_collider,
        RigidBody::Static,
        CollisionLayers::new(GameLayer::Wall, [
            GameLayer::Wall,
            GameLayer::Dynamic,
            GameLayer::Missile,
        ]),
    ));

    let slow_zone_material = materials.add(ColorMaterial::from(Color::srgba(0.3, 0.7, 1.0, 0.25)));
    for zone in slow_zones {
        let mesh = geometry_to_mesh(&SafeMultiPolygon::from(zone.clone()));
        let bbox = zone.bounding_rect().expect("slow zone is non-empty");
        let center =
            Vec2::new((bbox.min().x + bbox.max().x) / 2.0, (bbox.min().y + bbox.max().y) / 2.0);
        let half_size = Vec2::new(bbox.width() / 2.0, bbox.height() / 2.0);
        commands.spawn((
            LevelEntity,
            Mesh2d(meshes.add(mesh)),
            MeshMaterial2d(slow_zone_material.clone()),
            Transform::from_translation(Vec3::new(0.0, 0.0, TERRAIN_Z + 0.1)),
            SlowZoneMarker { center, half_size },
        ));
    }

    let door_material = materials.add(ColorMaterial::from(Color::srgb(0.5, 0.25, 0.1)));
    for door in &terrain_geometry.doors {
        let center = door.center();
        let disp = door.disp_size();

        let door_entity = commands
            .spawn((
                LevelEntity,
                Fragile,
                OpaqueVertices(door.disp_corners()),
                Mesh2d(meshes.add(Rectangle::new(disp.x, disp.y))),
                MeshMaterial2d(door_material.clone()),
                Transform::from_translation(center.extend(MOVABLE_Z)),
                RigidBody::Dynamic,
                door.collider(),
                CollisionLayers::new(GameLayer::Dynamic, [GameLayer::Wall, GameLayer::Dynamic]),
            ))
            .id();

        let hinge = door.hinge_vec();
        let joint_entity = commands
            .spawn((
                LevelEntity,
                RevoluteJoint::new(door_entity, terrain_entity)
                    .with_local_anchor1(hinge - center)
                    .with_local_anchor2(hinge),
            ))
            .id();
        commands.entity(door_entity).add_child(joint_entity);
    }

    commands.insert_resource(WorldBounds { width: WORLD_WIDTH, height: WORLD_HEIGHT });
    let rubble_material = materials.add(ColorMaterial::from(Color::srgb(0.5, 0.5, 0.5)));
    commands.insert_resource(RubbleMaterial(rubble_material));

    fov::spawn_fov_meshes(commands, meshes, materials, WORLD_WIDTH, WORLD_HEIGHT);

    commands.spawn((LevelEntity, NavMeshIslandMarker, Island2dBundle {
        island: Island,
        archipelago_ref: ArchipelagoRef2d::new(archipelago_id),
        nav_mesh: NavMeshHandle(nav_mesh_handle),
    }));

    populate_level::populate(
        commands,
        meshes,
        materials,
        &terrain_geometry.playable_area,
        archipelago_id,
        depth,
        saved_player_hp,
        &mut rng,
    );
}

/// The core game plugin. Set `headless = true` to skip egui-dependent plugins/systems for
/// tests — gameplay logic runs identically either way.
pub struct GamePlugin {
    pub headless: bool,
}

impl Default for GamePlugin {
    fn default() -> Self { Self { headless: false } }
}

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((avian2d::PhysicsPlugins::default(), Landmass2dPlugin::default()));

        if !self.headless {
            app.add_plugins(UiPlugin);
        } else {
            app.init_resource::<rogue_angles::hud::MessageLog>();
        }

        app.init_state::<GameState>()
            .init_resource::<DungeonDepth>()
            .init_resource::<SavedPlayer>()
            .add_systems(Startup, init_dig_shot_assets)
            .add_systems(
                OnEnter(GameState::Restart),
                (despawn_level_entities, on_enter_restart).chain(),
            )
            .add_systems(
                OnEnter(GameState::Descend),
                (despawn_level_entities, on_enter_descend).chain(),
            )
            .add_systems(OnExit(GameState::InLevel), save_player_on_exit)
            .add_status_effects::<StatusEffect>()
            .add_systems(Update, update_slow_zone_multipliers)
            .add_systems(Update, set_target_on_click.run_if(in_state(GameState::InLevel)))
            .add_systems(Update, move_player.after(update_slow_zone_multipliers))
            .add_systems(
                Update,
                nav::apply_nav_velocity.after(move_player).after(sync_movement_modifiers),
            )
            .add_systems(Update, rotate_player_to_velocity.after(move_player))
            .add_systems(Update, advance_exploration.after(move_player))
            .add_systems(Update, tick_fire_cooldown)
            .add_systems(
                Update,
                fire_dig_shot.run_if(resource_exists::<DigShotAssets>.and(in_state(GameState::InLevel))),
            )
            .add_systems(Update, update_dig_shots.after(fire_dig_shot))
            .add_systems(Update, check_player_death.run_if(in_state(GameState::InLevel)))
            .add_systems(Update, monster::update_monster_ai)
            .add_systems(Update, monster::refresh_monster_tooltips.after(monster::update_monster_ai))
            .add_systems(Update, nav::sync_island_nav_mesh)
            .add_systems(
                Update,
                sync_movement_modifiers.after(update_slow_zone_multipliers),
            )
            .add_systems(Update, fov::update_fov.after(sync_movement_modifiers))
            .add_systems(Update, sync_dungeon_to_entities)
            .add_systems(Update, sync_glass_walls_to_entities)
            .add_systems(Update, update_hit_flash)
            .add_systems(
                Update,
                check_level_exit.after(move_player).run_if(in_state(GameState::InLevel)),
            );
    }
}
