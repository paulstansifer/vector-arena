// Drag-to-draw rope with Verlet physics.
// Press O once to mark the start, once more to mark the end.  `spawn_rope` places
// verlet points spaced by SEGMENT_TARGET_LEN and connects them with VerletSticks.
// Endpoints near a static surface are pinned with VerletLocked; endpoints near a
// dynamic body get a RopeEndAnchor that tracks the body's movement each frame.
use avian2d::prelude::*;
use bevy::prelude::*;
use bevy_verlet::prelude::*;

use crate::{GameLayer, GameState, fov::MOVABLE_Z};

const SEGMENT_TARGET_LEN: f32 = 10.0;
// How far from any collider surface a click can be and still anchor.
const ANCHOR_RADIUS: f32 = 60.0;
// Minimum clearance between a rope point and any wall surface.
const ROPE_COLLISION_RADIUS: f32 = 2.5;

#[derive(Component)]
pub struct Rope {
    points: Vec<Entity>,
}

#[derive(Component)]
struct RopePoint;

// Attached to a locked rope endpoint that should follow a dynamic body.
#[derive(Component)]
struct RopeEndAnchor {
    tracked: Entity,
    local: Vec3, // attachment point in the tracked entity's local space
}

#[derive(Resource, Default)]
pub struct RopeDragState {
    start: Option<Vec2>,
}

pub struct RopePlugin;

impl Plugin for RopePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(VerletPlugin::default())
            .insert_resource(VerletConfig {
                gravity: Vec3::new(0.0, -200.0, 0.0),
                friction: 0.02,
                sticks_computation_depth: 5,
                parallel_processing: true,
            })
            .init_resource::<RopeDragState>()
            .add_systems(Update, handle_rope_drag)
            .add_systems(Update, draw_rope)
            .add_systems(FixedPreUpdate, update_tracked_anchors)
            .add_systems(FixedPostUpdate, push_rope_out_of_terrain);
    }
}

fn handle_rope_drag(
    keyboard: Res<ButtonInput<KeyCode>>,
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    mut drag_state: ResMut<RopeDragState>,
    mut commands: Commands,
    spatial_query: SpatialQuery,
    anchor_query: Query<(&GlobalTransform, &RigidBody)>,
) {
    let Some(cursor_pos) = window.cursor_position() else { return };
    let (cam, cam_tf) = *camera;
    let Ok(world_pos) = cam.viewport_to_world_2d(cam_tf, cursor_pos) else { return };

    if keyboard.just_pressed(KeyCode::KeyO) {
        if drag_state.start.is_none() {
            drag_state.start = Some(world_pos);
        } else {
            let start = drag_state.start.take().unwrap();
            spawn_rope(&mut commands, start, world_pos, &spatial_query, &anchor_query);
        }
    }
}

fn spawn_rope(
    commands: &mut Commands,
    start: Vec2,
    end: Vec2,
    spatial_query: &SpatialQuery,
    anchor_query: &Query<(&GlobalTransform, &RigidBody)>,
) {
    let total = (end - start).length();
    if total < SEGMENT_TARGET_LEN {
        return;
    }

    let n_segs = (total / SEGMENT_TARGET_LEN).ceil() as usize;
    let n_pts = n_segs + 1;
    let stick_len = total / n_segs as f32;

    let start_anchor = find_anchor(spatial_query, start);
    let end_anchor = find_anchor(spatial_query, end);

    let mut points = Vec::with_capacity(n_pts);
    for i in 0..n_pts {
        let t = i as f32 / (n_pts - 1) as f32;
        let pos = start.lerp(end, t);
        let mut entity_cmd = commands.spawn((
            DespawnOnExit(GameState::InLevel),
            RopePoint,
            VerletPoint::default(),
            Transform::from_translation(pos.extend(MOVABLE_Z)),
        ));
        let anchor = if i == 0 {
            start_anchor
        } else if i == n_pts - 1 {
            end_anchor
        } else {
            None
        };
        if let Some((anchor_entity, proj_point)) = anchor {
            entity_cmd.insert(VerletLocked);
            if let Ok((gtf, rb)) = anchor_query.get(anchor_entity) {
                if *rb != RigidBody::Static {
                    let local = gtf.affine().inverse().transform_point3(proj_point.extend(0.0));
                    entity_cmd.insert(RopeEndAnchor { tracked: anchor_entity, local });
                }
            }
        }
        points.push(entity_cmd.id());
    }

    for i in 0..n_segs {
        commands.spawn((
            DespawnOnExit(GameState::InLevel),
            VerletStick {
                point_a_entity: points[i],
                point_b_entity: points[i + 1],
                length: stick_len,
            },
            SweptCcd::default(),
        ));
    }

    commands.spawn((DespawnOnExit(GameState::InLevel), Rope { points }));
}

fn draw_rope(ropes: Query<&Rope>, points: Query<&Transform, With<RopePoint>>, mut gizmos: Gizmos) {
    for rope in &ropes {
        for window in rope.points.windows(2) {
            let Ok(tf_a) = points.get(window[0]) else { continue };
            let Ok(tf_b) = points.get(window[1]) else { continue };
            gizmos.line_2d(
                tf_a.translation.truncate(),
                tf_b.translation.truncate(),
                Color::srgb(0.65, 0.45, 0.25),
            );
        }
    }
}

fn update_tracked_anchors(
    mut endpoints: Query<(Entity, &mut Transform, &RopeEndAnchor)>,
    tracked: Query<&GlobalTransform>,
    mut commands: Commands,
) {
    for (entity, mut tf, anchor) in &mut endpoints {
        match tracked.get(anchor.tracked) {
            Ok(gtf) => tf.translation = gtf.transform_point(anchor.local),
            Err(_) => {
                commands.entity(entity).remove::<(VerletLocked, RopeEndAnchor)>();
            }
        }
    }
}

fn push_rope_out_of_terrain(
    mut points: Query<(&mut Transform, &mut VerletPoint), Without<VerletLocked>>,
    spatial_query: SpatialQuery,
) {
    let filter = SpatialQueryFilter::from_mask([GameLayer::Wall]);
    for (mut tf, mut point) in &mut points {
        let pos = tf.translation.truncate();
        let Some(proj) = spatial_query.project_point(pos, true, &filter) else { continue };
        let dist = proj.point.distance(pos);
        if proj.is_inside || dist < ROPE_COLLISION_RADIUS {
            // When inside a solid shape, proj.point is the nearest exit on the boundary
            // and (pos - proj.point) points deeper in.  We need the opposite direction.
            let outward = if dist < 1e-5 {
                Vec2::Y
            } else if proj.is_inside {
                (proj.point - pos) / dist
            } else {
                (pos - proj.point) / dist
            };
            let corrected = (proj.point + outward * ROPE_COLLISION_RADIUS).extend(tf.translation.z);
            tf.translation = corrected;
            point.old_position = Some(corrected);
        }
    }
}

// Returns the nearest collider entity and the projected surface point.
fn find_anchor(spatial_query: &SpatialQuery, pos: Vec2) -> Option<(Entity, Vec2)> {
    let filter = SpatialQueryFilter::from_mask([GameLayer::Wall, GameLayer::Dynamic]);
    let proj = spatial_query.project_point(pos, false, &filter)?;
    if proj.is_inside || proj.point.distance(pos) <= ANCHOR_RADIUS {
        Some((proj.entity, proj.point))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::SEGMENT_TARGET_LEN;
    use crate::GameLayer;
    use avian2d::prelude::*;
    use bevy::prelude::*;
    use bevy_verlet::prelude::*;
    use std::time::Duration;

    fn physics_app(gravity: Vec2) -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            AssetPlugin::default(),
            bevy::scene::ScenePlugin,
            PhysicsPlugins::default(),
            VerletPlugin::default(),
        ))
        .add_systems(FixedPostUpdate, super::push_rope_out_of_terrain)
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
            1.0 / 60.0,
        )))
        .insert_resource(Time::<Virtual>::default())
        .insert_resource(Gravity(gravity))
        .insert_resource(VerletConfig {
            gravity: gravity.extend(0.0),
            friction: 0.02,
            sticks_computation_depth: 5,
            parallel_processing: false,
        });
        app.finish();
        app
    }

    fn tick(app: &mut App) {
        app.world_mut()
            .resource_mut::<Time<Virtual>>()
            .advance_by(Duration::from_secs_f32(1.0 / 60.0));
        app.update();
    }

    fn loc(app: &App, entity: Entity) -> Vec3 {
        app.world().entity(entity).get::<Transform>().unwrap().translation
    }

    #[test]
    fn rope_rests_on_rock_obstacle() {
        use bevy::ecs::system::SystemState;

        // Layout (Y up):
        //
        //   anchor(0,0) ===== rope ===== ball(100,0)
        //                    rock(50,-15)
        //                    (top at Y=-10)
        //
        // Under gravity the rope should sag onto the rock.  If segments pass
        // through the rock the midpoint will be well below -10.

        const N: usize = 10; // 10 segments × SEGMENT_TARGET_LEN = 100 unit rope
        const ROCK_X: f32 = 50.0;
        const ROCK_Y: f32 = -15.0;
        const ROCK_H: f32 = 10.0;
        const ROCK_TOP: f32 = ROCK_Y + ROCK_H / 2.0; // -10.0

        let mut app = physics_app(Vec2::new(0.0, -200.0));

        // Static pin at rope start — needs a Wall-layer collider for find_anchor's project_point.
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

        // Rock obstacle the rope should sag onto.
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

        // Ball anchors the rope end; Dynamic layer so find_anchor can locate it.
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

        // Tick once to populate the physics broadphase so find_anchor's project_point works.
        tick(&mut app);

        // Call the real production spawn_rope via SystemState.
        {
            type Params<'w, 's> = (
                Commands<'w, 's>,
                SpatialQuery<'w, 's>,
                Query<'w, 's, (&'static GlobalTransform, &'static RigidBody)>,
            );
            let mut state: SystemState<Params> = SystemState::new(app.world_mut());
            let (mut commands, spatial_query, anchor_query) = state.get_mut(app.world_mut());
            super::spawn_rope(
                &mut commands,
                Vec2::ZERO,
                Vec2::new(N as f32 * SEGMENT_TARGET_LEN, 0.0),
                &spatial_query,
                &anchor_query,
            );
            state.apply(app.world_mut());
        }

        // find_anchor doesn't reliably locate dynamic bodies in a headless test, so
        // lock both endpoints explicitly by finding the min-x and max-x rope points.
        // Remove any RopeEndAnchor so the endpoints stay fixed in this static test.
        {
            let mut q =
                app.world_mut().query_filtered::<(Entity, &Transform), With<super::RopePoint>>();
            let (start_e, end_e) = q.iter(app.world()).fold(
                (None::<(Entity, f32)>, None::<(Entity, f32)>),
                |(min, max), (e, tf)| {
                    let x = tf.translation.x;
                    (
                        Some(min.map_or((e, x), |(me, mx)| if x < mx { (e, x) } else { (me, mx) })),
                        Some(max.map_or((e, x), |(me, mx)| if x > mx { (e, x) } else { (me, mx) })),
                    )
                },
            );
            for opt in [start_e, end_e] {
                if let Some((e, _)) = opt {
                    app.world_mut()
                        .entity_mut(e)
                        .insert(VerletLocked)
                        .remove::<super::RopeEndAnchor>();
                }
            }
        }

        for frame in 0..100u32 {
            tick(&mut app);
            if frame % 10 == 9 {
                let mut q = app.world_mut().query_filtered::<&Transform, With<super::RopePoint>>();
                let positions: Vec<Vec3> = q.iter(app.world()).map(|t| t.translation).collect();
                let mid_y = positions
                    .iter()
                    .min_by(|a, b| (a.x - ROCK_X).abs().partial_cmp(&(b.x - ROCK_X).abs()).unwrap())
                    .map_or(f32::NEG_INFINITY, |p| p.y);
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

        let mut q = app.world_mut().query_filtered::<&Transform, With<super::RopePoint>>();
        let positions: Vec<Vec3> = q.iter(app.world()).map(|t| t.translation).collect();
        let mid_y = positions
            .iter()
            .min_by(|a, b| (a.x - ROCK_X).abs().partial_cmp(&(b.x - ROCK_X).abs()).unwrap())
            .map_or(f32::NEG_INFINITY, |p| p.y);
        let vel = app.world().entity(ball).get::<LinearVelocity>().unwrap().0;
        println!(
            "Final: ball y={:.1} vel={:.2?}  mid-rope y={:.1}  rock top={:.1}",
            loc(&app, ball).y,
            vel,
            mid_y,
            ROCK_TOP,
        );

        assert!(
            mid_y >= ROCK_TOP - 3.0,
            "Rope passed through rock: mid-rope y={:.1} is below rock top {:.1}",
            mid_y,
            ROCK_TOP,
        );

        // The ball (unconnected to the rope) falls freely under avian2d gravity.
        assert!(loc(&app, ball).y < -40.0);
    }
}
