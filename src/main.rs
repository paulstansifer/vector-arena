use avian2d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::NavMeshHandle;
use bevy_landmass::prelude::*;
use geo::{BooleanOps, MultiPolygon};
use rand::prelude::*;

use vector_arena::AGENT_RADIUS;
use vector_arena::{fov, monster, nav, player, terrain};
use vector_arena::WorldBounds;

use monster::Monster;
use player::{MoveTarget, PLAYER_SPEED, Player, move_player, set_target_on_click};
use terrain::{
    TerrainGeometry, geometry_to_collider, geometry_to_mesh, handle_right_click_excavation,
    playable_area_to_nav_mesh, DungeonState, DungeonVisuals, DungeonCollider, DungeonNavMesh,
    TerrainMarker, NavMeshIslandMarker, sync_dungeon_to_entities,
};

#[derive(Component)]
struct FovMeshMarker;

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
            // bevy_landmass::debug::Landmass2dDebugPlugin::default(),
        ))
        .add_systems(Startup, setup)
        .add_systems(Update, set_target_on_click)
        .add_systems(Update, move_player)
        .add_systems(Update, nav::apply_agent_velocity)
        .add_systems(Update, update_fov)
        .add_systems(Update, handle_right_click_excavation)
        .add_systems(Update, sync_dungeon_to_entities)
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
        .spawn(Archipelago2d::new(ArchipelagoOptions::from_agent_radius(
            AGENT_RADIUS,
        )))
        .id();

    let terrain_geometry = TerrainGeometry::new(window_width, window_height);

    // Build the underlying canonical state, visuals, collider, and navmesh
    let terrain_mesh = geometry_to_mesh(&terrain_geometry.solid_rock);
    let terrain_collider = geometry_to_collider(&terrain_geometry.solid_rock);
    let valid_nav_mesh = playable_area_to_nav_mesh(&terrain_geometry.playable_area);

    let terrain_mesh_handle = meshes.add(terrain_mesh);
    let nav_mesh_handle = nav_meshes.add(NavMesh2d {
        nav_mesh: valid_nav_mesh,
    });

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
            MeshMaterial2d(materials.add(ColorMaterial::from(Color::srgb(0.4, 0.4, 0.4)))),
            Transform::default(),
            terrain_collider,
            RigidBody::Static,
        ))
        .id();

    let door_material = materials.add(ColorMaterial::from(Color::srgb(0.5, 0.25, 0.1)));
    for door in &terrain_geometry.doors {
        let phys_width = door.phys_rect.width();
        let phys_height = door.phys_rect.height();
        let center = door.phys_rect.center();

        let disp_width = door.disp_rect.width();
        let disp_height = door.disp_rect.height();

        let door_entity = commands
            .spawn((
                Mesh2d(meshes.add(Rectangle::new(disp_width, disp_height))),
                MeshMaterial2d(door_material.clone()),
                Transform::from_translation(Vec3::new(center.x, center.y, 1.0)),
                RigidBody::Dynamic,
                Collider::rectangle(phys_width, phys_height),
            ))
            .id();

        let hinge_world = Vec2::new(door.hinge.0, door.hinge.1);
        let door_center = Vec2::new(center.x, center.y);

        commands.spawn(
            RevoluteJoint::new(door_entity, terrain_entity)
                .with_local_anchor1(hinge_world - door_center)
                .with_local_anchor2(hinge_world), // Terrain is at origin
        );
    }

    commands.insert_resource(WorldBounds {
        width: window_width,
        height: window_height,
    });
    let rubble_material = materials.add(ColorMaterial::from(Color::srgb(0.5, 0.45, 0.42)));
    commands.insert_resource(terrain::RubbleMaterial(rubble_material));

    let fov_material = materials.add(ColorMaterial::from(Color::srgba(0.0, 0.0, 0.0, 0.7)));
    commands.spawn((
        FovMeshMarker,
        Mesh2d(meshes.add(Mesh::new(
            bevy_mesh::PrimitiveTopology::TriangleList,
            Default::default(),
        ))),
        MeshMaterial2d(fov_material),
        Transform::from_translation(Vec3::new(0.0, 0.0, 50.0)),
    ));

    // Spawn the island (navigation surface) for landmass
    commands.spawn((
        NavMeshIslandMarker,
        Island2dBundle {
            island: Island,
            archipelago_ref: ArchipelagoRef2d::new(archipelago_id),
            nav_mesh: NavMeshHandle(nav_mesh_handle),
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
            Mesh2d(meshes.add(Circle::new(AGENT_RADIUS))),
            MeshMaterial2d(materials.add(ColorMaterial::from(Color::srgb(0.15, 0.65, 0.95)))),
            Transform::from_translation(player_position.extend(0.0)),
            RigidBody::Dynamic,
            Collider::circle(AGENT_RADIUS),
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
            Mesh2d(monster_mesh.clone()),
            MeshMaterial2d(monster_material.clone()),
            Transform::from_translation(position.extend(0.0)),
            RigidBody::Dynamic,
            Collider::circle(AGENT_RADIUS),
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

fn update_fov(
    player_query: Query<&Transform, (With<Player>, Changed<Transform>)>,
    fov_mesh_query: Query<&Mesh2d, With<FovMeshMarker>>,
    mut meshes: ResMut<Assets<Mesh>>,
    dungeon_state: Res<DungeonState>,
    bounds: Res<WorldBounds>,
) {
    let Ok(player_transform) = player_query.single() else {
        return;
    };
    let Ok(mesh_handle) = fov_mesh_query.single() else {
        return;
    };

    let origin = player_transform.translation.truncate();
    let radius = 600.0;

    let fov_poly = fov::fov_arc(origin, radius, None, &dungeon_state.solid_rock);
    let fov_multi = MultiPolygon::new(vec![fov_poly]);

    let w = bounds.width;
    let h = bounds.height;
    // To be safe, make it larger than bounds
    let bg_rect = geo::Rect::new(
        (-w / 2.0 - 200.0, -h / 2.0 - 200.0),
        (w / 2.0 + 200.0, h / 2.0 + 200.0),
    );
    let bg_poly = MultiPolygon::new(vec![bg_rect.to_polygon()]);

    let dark_area = bg_poly.difference(&fov_multi);

    if let Some(mesh) = meshes.get_mut(&mesh_handle.0) {
        *mesh = terrain::geometry_to_mesh(&dark_area);
    }
}
