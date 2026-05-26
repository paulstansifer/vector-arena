// Drag-to-draw rope with Verlet physics.
// Press O once to mark the start, once more to mark the end.  `spawn_rope` places
// verlet points spaced by SEGMENT_TARGET_LEN and connects them with VerletSticks.
// Endpoints near a static surface are pinned with VerletLocked; endpoints near a
// dynamic body get a RopeEndAnchor that tracks the body's movement each frame.
use avian2d::prelude::*;
use bevy::prelude::*;
use bevy_verlet::prelude::*;

use crate::{GameLayer, GameState, fov::MOVABLE_Z};

pub const SEGMENT_TARGET_LEN: f32 = 10.0;
// How far from any collider surface a click can be and still anchor.
const ANCHOR_RADIUS: f32 = 60.0;
// Minimum clearance between a rope point and any wall surface.
const ROPE_COLLISION_RADIUS: f32 = 2.5;

#[derive(Component)]
pub struct Rope {
    points: Vec<Entity>,
}

#[derive(Component)]
pub struct RopePoint;

// Attached to a locked rope endpoint that should follow a dynamic body.
#[derive(Component)]
pub struct RopeEndAnchor {
    pub tracked: Entity,
    pub local: Vec3, // attachment point in the tracked entity's local space
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

pub fn spawn_rope(
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

/// Register the rope simulation systems into a test app.
/// Call this after `bevy_verlet::VerletPlugin` has been added.
pub fn add_rope_test_systems(app: &mut App) {
    app.add_systems(FixedPreUpdate, update_tracked_anchors)
        .add_systems(FixedPostUpdate, push_rope_out_of_terrain);
}
