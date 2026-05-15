use bevy::prelude::*;
use bevy_rapier2d::prelude::*;
use geo::MultiPolygon;
use rand::prelude::*;

mod bsp;
mod monster;
mod player;
mod terrain;

use monster::{MONSTER_RADIUS, Monster, move_monsters};
use player::{MoveTarget, PLAYER_RADIUS, Player, move_player, set_target_on_click};
use terrain::{Terrain, TerrainGeometry, geometry_to_collider, geometry_to_mesh};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Vector Arena".into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(RapierPhysicsPlugin::<NoUserData>::default())
        .add_systems(Startup, setup)
        .add_systems(Update, set_target_on_click)
        .add_systems(Update, move_player)
        .add_systems(Update, move_monsters)
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    window: Single<&Window>,
) {
    commands.spawn(Camera2d);

    // Get window dimensions
    let window_width = window.width();
    let window_height = window.height();

    let terrain_geometry = if false {
        // Create terrain geometry
        TerrainGeometry::new(window_width, window_height)
    } else {
        TerrainGeometry {
            polygon: MultiPolygon::empty(),
            rooms: vec![geo::geometry::Rect::new(
                (0.0, 0.0),
                (window_width, window_width),
            )],
        }
    };

    // Spawn terrain entity with mesh and collider
    let terrain_mesh = geometry_to_mesh(&terrain_geometry.polygon);
    let terrain_collider = geometry_to_collider(&terrain_geometry.polygon);

    commands.spawn((
        Mesh2d(meshes.add(terrain_mesh)),
        MeshMaterial2d(materials.add(ColorMaterial::from(Color::srgb(0.2, 0.7, 0.3)))),
        Transform::default(),
        terrain_collider,
        RigidBody::Fixed,
        Terrain,
    ));

    let mut rng = rand::thread_rng();

    // Choose a random room for the player
    let player_position = if let Some(room) = terrain_geometry.rooms.choose(&mut rng) {
        let center = room.center();
        Vec2::new(center.x, center.y)
    } else {
        Vec2::ZERO // fallback
    };

    commands.spawn((
        Mesh2d(meshes.add(Circle::new(PLAYER_RADIUS))),
        MeshMaterial2d(materials.add(ColorMaterial::from(Color::srgb(0.15, 0.65, 0.95)))),
        Transform::from_translation(player_position.extend(0.0)),
        RigidBody::Dynamic,
        Collider::ball(PLAYER_RADIUS),
        LockedAxes::ROTATION_LOCKED,
        Velocity::zero(),
        Player,
        MoveTarget::default(),
    ));

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

    // Take up to 2 monsters
    for position in monster_positions.into_iter().take(2) {
        commands.spawn((
            Mesh2d(monster_mesh.clone()),
            MeshMaterial2d(monster_material.clone()),
            Transform::from_translation(position.extend(0.0)),
            RigidBody::Dynamic,
            Collider::ball(MONSTER_RADIUS),
            Velocity::zero(),
            Monster,
        ));
    }
}
