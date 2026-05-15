use std::ops::Deref;

use avian2d::prelude::*;
use bevy::prelude::*;
use geo::MultiPolygon;
use rand::prelude::*;

mod bsp;
mod monster;
mod nav;
mod player;
mod terrain;

use monster::{MONSTER_RADIUS, Monster /*move_monsters*/};
use player::{MoveTarget, PLAYER_RADIUS, Player, move_player, set_target_on_click};
use terrain::{Terrain, TerrainGeometry, geometry_to_collider, geometry_to_mesh};
use vleue_navigator::prelude::*;

#[derive(Component, Debug)]
struct Obstacle;

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
            vleue_navigator::VleueNavigatorPlugin,
            // Auto update the navmesh.
            // Obstacles will be entities with the `Obstacle` marker component,
            NavmeshUpdaterPlugin::<Collider, Obstacle>::default(),
        ))
        .add_systems(Startup, setup)
        .add_systems(Update, set_target_on_click)
        .add_systems(Update, move_player)
        .add_systems(Update, nav::move_navigator)
        .add_systems(Update, nav::refresh_path)
        .add_systems(Update, display_mesh)
        .insert_resource(Gravity::ZERO)
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

    commands.spawn((
        NavMeshSettings {
            // Define the outer borders of the navmesh.
            fixed: Triangulation::from_outer_edges(&[
                vec2(-window_width, -window_height),
                vec2(window_width, -window_height),
                vec2(window_width, window_height),
                vec2(-window_width, window_height),
            ]),
            simplify: 0.01,
            ..default()
        },
        // Mark it for update as soon as obstacles are changed.
        // Other modes can be debounced or manually triggered.
        NavMeshUpdateMode::Direct,
    ));

    let terrain_geometry = if true {
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
        MeshMaterial2d(materials.add(ColorMaterial::from(Color::srgb(0.4, 0.4, 0.4)))),
        Transform::default(),
        terrain_collider,
        RigidBody::Static,
        Obstacle,
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
            nav::Navigator {
                speed: crate::monster::MONSTER_SPEED,
            },
            nav::Target::Follow(player),
            nav::Path::default(),
        ));
    }
}

fn display_mesh(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    navmeshes: Res<Assets<NavMesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut current_mesh_entity: Local<Option<Entity>>,
    navmesh: Single<(&ManagedNavMesh, Ref<NavMeshStatus>)>,
) {
    let (navmesh_handle, status) = navmesh.deref();
    if !status.is_changed() || **status != NavMeshStatus::Built {
        return;
    }

    let Some(navmesh) = navmeshes.get(*navmesh_handle) else {
        return;
    };
    if let Some(entity) = *current_mesh_entity {
        commands.entity(entity).despawn();
    }

    *current_mesh_entity = Some(
        commands
            .spawn((
                Mesh2d(meshes.add(navmesh.to_mesh())),
                MeshMaterial2d(materials.add(ColorMaterial::from(Color::Srgba(Srgba {
                    red: 0.0,
                    green: 0.0,
                    blue: 0.5,
                    alpha: 0.5,
                })))),
            ))
            .with_children(|main_mesh| {
                main_mesh.spawn((
                    Mesh2d(meshes.add(navmesh.to_wireframe_mesh())),
                    Transform::from_translation(Vec3::new(0.0, 0.0, 0.1)),
                    MeshMaterial2d(materials.add(ColorMaterial::from(Color::Srgba(
                        bevy::color::palettes::tailwind::TEAL_300,
                    )))),
                ));
            })
            .id(),
    );
}
