use avian2d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::prelude::*;
use bevy_landmass::debug::Landmass2dDebugPlugin;
use bevy_landmass::NavMeshHandle;
use geo::MultiPolygon;
use rand::prelude::*;

mod bsp;
mod monster;
mod nav;
mod player;
mod terrain;

use monster::{MONSTER_RADIUS, Monster};
use player::{MoveTarget, PLAYER_RADIUS, Player, move_player, set_target_on_click};
use terrain::{Terrain, TerrainGeometry, geometry_to_collider, geometry_to_mesh, playable_area_to_nav_mesh};

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Vector Arena".into(),
                    ..default()
                }),
                ..default()
            }),
            avian2d::PhysicsPlugins::default(),
            Landmass2dPlugin::default(),
            Landmass2dDebugPlugin::default(),
        ))
        .add_systems(Startup, setup)
        .add_systems(Update, set_target_on_click)
        .add_systems(Update, move_player)
        .add_systems(Update, nav::apply_agent_velocity)
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
    commands.spawn(Camera2d);

    // Get window dimensions
    let window_width = window.width();
    let window_height = window.height();

    // Create the archipelago (the "world" for landmass pathfinding)
    let archipelago_id = commands
        .spawn(Archipelago2d::new(
            ArchipelagoOptions::from_agent_radius(MONSTER_RADIUS),
        ))
        .id();

    let terrain_geometry = if true {
        // Create terrain geometry
        TerrainGeometry::new(window_width, window_height)
    } else {
        let room = geo::geometry::Rect::new(
            (0.0, 0.0),
            (window_width, window_width),
        );
        TerrainGeometry {
            polygon: MultiPolygon::empty(),
            playable_area: MultiPolygon::new(vec![room.to_polygon()]),
            rooms: vec![room],
        }
    };

    // Spawn terrain entity with mesh and collider
    let terrain_mesh = geometry_to_mesh(&terrain_geometry.polygon);
    let terrain_collider = geometry_to_collider(&terrain_geometry.polygon);

    commands.spawn((
        Mesh2d(meshes.add(terrain_mesh)),
        MeshMaterial2d(materials.add(ColorMaterial::from(Color::srgb(0.4, 0.4, 0.4)))),
        Transform::default(),
        terrain_collider,
        RigidBody::Static,
        Terrain,
    ));

    // Build the navigation mesh from the playable area
    let valid_nav_mesh = playable_area_to_nav_mesh(&terrain_geometry.playable_area);
    let nav_mesh_handle = nav_meshes.add(NavMesh2d {
        nav_mesh: valid_nav_mesh,
    });

    // Spawn the island (navigation surface) for landmass
    commands.spawn((
        Island2dBundle {
            island: Island,
            archipelago_ref: ArchipelagoRef2d::new(archipelago_id),
            nav_mesh: NavMeshHandle(nav_mesh_handle.clone()),
        },
    ));

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
            Mesh2d(meshes.add(Circle::new(PLAYER_RADIUS))),
            MeshMaterial2d(materials.add(ColorMaterial::from(Color::srgb(0.15, 0.65, 0.95)))),
            Transform::from_translation(player_position.extend(0.0)),
            RigidBody::Dynamic,
            Collider::circle(PLAYER_RADIUS),
            LockedAxes::ROTATION_LOCKED,
            MoveTarget::default(),
        ))
        .id();
    //        .insert(Ccd::enabled());

    let monster_material = materials.add(ColorMaterial::from(Color::srgb(0.85, 0.12, 0.12)));
    let monster_mesh = meshes.add(Circle::new(MONSTER_RADIUS));

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
            Mesh2d(monster_mesh.clone()),
            MeshMaterial2d(monster_material.clone()),
            Transform::from_translation(position.extend(0.0)),
            RigidBody::Dynamic,
            Collider::circle(MONSTER_RADIUS),
            Agent2dBundle {
                agent: Default::default(),
                settings: AgentSettings {
                    radius: MONSTER_RADIUS,
                    desired_speed: monster::MONSTER_SPEED,
                    max_speed: monster::MONSTER_SPEED * 1.2,
                },
                archipelago_ref: ArchipelagoRef2d::new(archipelago_id),
            },
            AgentTarget2d::Entity(player),
        ));
    }
}
