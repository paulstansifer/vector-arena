// Core game plugin: all gameplay systems and startup logic.
// Separated from main.rs so integration tests can build the game without a window.
use avian2d::prelude::*;
use bevy::prelude::*;
use bevy_egui::{EguiPrimaryContextPass, input::egui_wants_any_pointer_input};
use bevy_landmass::{NavMeshHandle, prelude::*};
use geo::BoundingRect;
use rand::SeedableRng;

use crate::{
    AGENT_RADIUS, DungeonDepth, GameLayer, GameState, WORLD_HEIGHT, WORLD_WIDTH, WorldBounds,
    command_palette::{CommandPalettePlugin, LetterMap, open_palette_system},
    dungeon::{
        level_generation::TerrainGeometry,
        terrain::{
            DungeonCollider, DungeonState, DungeonVisuals, PointsOfInterest, TerrainMarker,
            TorporZoneParticles, geometry_to_collider, geometry_to_mesh, sync_dungeon_to_entities,
            update_torpor_multipliers,
        },
    },
    effects::{
        self, crumble_terrain::{Fragile, RubbleMaterial, handle_right_click_excavation},
        projectile::{
            apply_damage_on_hit, apply_dodge, apply_hit_flash_on_hit, apply_knockback_on_hit,
            apply_torpor_to_non_agents, detect_missile_hits, execute_missile_command,
            init_trail_meshes, monster_fire_missiles, register_missile_command,
            spawn_missile_trails, tick_knockback_cooldowns, update_missile_trails, update_missiles,
        },
        rope,
        scroll::{on_acquirement, on_magic_mapping, on_monster_confusion, on_summon_monster},
    },
    fov::{self, MOVABLE_Z, OpaqueVertices, TERRAIN_Z},
    goto,
    indicator::{render_state_indicators, tick_state_indicators, update_hit_flash},
    item::{
        Inventory, ItemIdentities, animate_pickup, execute_item_command, pickup_items,
        refresh_item_tooltips, register_item_commands,
    },
    monster::{self, Stats},
    nav::{self, DungeonNavMesh, NavMeshIslandMarker, TORPOR_NAV_COST, playable_area_to_nav_mesh},
    objects::{animate_sigil, detect_sigil_contact, explode_sigil, tick_sigil_explosions},
    player::{
        Player, advance_exploration, directional_move_system, execute_descend_command,
        execute_stop_command, move_player, register_player_commands, rotate_player_to_velocity,
        set_target_on_click,
    },
    populate_level,
    sprite::SpritePlugin,
    status_effect::{apply_confusion_to_velocity, tick_status_effects},
    time_scale::manage_time_scale,
    ui::{MessageLog, UiPlugin, enable_ui_input_absorption},
};

/// Optional resource: when present, dungeon generation uses this seed instead of thread_rng.
#[derive(Resource)]
pub struct DungeonSeed(pub u64);

#[derive(Resource, Default)]
pub struct SavedPlayer(pub Option<(Stats, Inventory)>);

fn on_enter_restart(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut nav_meshes: ResMut<Assets<NavMesh2d>>,
    mut depth: ResMut<DungeonDepth>,
    mut message_log: ResMut<MessageLog>,
    mut next_state: ResMut<NextState<GameState>>,
    mut monster_letters: ResMut<LetterMap>,
    mut identities: ResMut<ItemIdentities>,
    seed: Option<Res<DungeonSeed>>,
) {
    let seed_val = match seed.as_deref() {
        Some(s) => s.0,
        None => {
            let s = rand::random::<u64>();
            commands.insert_resource(DungeonSeed(s));
            s
        }
    };
    message_log.clear();
    depth.0 = 1;
    monster_letters.clear_monsters();
    identities.randomize(&mut rand::rngs::StdRng::seed_from_u64(seed_val));
    spawn_game_world(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut nav_meshes,
        1,
        None,
        &mut monster_letters,
        seed_val,
    );
    next_state.set(GameState::InLevel);
}

fn on_enter_descend(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut nav_meshes: ResMut<Assets<NavMesh2d>>,
    mut depth: ResMut<DungeonDepth>,
    mut message_log: ResMut<MessageLog>,
    saved_player: Res<SavedPlayer>,
    mut next_state: ResMut<NextState<GameState>>,
    mut monster_letters: ResMut<LetterMap>,
    seed: Res<DungeonSeed>,
) {
    depth.0 += 1;
    message_log.push(format!("You descend to depth {}.", depth.0));
    monster_letters.clear_monsters();
    spawn_game_world(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut nav_meshes,
        depth.0,
        saved_player.0.as_ref().map(|(stats, inv)| (*stats, Inventory(inv.0.clone()))),
        &mut monster_letters,
        seed.0,
    );
    next_state.set(GameState::InLevel);
}

fn save_player_on_exit(
    player_data: Query<(&Stats, &Inventory), With<Player>>,
    mut saved_player: ResMut<SavedPlayer>,
) {
    saved_player.0 =
        player_data.single().ok().map(|(stats, inv)| (*stats, Inventory(inv.0.clone())));
}

pub fn spawn_game_world(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    nav_meshes: &mut Assets<NavMesh2d>,
    depth: u32,
    saved_player: Option<(Stats, Inventory)>,
    monster_letters: &mut LetterMap,
    seed: u64,
) {
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed + depth as u64);

    let mut archipelago = Archipelago2d::new(ArchipelagoOptions::from_agent_radius(AGENT_RADIUS));
    archipelago.set_type_index_cost(1, TORPOR_NAV_COST).expect("torpor nav cost is positive");
    let archipelago_id = commands.spawn((DespawnOnExit(GameState::InLevel), archipelago)).id();

    let terrain_geometry = TerrainGeometry::new_seeded(WORLD_WIDTH, WORLD_HEIGHT, &mut rng);

    let terrain_mesh = geometry_to_mesh(&terrain_geometry.solid_rock);
    let terrain_collider = geometry_to_collider(&terrain_geometry.solid_rock);
    let valid_nav_mesh =
        playable_area_to_nav_mesh(&terrain_geometry.playable_area, &terrain_geometry.torpor_zones);

    let terrain_mesh_handle = meshes.add(terrain_mesh);
    let nav_mesh_handle = nav_meshes.add(NavMesh2d { nav_mesh: valid_nav_mesh });

    let dungeon_state = DungeonState {
        solid_rock: terrain_geometry.solid_rock.clone(),
        playable_area: terrain_geometry.playable_area.clone(),
        torpor_zones: terrain_geometry.torpor_zones.clone(),
    };
    let dungeon_visuals = DungeonVisuals(terrain_mesh_handle.clone());
    let dungeon_collider = DungeonCollider(terrain_collider.clone());
    let dungeon_nav_mesh = DungeonNavMesh(nav_mesh_handle.clone());

    commands.insert_resource(dungeon_state);
    commands.insert_resource(dungeon_visuals);
    commands.insert_resource(dungeon_collider);
    commands.insert_resource(dungeon_nav_mesh);

    let terrain_entity = commands
        .spawn((
            DespawnOnExit(GameState::InLevel),
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
                GameLayer::Rope,
            ]),
        ))
        .id();

    let torpor_material =
        materials.add(ColorMaterial::from(Color::Srgba(effects::torpor_particles::TORPOR_ZONE_COLOR)));
    for zone in &terrain_geometry.torpor_zones {
        let mesh = geometry_to_mesh(&geo::MultiPolygon::new(vec![zone.clone()]));
        // Bounding box of the zone, for seeding ambient particles. For today's
        // rectangular zones this matches the zone exactly.
        let bbox = zone.bounding_rect().expect("torpor zone is non-empty");
        let center = Vec2::new((bbox.min().x + bbox.max().x) / 2.0, (bbox.min().y + bbox.max().y) / 2.0);
        let half_size = Vec2::new(bbox.width() / 2.0, bbox.height() / 2.0);
        commands.spawn((
            DespawnOnExit(GameState::InLevel),
            Mesh2d(meshes.add(mesh)),
            MeshMaterial2d(torpor_material.clone()),
            Transform::from_translation(Vec3::new(0.0, 0.0, TERRAIN_Z + 0.1)),
            TorporZoneParticles { center, half_size },
        ));
    }

    let door_material = materials.add(ColorMaterial::from(Color::srgb(0.5, 0.25, 0.1)));
    for door in &terrain_geometry.doors {
        let center = door.center();
        let disp = door.disp_size();

        let door_entity = commands
            .spawn((
                DespawnOnExit(GameState::InLevel),
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
                DespawnOnExit(GameState::InLevel),
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

    commands.spawn((DespawnOnExit(GameState::InLevel), NavMeshIslandMarker, Island2dBundle {
        island: Island,
        archipelago_ref: ArchipelagoRef2d::new(archipelago_id),
        nav_mesh: NavMeshHandle(nav_mesh_handle),
    }));

    populate_level::populate(
        commands,
        meshes,
        materials,
        &terrain_geometry.rooms,
        &terrain_geometry.playable_area,
        archipelago_id,
        depth,
        saved_player,
        monster_letters,
        &mut rng,
    );

    let mut poi_points: Vec<Vec2> = terrain_geometry
        .rooms
        .iter()
        .map(|r| {
            let c = r.center();
            Vec2::new(c.x, c.y)
        })
        .collect();
    poi_points.extend(terrain_geometry.corridor_ends.iter().copied());
    commands.insert_resource(PointsOfInterest { points: poi_points });
}

/// The core game plugin.
///
/// Set `headless = true` to skip all egui-dependent plugins/systems (UiPlugin,
/// CommandPalettePlugin, SpritePlugin, SvgPlugin). In that mode the resources
/// those plugins would have provided are initialized manually so gameplay
/// systems that depend on them (palette commands, item logic, etc.) still work.
pub struct GamePlugin {
    pub headless: bool,
}

impl Default for GamePlugin {
    fn default() -> Self { Self { headless: false } }
}

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        // Physics and navigation are always included.
        app.add_plugins((
            avian2d::PhysicsPlugins::default(),
            Landmass2dPlugin::default(),
            rope::RopePlugin,
        ));

        if self.headless {
            // In headless mode, skip egui-dependent plugins and stub their resources.
            // SvgPlugin + insert_svg_components are kept so SVG sprites (player, items) render.
            // Only the egui texture-registration systems are dropped.
            use crate::{
                command_palette::{
                    CommandPaletteState, CommandPaletteWatchesClicks, PaletteRegistry,
                },
                sprite::{SpriteCache, SpriteEguiTextures, insert_svg_components},
                ui::MessageLog,
            };
            app.add_plugins(bevy_svg::prelude::SvgPlugin)
                .init_resource::<MessageLog>()
                .init_resource::<CommandPaletteState>()
                .init_resource::<PaletteRegistry>()
                .init_resource::<CommandPaletteWatchesClicks>()
                .init_resource::<SpriteCache>()
                .init_resource::<SpriteEguiTextures>()
                .add_systems(Update, insert_svg_components);
        } else {
            app.add_plugins((
                bevy_svg::prelude::SvgPlugin,
                UiPlugin,
                CommandPalettePlugin,
                SpritePlugin,
            ));
        }

        app.init_state::<GameState>()
            .init_resource::<SavedPlayer>()
            .init_resource::<LetterMap>()
            .init_resource::<ItemIdentities>();

        if !self.headless {
            app.add_systems(Startup, enable_ui_input_absorption);
        }

        app.add_systems(Startup, init_trail_meshes)
            .add_systems(Startup, register_item_commands)
            .add_systems(Startup, goto::register_goto_command)
            .add_systems(Startup, register_player_commands)
            .add_systems(Startup, register_missile_command)
            .add_systems(OnEnter(GameState::Restart), on_enter_restart)
            .add_systems(OnEnter(GameState::Descend), on_enter_descend)
            .add_systems(OnExit(GameState::InLevel), save_player_on_exit)
            .add_systems(Update, tick_status_effects)
            .add_systems(Update, update_torpor_multipliers)
            .add_systems(Update, execute_item_command)
            .add_systems(Update, refresh_item_tooltips)
            .add_observer(on_summon_monster)
            .add_observer(on_magic_mapping)
            .add_observer(on_monster_confusion)
            .add_observer(on_acquirement)
            .add_systems(Update, set_target_on_click)
            .add_systems(Update, move_player.after(update_torpor_multipliers))
            .add_systems(Update, nav::apply_nav_velocity.after(move_player))
            .add_systems(Update, directional_move_system.after(nav::apply_nav_velocity).after(open_palette_system))
            .add_systems(Update, rotate_player_to_velocity.after(move_player))
            .add_systems(Update, advance_exploration.after(move_player))
            .add_systems(Update, execute_stop_command)
            .add_systems(Update, execute_descend_command)
            .add_systems(Update, monster::update_monster_ai)
            .add_systems(
                Update,
                monster::refresh_monster_tooltips.after(monster::update_monster_ai),
            )
            .add_systems(
                Update,
                apply_confusion_to_velocity
                    .after(nav::apply_nav_velocity)
                    .after(directional_move_system),
            )
            .add_systems(Update, nav::sync_island_nav_mesh)
            .add_systems(Update, fov::update_fov)
            .add_systems(Update, fov::update_staircase_fog_copy.after(fov::update_fov))
            .add_systems(
                Update,
                if self.headless {
                    handle_right_click_excavation.into_configs()
                } else {
                    handle_right_click_excavation
                        .run_if(not(egui_wants_any_pointer_input))
                        .into_configs()
                },
            )
            .add_systems(Update, sync_dungeon_to_entities)
            .add_systems(Update, execute_missile_command)
            .add_systems(Update, monster_fire_missiles)
            .add_systems(Update, update_missiles)
            .add_systems(
                Update,
                apply_torpor_to_non_agents.after(update_torpor_multipliers).after(update_missiles),
            )
            .add_systems(Update, spawn_missile_trails)
            .add_systems(Update, update_missile_trails)
            .add_observer(apply_knockback_on_hit)
            .add_observer(apply_dodge)
            .add_observer(apply_hit_flash_on_hit)
            .add_observer(apply_damage_on_hit)
            .add_systems(Update, update_hit_flash.before(detect_missile_hits))
            .add_systems(Update, tick_state_indicators);

        if !self.headless {
            app.add_systems(EguiPrimaryContextPass, render_state_indicators);
        }

        app.add_systems(Update, detect_missile_hits.before(spawn_missile_trails))
            .add_systems(Update, tick_knockback_cooldowns)
            .add_systems(Update, animate_sigil)
            .add_systems(Update, detect_sigil_contact)
            .add_systems(Update, explode_sigil.after(detect_sigil_contact))
            .add_systems(Update, tick_sigil_explosions)
            .add_systems(Update, pickup_items)
            .add_systems(Update, animate_pickup)
            .add_systems(Update, manage_time_scale.after(move_player))
            .add_systems(Update, goto::compute_goto_assignments)
            .add_systems(Update, goto::reset_goto_on_close)
            .add_systems(Update, goto::execute_goto_command)
            .init_resource::<goto::GotoState>()
            .insert_resource(Gravity::ZERO)
            .insert_resource(SubstepCount(40))
            .init_resource::<DungeonDepth>();
    }
}
