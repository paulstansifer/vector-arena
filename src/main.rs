use bevy::prelude::*;
use bevy_rapier2d::prelude::*;

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

    // Create terrain geometry
    let terrain_geometry = TerrainGeometry::new(window_width, window_height);

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

    commands.spawn((
        Mesh2d(meshes.add(Circle::new(PLAYER_RADIUS))),
        MeshMaterial2d(materials.add(ColorMaterial::from(Color::srgb(0.15, 0.65, 0.95)))),
        Transform::default(),
        Player,
        MoveTarget::default(),
    ));

    let monster_material = materials.add(ColorMaterial::from(Color::srgb(0.85, 0.12, 0.12)));
    let monster_mesh = meshes.add(Circle::new(MONSTER_RADIUS));
    let monster_positions = [Vec2::new(-220.0, 100.0), Vec2::new(220.0, -80.0)];

    for position in monster_positions {
        commands.spawn((
            Mesh2d(monster_mesh.clone()),
            MeshMaterial2d(monster_material.clone()),
            Transform::from_translation(position.extend(0.0)),
            Monster,
        ));
    }
}
