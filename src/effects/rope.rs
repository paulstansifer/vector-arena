// Drag-to-draw rope with segment physics.
// Press O once to mark the start, once more to mark the end.  `spawn_rope` subdivides
// the line into SEGMENT_TARGET_LEN capsule segments connected by RevoluteJoints.
// Endpoints are anchored to the nearest collider surface within ANCHOR_RADIUS units;
// `project_point` is used so clicking anywhere inside solid rock finds the wall edge.
use avian2d::prelude::*;
use bevy::prelude::*;

use crate::{GameLayer, GameState, fov::MOVABLE_Z};

const SEGMENT_TARGET_LEN: f32 = 10.0;
const ROPE_RADIUS: f32 = 2.5;
const ROPE_VISUAL_RADIUS: f32 = 1.0;
// How far from any collider surface a click can be and still anchor.
// Generous so clicking anywhere inside thick solid rock works.
const ANCHOR_RADIUS: f32 = 60.0;
// Small but visible compliance: joints stretch a few pixels under load.
const ROPE_COMPLIANCE: f32 = 0.0002 * 0.0001;
// cos(~14°): adjacent free segments must be this close to collinear to count as taut.
const TAUT_THRESHOLD: f32 = 0.97;

#[derive(Component)]
pub struct Rope {
    segments: Vec<Entity>,
}

#[derive(Component)]
struct RopeSegment;

#[derive(Resource, Default)]
pub struct RopeDragState {
    start: Option<Vec2>,
}

pub struct RopePlugin;

impl Plugin for RopePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RopeDragState>().add_systems(Update, handle_rope_drag);
        //.add_systems(Update, update_rope_tautness.after(handle_rope_drag));
    }
}

fn handle_rope_drag(
    keyboard: Res<ButtonInput<KeyCode>>,
    window: Single<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    mut drag_state: ResMut<RopeDragState>,
    mut commands: Commands,
    spatial_query: SpatialQuery,
    anchor_query: Query<&GlobalTransform, With<RigidBody>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let Some(cursor_pos) = window.cursor_position() else { return };
    let (cam, cam_tf) = *camera;
    let Ok(world_pos) = cam.viewport_to_world_2d(cam_tf, cursor_pos) else { return };

    if keyboard.just_pressed(KeyCode::KeyO) {
        if drag_state.start.is_none() {
            drag_state.start = Some(world_pos);
        } else {
            let start = drag_state.start.take().unwrap();
            spawn_rope(
                &mut commands,
                start,
                world_pos,
                &spatial_query,
                &anchor_query,
                &mut meshes,
                &mut materials,
            );
        }
    }
}

fn spawn_rope(
    commands: &mut Commands,
    start: Vec2,
    end: Vec2,
    spatial_query: &SpatialQuery,
    anchor_query: &Query<&GlobalTransform, With<RigidBody>>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
) {
    let total = (end - start).length();
    if total < SEGMENT_TARGET_LEN {
        return;
    }

    let dir = (end - start) / total;
    let n = (total / SEGMENT_TARGET_LEN).ceil() as usize;
    let spacing = total / n as f32;
    // half_inner: distance from segment center to hemisphere center along X.
    // Keeps the physical tip exactly at ±spacing/2, so adjacent segments touch but don't overlap.
    let half_inner = (spacing / 2.0 - ROPE_RADIUS).max(0.1);
    let angle = dir.y.atan2(dir.x);

    // Overlap a little bit so the rope doesn't become a dashed line.
    let seg_mesh = meshes.add(Rectangle::new(spacing * 1.1, ROPE_VISUAL_RADIUS * 2.0));
    let seg_mat = materials.add(Color::srgb(0.65, 0.45, 0.25));

    let mut segments = Vec::with_capacity(n);
    for i in 0..n {
        let pos = start + dir * ((i as f32 + 0.5) * spacing);
        let seg = commands
            .spawn((
                DespawnOnExit(GameState::InLevel),
                RopeSegment,
                RigidBody::Dynamic,
                Collider::capsule_endpoints(
                    ROPE_RADIUS,
                    Vec2::new(-half_inner, 0.0),
                    Vec2::new(half_inner, 0.0),
                ),
                CollidingEntities::default(),
                CollisionLayers::new(GameLayer::Rope, [GameLayer::Wall]),
                Mesh2d(seg_mesh.clone()),
                MeshMaterial2d(seg_mat.clone()),
                Transform::from_translation(pos.extend(MOVABLE_Z))
                    .with_rotation(Quat::from_rotation_z(angle)),
                LinearDamping(3.0),
                AngularDamping(3.0),
                // Don't bounce:
                Restitution { coefficient: 0.0, combine_rule: CoefficientCombine::Min },
                Mass(0.5),
                // SweptCcd::default(),  // TODO: not enough to fix everything yet
            ))
            .id();
        segments.push(seg);
    }

    // The joint attaches at each segment's physical tip: half_inner (hemisphere centre)
    // + ROPE_RADIUS (hemisphere radius) = spacing/2.  This places the joint exactly
    // where adjacent capsules are touching in their spawn layout, so there is zero
    // initial constraint violation and the rope doesn't snap on the first frame.
    let tip = spacing / 2.0;

    // Hinge joints between adjacent segments, with compliance for elasticity.
    for i in 0..n - 1 {
        let (a, b) = (segments[i], segments[i + 1]);
        commands.spawn((
            DespawnOnExit(GameState::InLevel),
            RevoluteJoint::new(a, b)
                .with_local_anchor1(Vec2::new(tip, 0.0))
                .with_local_anchor2(Vec2::new(-tip, 0.0))
                .with_point_compliance(ROPE_COMPLIANCE),
            // TODO: This needs more tuning to make the rope less jiggly.
            JointDamping { linear: 15.0, angular: 10.0 },
        ));
    }

    // Pin the start of the rope to whatever is at the drag origin.
    if let Some((anchor, anchor_pos)) = find_anchor(spatial_query, start) {
        let local = world_to_local(anchor, anchor_pos, anchor_query);
        commands.spawn((
            DespawnOnExit(GameState::InLevel),
            RevoluteJoint::new(anchor, segments[0])
                .with_local_anchor1(local)
                .with_local_anchor2(Vec2::new(-tip, 0.0)),
        ));
        // Wall collision would eject the segment while the joint tries to hold it.
        commands
            .entity(segments[0])
            .insert(CollisionLayers::new(GameLayer::Rope, [GameLayer::Rope; 0]));
    }

    // Pin the end.
    if let Some((anchor, anchor_pos)) = find_anchor(spatial_query, end) {
        let local = world_to_local(anchor, anchor_pos, anchor_query);
        let last = *segments.last().unwrap();
        commands.spawn((
            DespawnOnExit(GameState::InLevel),
            RevoluteJoint::new(anchor, last)
                .with_local_anchor1(local)
                .with_local_anchor2(Vec2::new(tip, 0.0)),
        ));
        commands.entity(last).insert(CollisionLayers::new(GameLayer::Rope, [GameLayer::Rope; 0]));
    }

    commands.spawn(Rope { segments });
}

// Returns the nearest collider entity and the projected surface point.
// Uses project_point so a click anywhere inside solid rock (whose collider is a polyline
// boundary, not a filled shape) still finds the wall and snaps to its surface.
fn find_anchor(spatial_query: &SpatialQuery, pos: Vec2) -> Option<(Entity, Vec2)> {
    let filter = SpatialQueryFilter::from_mask([GameLayer::Wall, GameLayer::Dynamic]);
    let proj = spatial_query.project_point(pos, false, &filter)?;
    // is_inside is true for solid bodies; for polyline terrain it's always false,
    // so fall back to a distance check generous enough to cover thick rock walls.
    if proj.is_inside || proj.point.distance(pos) <= ANCHOR_RADIUS {
        Some((proj.entity, proj.point))
    } else {
        None
    }
}

fn world_to_local(
    entity: Entity,
    world_pos: Vec2,
    query: &Query<&GlobalTransform, With<RigidBody>>,
) -> Vec2 {
    query
        .get(entity)
        .map(|gtf| gtf.affine().inverse().transform_point3(world_pos.extend(0.0)).truncate())
        .unwrap_or(world_pos)
}

#[cfg(test)]
mod tests {
    use super::SEGMENT_TARGET_LEN;
    use crate::GameLayer;
    use avian2d::prelude::*;
    use bevy::prelude::*;
    use std::time::Duration;

    fn physics_app(gravity: Vec2) -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            AssetPlugin::default(),
            bevy::scene::ScenePlugin,
            PhysicsPlugins::default(),
        ))
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
            1.0 / 60.0,
        )))
        .insert_resource(Time::<Virtual>::default())
        .insert_resource(Gravity(gravity));
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

        // spawn_rope creates Mesh2d/MeshMaterial2d assets; register the types headlessly.
        app.init_asset::<Mesh>();
        app.init_asset::<ColorMaterial>();

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
                Query<'w, 's, &'static GlobalTransform, With<RigidBody>>,
                ResMut<'w, Assets<Mesh>>,
                ResMut<'w, Assets<ColorMaterial>>,
            );
            let mut state: SystemState<Params> = SystemState::new(app.world_mut());
            let (mut commands, spatial_query, anchor_query, mut meshes, mut materials) =
                state.get_mut(app.world_mut());
            super::spawn_rope(
                &mut commands,
                Vec2::ZERO,
                Vec2::new(N as f32 * SEGMENT_TARGET_LEN, 0.0),
                &spatial_query,
                &anchor_query,
                &mut meshes,
                &mut materials,
            );
            state.apply(app.world_mut());
        }

        // find_anchor doesn't reliably locate dynamic bodies in a headless test (the
        // broadphase may not expose them to project_point), so mirror the old test and
        // explicitly join the ball to the last rope segment.
        {
            let mut q = app
                .world_mut()
                .query_filtered::<(Entity, &Transform), With<super::RopeSegment>>();
            let last_seg = q
                .iter(app.world())
                .max_by(|a, b| {
                    a.1.translation.x.partial_cmp(&b.1.translation.x).unwrap()
                })
                .map(|(e, _)| e);
            if let Some(last) = last_seg {
                let tip = SEGMENT_TARGET_LEN / 2.0;
                app.world_mut().spawn(
                    RevoluteJoint::new(last, ball)
                        .with_local_anchor1(Vec2::new(tip, 0.0))
                        .with_local_anchor2(Vec2::ZERO),
                );
            }
        }

        for frame in 0..100u32 {
            tick(&mut app);
            if frame % 10 == 9 {
                let mut q =
                    app.world_mut().query_filtered::<&Transform, With<super::RopeSegment>>();
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

        let mut q = app.world_mut().query_filtered::<&Transform, With<super::RopeSegment>>();
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

        // The ball should have gone downward, but gotten stopped by the rope.
        assert!(loc(&app, ball).y < -40.0);
        assert!(loc(&app, ball).y >= -60.0);
    }
}

// Making taut ropes "move up" and collide is cute, but may be unworkable:
#[allow(unused)]
fn update_rope_tautness(
    ropes: Query<&Rope>,
    segment_query: Query<(&Transform, &CollidingEntities), With<RopeSegment>>,
    mut layers_query: Query<&mut CollisionLayers, With<RopeSegment>>,
) {
    for rope in &ropes {
        let taut = check_taut(&rope.segments, &segment_query);
        for &seg in &rope.segments {
            let Ok(mut layers) = layers_query.get_mut(seg) else { continue };
            *layers = if taut {
                // Become a dynamic obstacle: collides with terrain and other physics objects.
                CollisionLayers::new(GameLayer::Dynamic, [GameLayer::Wall, GameLayer::Dynamic])
            } else {
                CollisionLayers::new(GameLayer::Rope, [GameLayer::Wall])
            };
        }
    }
}

// The rope is taut if every pair of adjacent segments that aren't touching anything
// are approximately collinear. Requires at least one such free pair.
#[allow(unused)]
fn check_taut(
    segments: &[Entity],
    query: &Query<(&Transform, &CollidingEntities), With<RopeSegment>>,
) -> bool {
    let mut any_free = false;
    for w in segments.windows(2) {
        let Ok((tf_a, coll_a)) = query.get(w[0]) else { continue };
        let Ok((tf_b, coll_b)) = query.get(w[1]) else { continue };
        if !coll_a.is_empty() || !coll_b.is_empty() {
            continue;
        }
        any_free = true;
        let dir_a = (tf_a.rotation * Vec3::X).truncate();
        let dir_b = (tf_b.rotation * Vec3::X).truncate();
        if dir_a.dot(dir_b) < TAUT_THRESHOLD {
            return false;
        }
    }
    any_free
}
