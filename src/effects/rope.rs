// Drag-to-draw rope with Verlet physics.
// Press O once to mark the start, once more to mark the end.  `spawn_rope` places
// verlet points spaced by SEGMENT_TARGET_LEN and connects them with VerletSticks.
// Endpoints near a static surface are pinned with VerletLocked; endpoints near a
// dynamic body get a RopeEndAnchor that tracks the body's movement each frame.
//
// Collision correctness: terrain projection runs after *each* stick-constraint
// iteration (not just at the end) so that constraint re-projection can never push
// a point back through a wall that was already corrected.
use avian2d::prelude::*;
use bevy::{input::keyboard::Key, platform::collections::HashMap, prelude::*};
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
        app.add_plugins(VerletPlugin { custom_sticks: true, ..VerletPlugin::default() })
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
            .add_systems(
                FixedUpdate,
                (
                    update_rope_sticks_with_collision.after(VerletSet::Points),
                    apply_rope_tension.after(update_rope_sticks_with_collision),
                ),
            );
    }
}

fn handle_rope_drag(
    keyboard: Res<ButtonInput<Key>>,
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

    if keyboard.just_pressed(Key::Character("o".into())) {
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
            if let Ok((gtf, rb)) = anchor_query.get(anchor_entity)
                && *rb != RigidBody::Static
            {
                let local = gtf.affine().inverse().transform_point3(proj_point.extend(0.0));
                entity_cmd.insert(RopeEndAnchor { tracked: anchor_entity, local });
            }
        }
        points.push(entity_cmd.id());
    }

    for i in 0..n_segs {
        commands.spawn((DespawnOnExit(GameState::InLevel), VerletStick {
            point_a_entity: points[i],
            point_b_entity: points[i + 1],
            length: stick_len,
        }));
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

/// Combined stick-constraint + terrain-collision system.
///
/// Each iteration of the constraint solver is followed by a terrain projection
/// pass so that re-projection of sticks can never push a point back through a
/// wall that was just corrected.
fn update_rope_sticks_with_collision(
    config: Res<VerletConfig>,
    sticks_query: Query<&VerletStick>,
    mut points_query: Query<
        (Entity, &mut Transform, &mut VerletPoint, Option<&VerletLocked>),
        With<RopePoint>,
    >,
    spatial_query: SpatialQuery,
) {
    let filter = SpatialQueryFilter::from_mask([GameLayer::Wall]);

    // Build a map from entity → (transform, point, is_locked) for the stick solver.
    // We need to split borrows, so collect indices into a Vec and use index-based access.
    // Instead, we collect the data we need into a flat Vec and use get_disjoint_mut for sticks.
    //
    // For the stick solver we only need Transform + locked flag; for terrain we also need
    // VerletPoint. We use the full query for both.
    let mut points_map: HashMap<Entity, (Mut<Transform>, Mut<VerletPoint>, bool)> = points_query
        .iter_mut()
        .map(|(e, tf, vp, locked)| (e, (tf, vp, locked.is_some())))
        .collect();

    for _ in 0..=config.sticks_computation_depth {
        // --- Stick constraint projection (mirrors update_sticks inner body) ---
        for stick in sticks_query.iter() {
            let [Some((tf_a, _, a_locked)), Some((tf_b, _, b_locked))] =
                points_map.get_disjoint_mut([&stick.point_a_entity, &stick.point_b_entity])
            else {
                continue;
            };
            if *a_locked && *b_locked {
                continue;
            }
            let (coords_a, coords_b) = (tf_a.translation, tf_b.translation);
            let center = (coords_a + coords_b) / 2.0;
            let direction = (coords_a - coords_b).normalize() * stick.length / 2.0;
            if !*a_locked {
                tf_a.translation =
                    if *b_locked { tf_b.translation + direction * 2.0 } else { center + direction };
            }
            if !*b_locked {
                tf_b.translation =
                    if *a_locked { tf_a.translation - direction * 2.0 } else { center - direction };
            }
        }

        // --- Terrain projection pass ---
        for (_, (tf, point, is_locked)) in &mut points_map {
            if *is_locked {
                continue;
            }
            correct_point(tf, point, &spatial_query, &filter);
        }
    }
}

/// Push a single unlocked rope point out of terrain geometry.
fn correct_point(
    tf: &mut Transform,
    point: &mut VerletPoint,
    spatial_query: &SpatialQuery,
    filter: &SpatialQueryFilter,
) {
    let pos = tf.translation.truncate();
    let old_pos = point.old_position.map_or(pos, |p| p.truncate());
    let delta = pos - old_pos;
    let move_dist = delta.length();

    // Swept check: catch tunneling along the movement path.
    if move_dist > 1e-5
        && let Ok(dir) = Dir2::new(delta)
        && let Some(hit) = spatial_query.cast_ray(old_pos, dir, move_dist, true, filter)
    {
        let hit_point = old_pos + *dir * hit.distance;
        let corrected =
            (hit_point + hit.normal * (ROPE_COLLISION_RADIUS + 0.5)).extend(tf.translation.z);
        tf.translation = corrected;
        point.old_position = Some(corrected);
        return;
    }

    // Static check: point inside or too close to terrain surface.
    let Some(proj) = spatial_query.project_point(pos, true, filter) else { return };
    let dist = proj.point.distance(pos);
    if proj.is_inside || dist < ROPE_COLLISION_RADIUS {
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

/// Cancel the velocity component that would stretch the rope beyond its rest length.
///
/// For each rope endpoint attached to a dynamic body via `RopeEndAnchor`, we look at
/// the adjacent stick. If that stick is already at or beyond its rest length AND the
/// body is moving further away, we zero out the "away" component of the body's velocity.
/// This makes the rope behave like a true inextensible cord rather than a spring.
fn apply_rope_tension(
    rope_query: Query<&Rope>,
    point_query: Query<&Transform, With<RopePoint>>,
    anchor_query: Query<&RopeEndAnchor>,
    stick_query: Query<&VerletStick>,
    mut body_query: Query<&mut LinearVelocity>,
) {
    // Build a lookup from (endpoint_entity, neighbor_entity) → stick rest length.
    let stick_lengths: HashMap<(Entity, Entity), f32> = stick_query
        .iter()
        .flat_map(|s| {
            [
                ((s.point_a_entity, s.point_b_entity), s.length),
                ((s.point_b_entity, s.point_a_entity), s.length),
            ]
        })
        .collect();

    for rope in &rope_query {
        let n = rope.points.len();
        if n < 2 {
            continue;
        }
        // Check both endpoints: (endpoint index, neighbor index)
        for (ep_idx, nb_idx) in [(0, 1), (n - 1, n - 2)] {
            let (endpoint_e, neighbor_e) = (rope.points[ep_idx], rope.points[nb_idx]);
            let Ok(anchor) = anchor_query.get(endpoint_e) else { continue };
            let Ok(ep_tf) = point_query.get(endpoint_e) else { continue };
            let Ok(nb_tf) = point_query.get(neighbor_e) else { continue };
            let Ok(mut body_vel) = body_query.get_mut(anchor.tracked) else { continue };

            let ep_pos = ep_tf.translation.truncate();
            let nb_pos = nb_tf.translation.truncate();
            // Vector from rope interior toward the body (= the stretching direction).
            let sep = ep_pos - nb_pos;
            let dist = sep.length();
            if dist < 1e-5 {
                continue;
            }
            let away_dir = sep / dist;

            // Only constrain when the adjacent stick is actually taut.
            let Some(&rest_len) = stick_lengths.get(&(endpoint_e, neighbor_e)) else { continue };
            if dist <= rest_len {
                continue;
            }

            // Zero out the velocity component pulling the body further from the rope.
            let vel_away = body_vel.dot(away_dir);
            if vel_away > 0.0 {
                body_vel.0 -= away_dir * vel_away;
            }
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
/// Call this after `VerletPlugin { custom_sticks: true }` has been added.
pub fn add_rope_test_systems(app: &mut App) {
    app.add_systems(FixedPreUpdate, update_tracked_anchors).add_systems(
        FixedUpdate,
        (
            update_rope_sticks_with_collision.after(VerletSet::Points),
            apply_rope_tension.after(update_rope_sticks_with_collision),
        ),
    );
}
