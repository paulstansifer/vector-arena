mod test_lib;
use test_lib::{loc, physics_app, tick};

use avian2d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::{NavMeshHandle, prelude::*};
use rand::{prelude::*, rngs::StdRng};
use std::time::Duration;
use vector_arena::{
    AGENT_RADIUS, GameLayer,
    dungeon::{
        bsp::Partition,
        level_generation::{PartitionRole, RoomVariant, TerrainGeometry},
        terrain::geometry_to_collider,
    },
    monster::MONSTER_SPEED,
    nav::playable_area_to_nav_mesh,
    player::{MoveTarget, PLAYER_SPEED, Player, move_player},
};

/// Verifies that a magic missile aimed at a stationary monster registers a hit when fired from
/// various starting distances.
///
/// `apply_missile_knockback` polls missile positions once per frame via `shape_intersections`.
/// The missile travels ~58 units per 60 Hz frame, but the combined hit radius (missile 4 +
/// monster 10 = 14 units) is far narrower.
#[test]
fn missile_hits_monster_at_various_distances() {
    use vector_arena::{
        command_palette::LetterMap,
        effects::projectile::{MagicMissile, MISSILE_SPEED, apply_missile_knockback},
        monster::{Monster, Stats},
        ui::MessageLog,
    };

    const MONSTER_HP: f32 = 20.0;
    // Monster radius 10 + missile radius 4 = 14-unit hit window.
    // Missile speed / 60 Hz ≈ 58 units per frame, so most starting positions land outside.
    let distances: &[f32] = &[5.0, 30.0, 50.0, 100.0, 200.0, 500.0];

    let mut failures: Vec<f32> = Vec::new();

    for &distance in distances {
        let mut app = physics_app(Vec2::ZERO, false);
        app.init_asset::<ColorMaterial>();
        app.init_resource::<MessageLog>();
        app.init_resource::<LetterMap>();
        app.add_systems(Update, apply_missile_knockback);

        // Monster at origin.
        let monster = app
            .world_mut()
            .spawn((
                Monster,
                Stats { hp: MONSTER_HP, max_hp: MONSTER_HP, ..default() },
                RigidBody::Dynamic,
                Collider::circle(AGENT_RADIUS),
                CollisionLayers::new(GameLayer::Dynamic, [GameLayer::Wall, GameLayer::Dynamic]),
                LockedAxes::ROTATION_LOCKED,
                Mass(1.0),
                LinearVelocity::ZERO,
                Transform::from_xyz(0.0, 0.0, 0.0),
            ))
            .id();

        // One tick so the monster's collider is registered in the spatial query index.
        tick(&mut app);

        // Missile at (-distance, 0) aimed right (+X) toward the monster.
        let missile_vel = Vec2::new(MISSILE_SPEED, 0.0);
        app.world_mut().spawn((
            MagicMissile::new(true, Vec2::new(-distance, 0.0), missile_vel),
            RigidBody::Dynamic,
            Collider::circle(4.0),
            CollisionLayers::new(GameLayer::Missile, GameLayer::Wall),
            LockedAxes::ROTATION_LOCKED,
            LinearVelocity(missile_vel),
            Mass(0.1),
            Transform::from_xyz(-distance, 0.0, 0.0),
        ));

        // Run enough frames for the missile to travel well past the target.
        let frames = (distance / (MISSILE_SPEED / 60.0)).ceil() as u32 + 20;
        for _ in 0..frames {
            tick(&mut app);
        }

        let hit = match app.world().get::<Stats>(monster) {
            Some(stats) => stats.hp < MONSTER_HP,
            None => true, // monster was destroyed — definitely a hit
        };
        let hp_remaining = app.world().get::<Stats>(monster).map_or(0.0, |s| s.hp);
        println!(
            "distance {distance:5.0}: {}  (hp {hp_remaining}/{MONSTER_HP})",
            if hit { "HIT " } else { "MISS" },
        );
        if !hit {
            failures.push(distance);
        }
    }

    assert!(
        failures.is_empty(),
        "missile missed the monster at distances {failures:?}\n\
         (missile travels ~{:.0} units/frame; hit window is ~14 units wide)",
        MISSILE_SPEED / 60.0,
    );
}

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
        (bottom_left, PartitionRole::Room { variant: RoomVariant::Normal }),
        (bottom_right, PartitionRole::Room { variant: RoomVariant::Normal }),
        (top_left, PartitionRole::Corridor { double_width: false }),
        (top_right, PartitionRole::Corridor { double_width: false }),
    ];

    let terrain_geometry =
        TerrainGeometry::from_partitions_and_roles(1200.0, 800.0, allocated_partitions, &mut rng);

    assert_eq!(terrain_geometry.rooms.len(), 2);

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(TransformPlugin);
    app.add_plugins(AssetPlugin::default());
    app.add_plugins(Landmass2dPlugin::default());

    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
        1.0 / 60.0,
    )));
    app.insert_resource(Time::<Virtual>::default());
    app.add_systems(Update, (move_player, mock_physics_system).chain());

    let mut nav_meshes = app.world_mut().resource_mut::<Assets<NavMesh2d>>();
    let valid_nav_mesh = playable_area_to_nav_mesh(&terrain_geometry.playable_area);
    let nav_mesh_handle = nav_meshes.add(NavMesh2d { nav_mesh: valid_nav_mesh });

    let archipelago_id = app
        .world_mut()
        .spawn(Archipelago2d::new(ArchipelagoOptions::from_agent_radius(AGENT_RADIUS)))
        .id();

    app.world_mut().spawn(Island2dBundle {
        island: Island,
        archipelago_ref: ArchipelagoRef2d::new(archipelago_id),
        nav_mesh: NavMeshHandle(nav_mesh_handle),
    });

    // Start player in the center of the first room
    let r0 = terrain_geometry.rooms[0];
    let start_pos = Vec2::new(r0.center().x, r0.center().y);

    let player_id = app
        .world_mut()
        .spawn((
            Player,
            Transform::from_translation(start_pos.extend(0.0)),
            LinearVelocity::ZERO,
            MoveTarget {
                destination: start_pos,
                origin: start_pos,
                active: false,
                time_set: Default::default(),
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
            println!(
                "step {}: pos={:?}, vel={:?}, target={:?}, desired_velocity={:?}",
                step,
                current_pos,
                vel.0,
                target,
                desired.map(|d| d.velocity())
            );
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
            println!(
                "TEST2 step {}: pos={:?}, vel={:?}, target={:?}, desired_velocity={:?}",
                step,
                current_pos,
                vel.0,
                target,
                desired.map(|d| d.velocity())
            );
        }
        if current_pos.distance(inter_destination) <= AGENT_RADIUS + 2.0 {
            reached = true;
            break;
        }
    }
    assert!(reached, "Player failed to path find and reach the other room!");
}

/// Run a single body-vs-wall scenario and return the body's final Y position.
///
/// `polyline` controls whether the wall is a polyline (terrain-style) or a solid rectangle.
/// `driven` controls whether the body's velocity is forcibly reset each frame toward the wall
/// (as `apply_agent_velocity` does for monsters), or left to fall under gravity.
fn body_final_y(polyline: bool, driven: bool) -> f32 {
    // Rock: 30 wide, 20 tall, centred at (0, -30).  Top surface at y = -20.
    const ROCK_Y: f32 = -30.0;
    const ROCK_W: f32 = 30.0;
    const ROCK_H: f32 = 20.0;
    const ROCK_TOP: f32 = ROCK_Y + ROCK_H / 2.0; // -20.0

    // Body starts 30 units above rock top, well clear of any initial overlap.
    const START_Y: f32 = ROCK_TOP + AGENT_RADIUS as f32 + 30.0; // -20 + 10 + 30 = 20

    let gravity = if driven { Vec2::ZERO } else { Vec2::new(0.0, -200.0) };
    let mut app = physics_app(gravity, /* ropes */ false);

    // Wall collider: either a filled rectangle or a polyline of its boundary.
    let wall_collider = if polyline {
        // Mirror of geometry_to_collider: a closed polyline ring around the rectangle.
        let hw = ROCK_W / 2.0;
        let hh = ROCK_H / 2.0;
        Collider::polyline(
            vec![Vec2::new(-hw, hh), Vec2::new(hw, hh), Vec2::new(hw, -hh), Vec2::new(-hw, -hh)],
            Some(vec![[0, 1], [1, 2], [2, 3], [3, 0]]),
        )
    } else {
        Collider::rectangle(ROCK_W, ROCK_H)
    };

    app.world_mut().spawn((
        RigidBody::Static,
        wall_collider,
        CollisionLayers::new(vector_arena::GameLayer::Wall, [
            vector_arena::GameLayer::Wall,
            vector_arena::GameLayer::Dynamic,
        ]),
        Transform::from_xyz(0.0, ROCK_Y, 0.0),
    ));

    let body = app
        .world_mut()
        .spawn((
            RigidBody::Dynamic,
            Collider::circle(AGENT_RADIUS as f32),
            CollisionLayers::new(vector_arena::GameLayer::Dynamic, [
                vector_arena::GameLayer::Wall,
                vector_arena::GameLayer::Dynamic,
            ]),
            LockedAxes::ROTATION_LOCKED,
            LinearDamping(0.0),
            Mass(1.0),
            Transform::from_xyz(0.0, START_Y, 0.0),
        ))
        .id();

    for _ in 0..120 {
        if driven {
            // Mirror apply_agent_velocity: overwrite velocity toward the wall every frame.
            app.world_mut().entity_mut(body).get_mut::<LinearVelocity>().unwrap().0 =
                Vec2::new(0.0, -MONSTER_SPEED);
        }
        tick(&mut app);
    }

    app.world().entity(body).get::<Transform>().unwrap().translation.y
}

#[test]
fn monster_stopped_by_wall() {
    // Expected: body centre rests at rock_top + radius = -20 + 10 = -10.
    // Tolerance of 2 units for physics settling.
    const EXPECTED_STOP: f32 = -10.0;
    const TOL: f32 = 2.0;

    let y_solid_gravity = body_final_y(false, false);
    let y_poly_gravity = body_final_y(true, false);
    let y_solid_driven = body_final_y(false, true);
    let y_poly_driven = body_final_y(true, true);

    println!(
        "solid+gravity={:.1}  poly+gravity={:.1}  solid+driven={:.1}  poly+driven={:.1}  \
         expected≥{:.1}",
        y_solid_gravity,
        y_poly_gravity,
        y_solid_driven,
        y_poly_driven,
        EXPECTED_STOP - TOL,
    );

    // All four cases should stop the body near the rock surface.
    assert!(
        y_solid_gravity >= EXPECTED_STOP - TOL,
        "solid+gravity  clipped: y={:.1}",
        y_solid_gravity
    );
    assert!(
        y_poly_gravity >= EXPECTED_STOP - TOL,
        "poly+gravity   clipped: y={:.1}",
        y_poly_gravity
    );
    assert!(
        y_solid_driven >= EXPECTED_STOP - TOL,
        "solid+driven   clipped: y={:.1}",
        y_solid_driven
    );
    assert!(y_poly_driven >= EXPECTED_STOP - TOL, "poly+driven    clipped: y={:.1}", y_poly_driven);
}

/// Verify that geometry_to_collider produces working collision from a polygon-with-hole geometry.
/// This is the same structure the dungeon uses: a filled solid-rock polygon with the playable
/// area (room) carved out as an interior hole via geo BooleanOps.
///
/// Layout:
///   Outer boundary: (-100,-100) to (100,100)  — solid rock fills this minus the room
///   Room hole:       (-60,  -60) to ( 60,  60) — body starts at origin inside here
///   Left wall inner face: x = -60
///   Expected stop: x = -60 + AGENT_RADIUS = -50
#[test]
fn body_stops_at_dungeon_terrain_wall() {
    use geo::{BooleanOps, MultiPolygon, Rect};

    // Build solid_rock exactly as level_generation does: full bounds minus playable area.
    let earth: geo::Polygon<f32> =
        Rect::new((-100.0_f32, -100.0_f32), (100.0_f32, 100.0_f32)).to_polygon();
    let room: MultiPolygon<f32> = MultiPolygon::new(vec![
        Rect::new((-60.0_f32, -60.0_f32), (60.0_f32, 60.0_f32)).to_polygon(),
    ]);
    let solid_rock: MultiPolygon<f32> = earth.difference(&room);

    let mut app = physics_app(Vec2::ZERO, /* ropes */ false);

    app.world_mut().spawn((
        RigidBody::Static,
        geometry_to_collider(&solid_rock),
        CollisionLayers::new(GameLayer::Wall, [GameLayer::Wall, GameLayer::Dynamic]),
        Transform::default(),
    ));

    // Body starts at origin (inside the 120×120 room), driven left into the left wall.
    const START_X: f32 = 0.0;
    const START_Y: f32 = 0.0;
    const WALL_FACE_X: f32 = -60.0; // inner face of left solid-rock wall

    let body = app
        .world_mut()
        .spawn((
            RigidBody::Dynamic,
            Collider::circle(AGENT_RADIUS),
            CollisionLayers::new(GameLayer::Dynamic, [GameLayer::Wall, GameLayer::Dynamic]),
            LockedAxes::ROTATION_LOCKED,
            Mass(1.0),
            Transform::from_xyz(START_X, START_Y, 0.0),
        ))
        .id();

    for frame in 0..120u32 {
        app.world_mut().entity_mut(body).get_mut::<LinearVelocity>().unwrap().0 =
            Vec2::new(-MONSTER_SPEED, 0.0);
        tick(&mut app);
        if frame % 20 == 19 {
            let p = loc(&app, body);
            println!("  frame {:3}: x={:.2} y={:.2}", frame + 1, p.x, p.y);
        }
    }

    let final_x = loc(&app, body).x;
    println!(
        "Final x={:.2}  (wall face x={WALL_FACE_X:.1}, stop x≈{:.1})",
        final_x,
        WALL_FACE_X + AGENT_RADIUS,
    );

    assert!(
        final_x >= WALL_FACE_X + AGENT_RADIUS - 2.0,
        "Body clipped through dungeon terrain wall: x={:.2} (expected ≥ {:.2})",
        final_x,
        WALL_FACE_X + AGENT_RADIUS - 2.0,
    );
}
