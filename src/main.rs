use avian2d::prelude::*;
use bevy::prelude::*;
use bevy_egui::input::egui_wants_any_pointer_input;
use bevy_landmass::{NavMeshHandle, prelude::*};

use vector_arena::{
    AGENT_RADIUS, DungeonDepth, GameState, WorldBounds,
    effects::{projectile, rope},
    fov, item,
    monster::Stats,
    nav, player,
    ui::{self, enable_ui_input_absorption},
};

use fov::{Opaque, OpaqueVertices};
use item::{Inventory, animate_pickup, pickup_items};
use player::{Player, move_player, set_target_on_click};
use projectile::{
    apply_missile_knockback, init_trail_meshes, manage_time_scale, monster_fire_missiles,
    player_fire_missile, spawn_missile_trails, tick_knockback_cooldowns, update_hit_flash,
    update_missile_trails, update_missiles,
};
use ui::{MessageLog, UiPlugin};
use vector_arena::{
    GameLayer,
    dungeon::{
        level_generation::TerrainGeometry,
        terrain::{
            DungeonCollider, DungeonState, DungeonVisuals, TerrainMarker, geometry_to_collider,
            geometry_to_mesh, sync_dungeon_to_entities,
        },
    },
    effects::crumble_terrain::{Fragile, handle_right_click_excavation},
    nav::{DungeonNavMesh, NavMeshIslandMarker, playable_area_to_nav_mesh},
    populate_level,
};

#[derive(Resource, Default)]
struct SavedPlayer(Option<(Stats, Inventory)>);

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window { title: "Vector Arena".into(), ..default() }),
                ..default()
            }),
            avian2d::PhysicsPlugins::default(),
            Landmass2dPlugin::default(),
            // bevy_landmass::debug::Landmass2dDebugPlugin::default(),
            rope::RopePlugin,
            UiPlugin,
        ))
        .init_state::<GameState>()
        .init_resource::<SavedPlayer>()
        .add_systems(Startup, setup)
        .add_systems(Startup, enable_ui_input_absorption)
        .add_systems(Startup, init_trail_meshes)
        .add_systems(OnEnter(GameState::Restart), on_enter_restart)
        .add_systems(OnEnter(GameState::Descend), on_enter_descend)
        .add_systems(OnExit(GameState::InLevel), save_player_on_exit)
        .add_systems(Update, set_target_on_click.run_if(not(egui_wants_any_pointer_input)))
        .add_systems(Update, move_player)
        .add_systems(Update, nav::apply_agent_velocity)
        .add_systems(Update, nav::sync_island_nav_mesh)
        .add_systems(Update, fov::update_fov)
        .add_systems(Update, handle_right_click_excavation.run_if(not(egui_wants_any_pointer_input)))
        .add_systems(Update, sync_dungeon_to_entities)
        .add_systems(Update, player_fire_missile)
        .add_systems(Update, monster_fire_missiles)
        .add_systems(Update, update_missiles)
        .add_systems(Update, spawn_missile_trails)
        .add_systems(Update, update_missile_trails)
        .add_systems(Update, update_hit_flash.before(apply_missile_knockback))
        .add_systems(Update, apply_missile_knockback)
        .add_systems(Update, tick_knockback_cooldowns)
        .add_systems(Update, pickup_items)
        .add_systems(Update, animate_pickup)
        .add_systems(Update, manage_time_scale.after(move_player))
        .insert_resource(Gravity::ZERO)
        .insert_resource(SubstepCount(40)) // To make rope physics behave well.
        .init_resource::<DungeonDepth>()
        .run();
}

fn setup(mut commands: Commands) {
    commands.insert_resource(ClearColor(Color::srgb(0.9, 0.9, 0.9)));
    commands.spawn(Camera2d);
}

fn on_enter_restart(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut nav_meshes: ResMut<Assets<NavMesh2d>>,
    window: Single<&Window>,
    mut depth: ResMut<DungeonDepth>,
    mut message_log: ResMut<MessageLog>,
    mut next_state: ResMut<NextState<GameState>>,
) {
    message_log.messages.clear();
    depth.0 = 1;
    spawn_game_world(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut nav_meshes,
        window.width(),
        window.height(),
        1,
        None,
    );
    next_state.set(GameState::InLevel);
}

fn on_enter_descend(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut nav_meshes: ResMut<Assets<NavMesh2d>>,
    window: Single<&Window>,
    mut depth: ResMut<DungeonDepth>,
    mut message_log: ResMut<MessageLog>,
    saved_player: Res<SavedPlayer>,
    mut next_state: ResMut<NextState<GameState>>,
) {
    depth.0 += 1;
    message_log.push(format!("You descend to depth {}.", depth.0));
    spawn_game_world(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut nav_meshes,
        window.width(),
        window.height(),
        depth.0,
        saved_player.0.as_ref().map(|(stats, inv)| (*stats, Inventory(inv.0.clone()))),
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

fn spawn_game_world(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    nav_meshes: &mut Assets<NavMesh2d>,
    window_width: f32,
    window_height: f32,
    depth: u32,
    saved_player: Option<(Stats, Inventory)>,
) {
    // Create the archipelago (the "world" for landmass pathfinding)
    let archipelago_id = commands
        .spawn((
            DespawnOnExit(GameState::InLevel),
            Archipelago2d::new(ArchipelagoOptions::from_agent_radius(AGENT_RADIUS)),
        ))
        .id();

    let terrain_geometry = TerrainGeometry::new(window_width, window_height);

    // Build the underlying canonical state, visuals, collider, and navmesh
    let terrain_mesh = geometry_to_mesh(&terrain_geometry.solid_rock);
    let terrain_collider = geometry_to_collider(&terrain_geometry.solid_rock);
    let valid_nav_mesh = playable_area_to_nav_mesh(&terrain_geometry.playable_area);

    let terrain_mesh_handle = meshes.add(terrain_mesh);
    let nav_mesh_handle = nav_meshes.add(NavMesh2d { nav_mesh: valid_nav_mesh });

    let dungeon_state = DungeonState {
        solid_rock: terrain_geometry.solid_rock.clone(),
        playable_area: terrain_geometry.playable_area.clone(),
    };
    let dungeon_visuals = DungeonVisuals(terrain_mesh_handle.clone());
    let dungeon_collider = DungeonCollider(terrain_collider.clone());
    let dungeon_nav_mesh = DungeonNavMesh(nav_mesh_handle.clone());

    commands.insert_resource(dungeon_state);
    commands.insert_resource(dungeon_visuals);
    commands.insert_resource(dungeon_collider);
    commands.insert_resource(dungeon_nav_mesh);

    // Spawn terrain entity with mesh and collider from the resources
    let terrain_entity = commands
        .spawn((
            DespawnOnExit(GameState::InLevel),
            TerrainMarker,
            Mesh2d(terrain_mesh_handle),
            MeshMaterial2d(materials.add(ColorMaterial::from(Color::srgb(0.2, 0.2, 0.2)))),
            Transform::from_translation(Vec3::new(0.0, 0.0, crate::fov::TERRAIN_Z)),
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

    let door_material = materials.add(ColorMaterial::from(Color::srgb(0.5, 0.25, 0.1)));
    for door in &terrain_geometry.doors {
        let center = door.center();
        let disp = door.disp_size();

        let door_entity = commands
            .spawn((
                DespawnOnExit(GameState::InLevel),
                Fragile,
                Opaque,
                OpaqueVertices(door.disp_corners()),
                Mesh2d(meshes.add(Rectangle::new(disp.x, disp.y))),
                MeshMaterial2d(door_material.clone()),
                Transform::from_translation(center.extend(crate::fov::MOVABLE_Z)),
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
                    .with_local_anchor2(hinge), // Terrain is at origin
            ))
            .id();
        commands.entity(door_entity).add_child(joint_entity);
    }

    commands.insert_resource(WorldBounds { width: window_width, height: window_height });
    let rubble_material = materials.add(ColorMaterial::from(Color::srgb(0.5, 0.45, 0.42)));
    commands
        .insert_resource(vector_arena::effects::crumble_terrain::RubbleMaterial(rubble_material));

    fov::spawn_fov_meshes(commands, meshes, materials, window_width, window_height);

    // Spawn the island (navigation surface) for landmass
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
        archipelago_id,
        depth,
        saved_player,
    );
}
