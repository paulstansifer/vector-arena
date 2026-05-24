// Drag-to-draw rope with segment physics.
// Press O and drag to place a rope.  `spawn_rope` subdivides the line into
// SEGMENT_TARGET_LEN capsule segments connected by RevoluteJoints with slight
// compliance, anchoring endpoints to whatever entity (or terrain) is within
// 3 units.  Segments collide with Wall only.
use avian2d::prelude::*;
use bevy::prelude::*;

use crate::{GameLayer, GameState, fov::MOVABLE_Z};

const SEGMENT_TARGET_LEN: f32 = 10.0;
const ROPE_RADIUS: f32 = 1.0;
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
        drag_state.start = Some(world_pos);
    }

    if keyboard.just_released(KeyCode::KeyO) {
        let Some(start) = drag_state.start.take() else { return };
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
    let seg_mesh = meshes.add(Rectangle::new(spacing * 1.1, ROPE_RADIUS * 2.0));
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
            ))
            .id();
        segments.push(seg);
    }

    // Hinge joints between adjacent segments, with compliance for elasticity.
    for i in 0..n - 1 {
        let (a, b) = (segments[i], segments[i + 1]);
        commands.spawn((
            DespawnOnExit(GameState::InLevel),
            RevoluteJoint::new(a, b)
                .with_local_anchor1(Vec2::new(half_inner, 0.0))
                .with_local_anchor2(Vec2::new(-half_inner, 0.0))
                .with_point_compliance(ROPE_COMPLIANCE),
            // TODO: This needs more tuning to make the rope less jiggly.
            JointDamping { linear: 15.0, angular: 10.0 },
        ));
    }

    // TODO: anchoring to terrain doesn't actually work

    // Pin the start of the rope to whatever is at the drag origin.
    if let Some(anchor) = find_anchor(spatial_query, start) {
        let local = world_to_local(anchor, start, anchor_query);
        commands.spawn((
            DespawnOnExit(GameState::InLevel),
            RevoluteJoint::new(anchor, segments[0])
                .with_local_anchor1(local)
                .with_local_anchor2(Vec2::new(-half_inner, 0.0)),
        ));
    }

    // Pin the end.
    if let Some(anchor) = find_anchor(spatial_query, end) {
        let local = world_to_local(anchor, end, anchor_query);
        let last = *segments.last().unwrap();
        commands.spawn((
            DespawnOnExit(GameState::InLevel),
            RevoluteJoint::new(anchor, last)
                .with_local_anchor1(local)
                .with_local_anchor2(Vec2::new(half_inner, 0.0)),
        ));
    }

    commands.spawn(Rope { segments });
}

fn find_anchor(spatial_query: &SpatialQuery, pos: Vec2) -> Option<Entity> {
    spatial_query
        .shape_intersections(
            &Collider::circle(3.0),
            pos,
            0.0,
            &SpatialQueryFilter::from_mask([GameLayer::Wall, GameLayer::Dynamic]),
        )
        .first()
        .copied()
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
