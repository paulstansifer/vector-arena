use avian2d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::{NavMeshHandle, prelude::*};
use rand::prelude::*;

use vector_arena::{AGENT_RADIUS, WorldBounds, fov, monster, nav, player, projectile, terrain};

use fov::{Opaque, OpaqueVertices};
use monster::Monster;
use player::{MoveTarget, PLAYER_SPEED, Player, move_player, set_target_on_click};
use projectile::{
    MonsterShootTimer, apply_missile_knockback, init_trail_meshes, manage_time_scale,
    monster_fire_missiles, player_fire_missile, spawn_missile_trails, update_missile_trails,
    update_missiles,
};
use vector_arena::GameLayer;
use terrain::{
    DungeonCollider, DungeonNavMesh, DungeonState, DungeonVisuals, Fragile, NavMeshIslandMarker,
    TerrainGeometry, TerrainMarker, geometry_to_collider, geometry_to_mesh,
    handle_right_click_excavation, playable_area_to_nav_mesh, sync_dungeon_to_entities,
};

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
        ))
        .add_systems(Startup, setup)
        .add_systems(Startup, init_trail_meshes)
        .add_systems(Update, set_target_on_click)
        .add_systems(Update, move_player)
        .add_systems(Update, nav::apply_agent_velocity)
        .add_systems(Update, fov::update_fov)
        .add_systems(Update, handle_right_click_excavation)
        .add_systems(Update, sync_dungeon_to_entities)
        .add_systems(Update, player_fire_missile)
        .add_systems(Update, monster_fire_missiles)
        .add_systems(Update, update_missiles)
        .add_systems(Update, spawn_missile_trails)
        .add_systems(Update, update_missile_trails)
        .add_systems(Update, apply_missile_knockback)
        .add_systems(Update, manage_time_scale.after(move_player))
        .insert_resource(Gravity::ZERO)
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut nav_meshes: ResMut<Assets<NavMesh2d>>,
    window: Single<&Window>,
) {
    commands.insert_resource(ClearColor(Color::srgb(0.9, 0.9, 0.9)));
    commands.spawn(Camera2d);

    // Get window dimensions
    let window_width = window.width();
    let window_height = window.height();

    // Create the archipelago (the "world" for landmass pathfinding)
    let archipelago_id = commands
        .spawn(Archipelago2d::new(ArchipelagoOptions::from_agent_radius(AGENT_RADIUS)))
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
            TerrainMarker,
            Mesh2d(terrain_mesh_handle),
            MeshMaterial2d(materials.add(ColorMaterial::from(Color::srgb(0.2, 0.2, 0.2)))),
            Transform::from_translation(Vec3::new(0.0, 0.0, crate::fov::TERRAIN_Z)),
            terrain_collider,
            RigidBody::Static,
            CollisionLayers::new(GameLayer::Wall, [GameLayer::Wall, GameLayer::Dynamic, GameLayer::Missile]),
        ))
        .id();

    let door_material = materials.add(ColorMaterial::from(Color::srgb(0.5, 0.25, 0.1)));
    for door in &terrain_geometry.doors {
        let center = door.center();
        let disp = door.disp_size();

        let door_entity = commands
            .spawn((
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
            .spawn(
                RevoluteJoint::new(door_entity, terrain_entity)
                    .with_local_anchor1(hinge - center)
                    .with_local_anchor2(hinge), // Terrain is at origin
            )
            .id();
        commands.entity(door_entity).add_child(joint_entity);
    }

    commands.insert_resource(WorldBounds { width: window_width, height: window_height });
    let rubble_material = materials.add(ColorMaterial::from(Color::srgb(0.5, 0.45, 0.42)));
    commands.insert_resource(terrain::RubbleMaterial(rubble_material));

    fov::spawn_fov_meshes(&mut commands, &mut meshes, &mut materials, window_width, window_height);

    // Spawn the island (navigation surface) for landmass
    commands.spawn((NavMeshIslandMarker, Island2dBundle {
        island: Island,
        archipelago_ref: ArchipelagoRef2d::new(archipelago_id),
        nav_mesh: NavMeshHandle(nav_mesh_handle),
    }));

    let mut rng = rand::thread_rng();

    // Choose a random room for the player
    let player_position = if let Some(room) = terrain_geometry.rooms.choose(&mut rng) {
        let center = room.center();
        Vec2::new(center.x, center.y)
    } else {
        Vec2::ZERO // fallback
    };

    let player = commands
        .spawn((
            Player,
            Mesh2d(meshes.add(Circle::new(AGENT_RADIUS))),
            MeshMaterial2d(materials.add(ColorMaterial::from(Color::srgb(0.15, 0.65, 0.95)))),
            Transform::from_translation(player_position.extend(fov::MOVABLE_Z)),
            RigidBody::Dynamic,
            Collider::circle(AGENT_RADIUS),
            CollisionLayers::new(GameLayer::Dynamic, [GameLayer::Wall, GameLayer::Dynamic]),
            LockedAxes::ROTATION_LOCKED,
            MoveTarget::default(),
            Agent2dBundle {
                agent: Default::default(),
                settings: AgentSettings {
                    radius: AGENT_RADIUS,
                    desired_speed: PLAYER_SPEED,
                    max_speed: PLAYER_SPEED * 1.2,
                },
                archipelago_ref: ArchipelagoRef2d::new(archipelago_id),
            },
            AgentTarget2d::None,
        ))
        .id();
    //        .insert(Ccd::enabled());

    let monster_material = materials.add(ColorMaterial::from(Color::srgb(0.85, 0.12, 0.12)));
    let monster_mesh = meshes.add(Circle::new(AGENT_RADIUS));

    // Spawn monsters in other rooms
    let mut monster_positions = Vec::new();
    for room in &terrain_geometry.rooms {
        let center = room.center();
        let pos = Vec2::new(center.x, center.y);
        if pos != player_position {
            monster_positions.push(pos);
        }
    }

    for position in monster_positions.into_iter().take(2) {
        commands.spawn((
            Monster,
            MonsterShootTimer::new(),
            Mesh2d(monster_mesh.clone()),
            MeshMaterial2d(monster_material.clone()),
            Transform::from_translation(position.extend(crate::fov::MOVABLE_Z)),
            RigidBody::Dynamic,
            Collider::circle(AGENT_RADIUS),
            CollisionLayers::new(GameLayer::Dynamic, [GameLayer::Wall, GameLayer::Dynamic]),
            LockedAxes::ROTATION_LOCKED,
            Agent2dBundle {
                agent: Default::default(),
                settings: AgentSettings {
                    radius: AGENT_RADIUS,
                    desired_speed: monster::MONSTER_SPEED,
                    max_speed: monster::MONSTER_SPEED * 1.2,
                },
                archipelago_ref: ArchipelagoRef2d::new(archipelago_id),
            },
            AgentTarget2d::Entity(player),
        ));
    }
}
