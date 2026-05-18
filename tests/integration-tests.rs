use bevy::prelude::*;
use bevy_landmass::prelude::*;
use bevy_landmass::NavMeshHandle;
use rand::prelude::*;
use rand::rngs::StdRng;
use std::time::Duration;
use vector_arena::player::{Player, MoveTarget, move_player, PLAYER_SPEED};
use vector_arena::terrain::{TerrainGeometry, playable_area_to_nav_mesh, PartitionRole};
use vector_arena::bsp::Partition;
use vector_arena::AGENT_RADIUS;
use avian2d::prelude::LinearVelocity;

fn mock_physics_system(
    mut query: Query<(&mut Transform, &LinearVelocity)>,
    time: Res<Time<Virtual>>,
) {
    for (mut transform, velocity) in query.iter_mut() {
        transform.translation += velocity.0.extend(0.0) * time.delta_secs();
    }
}

#[test]
fn test_player_can_path_within_room_and_to_other_room() {
    let mut rng = StdRng::seed_from_u64(1234);

    let bottom_left = Partition {
        x: (10.0, 600.0),
        y: (10.0, 400.0),
        horz_conn: (vec![], vec![]),
        vert_conn: (vec![], vec![300.0]),
    };

    let bottom_right = Partition {
        x: (600.0, 1190.0),
        y: (10.0, 400.0),
        horz_conn: (vec![], vec![]),
        vert_conn: (vec![], vec![900.0]),
    };

    let top_left = Partition {
        x: (10.0, 600.0),
        y: (400.0, 790.0),
        horz_conn: (vec![], vec![600.0]),
        vert_conn: (vec![300.0], vec![]),
    };

    let top_right = Partition {
        x: (600.0, 1190.0),
        y: (400.0, 790.0),
        horz_conn: (vec![600.0], vec![]),
        vert_conn: (vec![900.0], vec![]),
    };

    let allocated_partitions = vec![
        (bottom_left, PartitionRole::Room),
        (bottom_right, PartitionRole::Room),
        (top_left, PartitionRole::Corridor { double_width: false }),
        (top_right, PartitionRole::Corridor { double_width: false }),
    ];

    let terrain_geometry = TerrainGeometry::from_partitions_and_roles(
        1200.0,
        800.0,
        allocated_partitions,
        &mut rng,
    );

    assert_eq!(terrain_geometry.rooms.len(), 2);

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(TransformPlugin);
    app.add_plugins(AssetPlugin::default());
    app.add_plugins(Landmass2dPlugin::default());

    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(1.0 / 60.0)));
    app.insert_resource(Time::<Virtual>::default());
    app.add_systems(Update, (move_player, mock_physics_system).chain());

    let mut nav_meshes = app.world_mut().resource_mut::<Assets<NavMesh2d>>();
    let valid_nav_mesh = playable_area_to_nav_mesh(&terrain_geometry.playable_area);
    let nav_mesh_handle = nav_meshes.add(NavMesh2d {
        nav_mesh: valid_nav_mesh,
    });

    let archipelago_id = app.world_mut()
        .spawn(Archipelago2d::new(ArchipelagoOptions::from_agent_radius(
            AGENT_RADIUS,
        )))
        .id();

    app.world_mut().spawn(Island2dBundle {
        island: Island,
        archipelago_ref: ArchipelagoRef2d::new(archipelago_id),
        nav_mesh: NavMeshHandle(nav_mesh_handle),
    });

    // Start player in the center of the first room
    let r0 = terrain_geometry.rooms[0];
    let start_pos = Vec2::new(r0.center().x, r0.center().y);

    let player_id = app.world_mut()
        .spawn((
            Player,
            Transform::from_translation(start_pos.extend(0.0)),
            LinearVelocity::ZERO,
            MoveTarget {
                destination: start_pos,
                origin: start_pos,
                active: false,
            },
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

    // Update Bevy systems to let Landmass initialize
    app.update();

    // TEST 1: Path to a different point in the same room
    let intra_destination = start_pos + Vec2::new(30.0, 30.0);
    {
        let mut player_entity = app.world_mut().entity_mut(player_id);
        let mut move_target = player_entity.get_mut::<MoveTarget>().unwrap();
        move_target.destination = intra_destination;
        move_target.origin = start_pos;
        move_target.active = true;

        let mut agent_target = player_entity.get_mut::<AgentTarget2d>().unwrap();
        *agent_target = AgentTarget2d::Point(intra_destination);
    }

    // Run app updates until player reaches intra-room target or we hit a step limit
    let mut reached = false;
    for step in 0..1000 {
        let mut time = app.world_mut().resource_mut::<Time<Virtual>>();
        time.advance_by(Duration::from_secs_f32(1.0 / 60.0));
        app.update();

        let player_entity = app.world().entity(player_id);
        let transform = player_entity.get::<Transform>().unwrap();
        let current_pos = transform.translation.truncate();
        let vel = player_entity.get::<LinearVelocity>().unwrap();
        let target = player_entity.get::<AgentTarget2d>().unwrap();
        let desired = player_entity.get::<AgentDesiredVelocity2d>();
        if step < 20 || step % 100 == 0 {
            println!("step {}: pos={:?}, vel={:?}, target={:?}, desired_velocity={:?}", 
                step, current_pos, vel.0, target, desired.map(|d| d.velocity()));
        }
        if current_pos.distance(intra_destination) <= AGENT_RADIUS + 2.0 {
            reached = true;
            break;
        }
    }
    assert!(reached, "Player failed to reach the target within the same room!");

    // TEST 2: Path into the other room
    let r1 = terrain_geometry.rooms[1];
    let inter_destination = Vec2::new(r1.center().x, r1.center().y);

    let start_pos_test2 = {
        let player_entity = app.world().entity(player_id);
        player_entity.get::<Transform>().unwrap().translation.truncate()
    };

    {
        let mut player_entity = app.world_mut().entity_mut(player_id);
        let mut move_target = player_entity.get_mut::<MoveTarget>().unwrap();
        move_target.destination = inter_destination;
        move_target.origin = start_pos_test2;
        move_target.active = true;

        let mut agent_target = player_entity.get_mut::<AgentTarget2d>().unwrap();
        *agent_target = AgentTarget2d::Point(inter_destination);
    }

    reached = false;
    for step in 0..3000 {
        let mut time = app.world_mut().resource_mut::<Time<Virtual>>();
        time.advance_by(Duration::from_secs_f32(1.0 / 60.0));
        app.update();

        let player_entity = app.world().entity(player_id);
        let transform = player_entity.get::<Transform>().unwrap();
        let current_pos = transform.translation.truncate();
        let vel = player_entity.get::<LinearVelocity>().unwrap();
        let target = player_entity.get::<AgentTarget2d>().unwrap();
        let desired = player_entity.get::<AgentDesiredVelocity2d>();
        if step < 20 || step % 200 == 0 {
            println!("TEST2 step {}: pos={:?}, vel={:?}, target={:?}, desired_velocity={:?}", 
                step, current_pos, vel.0, target, desired.map(|d| d.velocity()));
        }
        if current_pos.distance(inter_destination) <= AGENT_RADIUS + 2.0 {
            reached = true;
            break;
        }
    }
    assert!(reached, "Player failed to path find and reach the other room!");
}
