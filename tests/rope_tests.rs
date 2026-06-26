#[path = "test_lib/physics.rs"]
mod test_lib;
use test_lib::{loc, physics_app, tick};

use avian2d::prelude::*;
use bevy::{ecs::system::SystemState, prelude::*};
use bevy_verlet::prelude::*;
use vector_arena::{
    GameLayer,
    effects::rope::{RopeEndAnchor, RopePoint, SEGMENT_TARGET_LEN, spawn_rope},
};

// Rope is 10 segments × SEGMENT_TARGET_LEN = 100 units long.
const N: usize = 10;
const ROCK_X: f32 = 50.0;
const ROCK_Y: f32 = -15.0;
const ROCK_H: f32 = 10.0;
const ROCK_TOP: f32 = ROCK_Y + ROCK_H / 2.0; // -10.0

fn spawn_wall_pin(app: &mut App) {
    app.world_mut().spawn((
        RigidBody::Static,
        Collider::circle(3.0),
        CollisionLayers::new(GameLayer::Wall, [
            GameLayer::Wall,
            GameLayer::Dynamic,
            GameLayer::Rope,
        ]),
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));
}

fn spawn_rock(app: &mut App) {
    app.world_mut().spawn((
        RigidBody::Static,
        Collider::rectangle(30.0, ROCK_H),
        CollisionLayers::new(GameLayer::Wall, [
            GameLayer::Wall,
            GameLayer::Dynamic,
            GameLayer::Rope,
        ]),
        Transform::from_xyz(ROCK_X, ROCK_Y, 0.0),
    ));
}

/// Spawn the rope and lock/attach its endpoints.
///
/// The left (min-x) endpoint is always pinned with `VerletLocked`.
/// `end_anchor`: if `Some(entity)`, the right endpoint tracks that entity via
/// `RopeEndAnchor`; if `None`, it is also pinned in place.
///
/// Returns `(left_entity, right_entity)`.
fn setup_rope(app: &mut App, end_anchor: Option<Entity>) -> (Entity, Entity) {
    type Params<'w, 's> = (
        Commands<'w, 's>,
        SpatialQuery<'w, 's>,
        Query<'w, 's, (&'static GlobalTransform, &'static RigidBody)>,
    );
    let mut state: SystemState<Params> = SystemState::new(app.world_mut());
    let (mut commands, spatial_query, anchor_query) = state.get_mut(app.world_mut());
    spawn_rope(
        &mut commands,
        Vec2::ZERO,
        Vec2::new(N as f32 * SEGMENT_TARGET_LEN, 0.0),
        &spatial_query,
        &anchor_query,
        None,
    );
    state.apply(app.world_mut());

    let (start_opt, end_opt) = {
        let mut q = app.world_mut().query_filtered::<(Entity, &Transform), With<RopePoint>>();
        q.iter(app.world()).fold(
            (None::<(Entity, f32)>, None::<(Entity, f32)>),
            |(mn, mx), (e, tf)| {
                let x = tf.translation.x;
                (
                    Some(mn.map_or((e, x), |(me, v)| if x < v { (e, x) } else { (me, v) })),
                    Some(mx.map_or((e, x), |(me, v)| if x > v { (e, x) } else { (me, v) })),
                )
            },
        )
    };
    let (start_e, end_e) = (start_opt.unwrap().0, end_opt.unwrap().0);

    app.world_mut().entity_mut(start_e).insert(VerletLocked).remove::<RopeEndAnchor>();

    let mut em = app.world_mut().entity_mut(end_e);
    em.insert(VerletLocked);
    match end_anchor {
        Some(tracked) => {
            em.insert(RopeEndAnchor { tracked, local: Vec3::ZERO });
        }
        None => {
            em.remove::<RopeEndAnchor>();
        }
    }

    (start_e, end_e)
}

fn mid_rope_y(app: &mut App) -> f32 {
    let mut q = app.world_mut().query_filtered::<&Transform, With<RopePoint>>();
    q.iter(app.world())
        .min_by(|a, b| {
            (a.translation.x - ROCK_X).abs().partial_cmp(&(b.translation.x - ROCK_X).abs()).unwrap()
        })
        .map_or(f32::NEG_INFINITY, |t| t.translation.y)
}

// Layout (Y up):
//
//   anchor(0,0) ===== rope ===== ball(100,0)
//                    rock(50,-15)
//
// Gravity pulls rope and ball down; rope should sag onto rock top without
// passing through it.
#[test]
fn rope_rests_on_rock_obstacle() {
    let mut app = physics_app(Vec2::new(0.0, -200.0), /* ropes */ true);
    spawn_wall_pin(&mut app);
    spawn_rock(&mut app);

    let ball = app
        .world_mut()
        .spawn((
            RigidBody::Dynamic,
            Collider::circle(3.0),
            CollisionLayers::new(GameLayer::Dynamic, [GameLayer::Wall, GameLayer::Dynamic]),
            LinearDamping(2.0),
            AngularDamping(2.0),
            Mass(1.0),
            Transform::from_xyz(N as f32 * SEGMENT_TARGET_LEN, 0.0, 0.0),
        ))
        .id();

    tick(&mut app);
    setup_rope(&mut app, None);

    for frame in 0..100u32 {
        tick(&mut app);
        if frame % 10 == 9 {
            let mid_y = mid_rope_y(&mut app);
            let b = loc(&app, ball);
            let vel = app.world().entity(ball).get::<LinearVelocity>().unwrap().0;
            println!(
                "  frame {:4}: ball=({:.1},{:.1}) vel={:.2?}  mid-rope y={:.1}",
                frame + 1,
                b.x,
                b.y,
                vel,
                mid_y,
            );
        }
    }

    let mid_y = mid_rope_y(&mut app);
    let vel = app.world().entity(ball).get::<LinearVelocity>().unwrap().0;
    println!(
        "Final: ball y={:.1} vel={:.2?}  mid-rope y={:.1}  rock top={ROCK_TOP:.1}",
        loc(&app, ball).y,
        vel,
        mid_y,
    );

    assert!(
        mid_y >= ROCK_TOP - 3.0,
        "Rope passed through rock: mid-rope y={mid_y:.1} is below rock top {ROCK_TOP:.1}"
    );
    assert!(loc(&app, ball).y < -40.0);
}

// Layout (Y up): no gravity. Monster at rope end moves downward, pulling the
// rope toward the rock.
//
//   anchor(0,0) ===== rope ===== monster(100,0) → moving down
//                    rock(50,-15)
#[test]
fn rope_rests_on_rock_while_monster_pulls_end_down() {
    const MONSTER_SPEED: f32 = -30.0;

    let mut app = physics_app(Vec2::ZERO, /* ropes */ true);
    spawn_wall_pin(&mut app);
    spawn_rock(&mut app);

    let monster = app
        .world_mut()
        .spawn((
            RigidBody::Dynamic,
            Collider::circle(3.0),
            CollisionLayers::new(GameLayer::Dynamic, [GameLayer::Wall, GameLayer::Dynamic]),
            LinearDamping(0.0),
            AngularDamping(0.0),
            Mass(1.0),
            GravityScale(0.0),
            LinearVelocity(Vec2::new(0.0, MONSTER_SPEED)),
            Transform::from_xyz(N as f32 * SEGMENT_TARGET_LEN, 0.0, 0.0),
        ))
        .id();

    tick(&mut app);
    setup_rope(&mut app, Some(monster));

    for frame in 0..120u32 {
        app.world_mut().entity_mut(monster).insert(LinearVelocity(Vec2::new(0.0, MONSTER_SPEED)));
        tick(&mut app);
        if frame % 10 == 9 {
            let mid_y = mid_rope_y(&mut app);
            let monster_y = loc(&app, monster).y;
            println!(
                "  frame {:4}: monster_y={:.1}  mid-rope y={:.1}  rock top={ROCK_TOP:.1}",
                frame + 1,
                monster_y,
                mid_y,
            );
        }
    }

    let mid_y = mid_rope_y(&mut app);
    let monster_y = loc(&app, monster).y;
    println!("Final: monster_y={monster_y:.1}  mid-rope y={mid_y:.1}  rock top={ROCK_TOP:.1}");

    assert!(
        mid_y >= ROCK_TOP - 3.0,
        "Rope passed through rock: mid-rope y={mid_y:.1} is below rock top {ROCK_TOP:.1}"
    );
}
