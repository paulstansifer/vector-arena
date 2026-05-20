use bevy::prelude::*;
use geo::Contains;

use crate::{WorldBounds, player::Player, terrain::DungeonState};

const MISSILE_SPEED: f32 = 600.0;
const TRAIL_LIFETIME: f32 = 1.5;
const MISSILE_Z: f32 = 12.0;
const TRAIL_Z: f32 = 11.0;
const MISSILE_RADIUS: f32 = 4.0;
const TRAIL_WIDTH: f32 = 4.0;

// Purple-ish magic missile color
const R: f32 = 0.7;
const G: f32 = 0.3;
const B: f32 = 1.0;

#[derive(Component)]
pub struct MagicMissile {
    velocity: Vec2,
}

#[derive(Component)]
pub struct TrailSegment {
    born_at: f32,
}

pub fn fire_missile(
    window: Single<&Window>,
    camera_query: Single<(&Camera, &GlobalTransform)>,
    keyboard: Res<ButtonInput<KeyCode>>,
    player_query: Single<&Transform, With<Player>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    if !keyboard.just_pressed(KeyCode::Space) {
        return;
    }

    let Some(cursor_pos) = window.cursor_position() else { return };
    let (camera, cam_transform) = *camera_query;
    let Ok(world_cursor) = camera.viewport_to_world_2d(cam_transform, cursor_pos) else { return };

    let player_pos = player_query.translation.truncate();
    let dir = (world_cursor - player_pos).normalize_or_zero();
    if dir == Vec2::ZERO {
        return;
    }

    let spawn_pos = player_pos + dir * (crate::AGENT_RADIUS + MISSILE_RADIUS + 1.0);

    commands.spawn((
        MagicMissile { velocity: dir * MISSILE_SPEED },
        Mesh2d(meshes.add(Circle::new(MISSILE_RADIUS))),
        MeshMaterial2d(materials.add(ColorMaterial::from(Color::srgb(R, G, B)))),
        Transform::from_translation(spawn_pos.extend(MISSILE_Z)),
    ));
}

pub fn update_missiles(
    mut commands: Commands,
    time: Res<Time<Real>>,
    dungeon: Res<DungeonState>,
    bounds: Res<WorldBounds>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut query: Query<(Entity, &mut Transform, &MagicMissile)>,
) {
    let now = time.elapsed_secs();
    for (entity, mut transform, missile) in query.iter_mut() {
        let old_pos = transform.translation.truncate();
        let new_pos = old_pos + missile.velocity * time.delta_secs();

        let hit_wall = dungeon.solid_rock.contains(&geo::Point::new(new_pos.x, new_pos.y));
        let out_of_bounds =
            new_pos.x.abs() > bounds.width / 2.0 || new_pos.y.abs() > bounds.height / 2.0;

        // Spawn a trail segment covering this frame's movement
        let diff = new_pos - old_pos;
        let length = diff.length();
        if length > 0.1 {
            let mid = (old_pos + new_pos) / 2.0;
            let angle = diff.y.atan2(diff.x);
            commands.spawn((
                TrailSegment { born_at: now },
                Mesh2d(meshes.add(Rectangle::new(length, TRAIL_WIDTH))),
                MeshMaterial2d(materials.add(ColorMaterial::from(Color::srgba(R, G, B, 1.0)))),
                Transform {
                    translation: mid.extend(TRAIL_Z),
                    rotation: Quat::from_rotation_z(angle),
                    ..default()
                },
            ));
        }

        if hit_wall || out_of_bounds {
            commands.entity(entity).despawn();
        } else {
            transform.translation = new_pos.extend(MISSILE_Z);
        }
    }
}

pub fn fade_trails(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    query: Query<(Entity, &TrailSegment, &MeshMaterial2d<ColorMaterial>)>,
) {
    let now = time.elapsed_secs();
    for (entity, trail, mat_handle) in query.iter() {
        let t = ((now - trail.born_at) / TRAIL_LIFETIME).clamp(0.0, 1.0);

        if t >= 1.0 {
            commands.entity(entity).despawn();
            continue;
        }

        // 1 - t^3: nearly opaque for most of the lifetime, then rapid drop near the end
        let alpha = 1.0 - t * t * t;

        if let Some(mat) = materials.get_mut(mat_handle.id()) {
            mat.color = Color::srgba(R, G, B, alpha);
        }
    }
}
