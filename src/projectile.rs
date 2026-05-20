use avian2d::prelude::*;
use bevy::prelude::*;
use rand::Rng;
use std::time::Duration;

use crate::{
    AGENT_RADIUS, GameLayer,
    fov::MOVABLE_Z,
    monster::Monster,
    player::{MoveTarget, PLAYER_SPEED, Player},
};

pub const MISSILE_SPEED: f32 = PLAYER_SPEED * 20.0;
pub const MISSILE_MAX_DISTANCE: f32 = 1000.0;
pub const MONSTER_FIRE_RANGE: f32 = 100.0;
const TIME_SCALE_MISSILE: f32 = 0.05;
const KNOCKBACK_SPEED: f32 = 6000.0;

#[derive(Component)]
pub struct MagicMissile {
    pub distance_traveled: f32,
    pub fired_by_player: bool,
    knocked_back: Vec<Entity>,
}

#[derive(Component)]
pub struct MonsterShootTimer(pub f32);

impl MonsterShootTimer {
    pub fn new() -> Self {
        let mut rng = rand::thread_rng();
        Self(rng.gen_range(1.0..2.0))
    }
}

fn spawn_missile(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    origin: Vec2,
    direction: Vec2,
    fired_by_player: bool,
) {
    let spawn_pos = origin + direction * (AGENT_RADIUS + 6.0);
    let color =
        if fired_by_player { Color::srgb(0.4, 0.6, 1.0) } else { Color::srgb(1.0, 0.4, 0.2) };

    commands.spawn((
        MagicMissile { distance_traveled: 0.0, fired_by_player, knocked_back: Vec::new() },
        Mesh2d(meshes.add(Circle::new(4.0))),
        MeshMaterial2d(materials.add(ColorMaterial::from(color))),
        Transform::from_translation(spawn_pos.extend(MOVABLE_Z + 1.0)),
        RigidBody::Dynamic,
        Collider::circle(4.0),
        CollisionLayers::new(GameLayer::Missile, GameLayer::Wall),
        LockedAxes::ROTATION_LOCKED,
        LinearVelocity(direction * MISSILE_SPEED),
        Mass(0.1),
        Restitution { coefficient: 1.0, combine_rule: CoefficientCombine::Max },
        Friction {
            dynamic_coefficient: 0.0,
            static_coefficient: 0.0,
            combine_rule: CoefficientCombine::Min,
        },
        SweptCcd::default(),
    ));
}

pub fn player_fire_missile(
    keyboard: Res<ButtonInput<KeyCode>>,
    window: Single<&Window>,
    camera_query: Single<(&Camera, &GlobalTransform)>,
    player_query: Single<&Transform, With<Player>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    if !keyboard.just_pressed(KeyCode::KeyM) {
        return;
    }

    let Some(cursor_pos) = window.cursor_position() else { return };
    let (camera, camera_transform) = *camera_query;
    let Ok(world_pos) = camera.viewport_to_world_2d(camera_transform, cursor_pos) else { return };

    let player_pos = player_query.translation.truncate();
    let direction = (world_pos - player_pos).normalize_or_zero();
    if direction == Vec2::ZERO {
        return;
    }

    spawn_missile(&mut commands, &mut meshes, &mut materials, player_pos, direction, true);
}

pub fn monster_fire_missiles(
    time: Res<Time>,
    player_query: Single<&Transform, With<Player>>,
    mut monster_query: Query<(&Transform, &mut MonsterShootTimer), With<Monster>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let player_pos = player_query.translation.truncate();

    for (transform, mut timer) in monster_query.iter_mut() {
        timer.0 -= time.delta_secs();
        if timer.0 > 0.0 {
            continue;
        }

        let mut rng = rand::thread_rng();
        timer.0 = rng.gen_range(1.0..2.0);

        let monster_pos = transform.translation.truncate();
        if monster_pos.distance(player_pos) > MONSTER_FIRE_RANGE {
            continue;
        }

        let direction = (player_pos - monster_pos).normalize_or_zero();
        spawn_missile(&mut commands, &mut meshes, &mut materials, monster_pos, direction, false);
    }
}

pub fn update_missiles(
    mut commands: Commands,
    time: Res<Time>,
    mut query: Query<(Entity, &mut MagicMissile, &LinearVelocity)>,
) {
    for (entity, mut missile, velocity) in query.iter_mut() {
        missile.distance_traveled += velocity.length() * time.delta_secs();
        if missile.distance_traveled >= MISSILE_MAX_DISTANCE {
            commands.entity(entity).despawn();
        }
    }
}

/// For each missile, find overlapping Dynamic-layer entities and apply a one-shot velocity push.
/// Missiles don't physically interact with Dynamic entities (collision layers exclude this),
/// so we handle the knockback manually here.
pub fn apply_missile_knockback(
    spatial_query: SpatialQuery,
    mut missiles: Query<(&Transform, &LinearVelocity, &mut MagicMissile)>,
    mut dynamic_query: Query<&mut LinearVelocity, (Without<MagicMissile>, With<RigidBody>)>,
) {
    for (transform, missile_vel, mut missile) in missiles.iter_mut() {
        let pos = transform.translation.truncate();
        let knockback_dir = missile_vel.0.normalize_or_zero();

        let hits = spatial_query.shape_intersections(
            &Collider::circle(4.0),
            pos,
            0.0,
            &SpatialQueryFilter::from_mask(GameLayer::Dynamic),
        );

        for hit in hits {
            if missile.knocked_back.contains(&hit) {
                continue;
            }
            missile.knocked_back.push(hit);
            if let Ok(mut vel) = dynamic_query.get_mut(hit) {
                vel.0 += knockback_dir * KNOCKBACK_SPEED;
            }
        }
    }
}

/// TODO: move this to another file!
/// Single source of truth for virtual time scale and physics step rate.
/// Replaces the time-setting code that used to live in player.rs.
pub fn manage_time_scale(
    mut time: ResMut<Time<Virtual>>,
    mut fixed_time: ResMut<Time<Fixed>>,
    missile_query: Query<&MagicMissile>,
    move_target: Single<&MoveTarget, With<Player>>,
) {
    let any_missile = missile_query.iter().next().is_some();

    if any_missile {
        time.set_relative_speed(TIME_SCALE_MISSILE);
        // Time<Fixed> accumulates from virtual time, so at 0.05× virtual speed physics would
        // drop to 3.2 Hz real-time (one step every ~312 ms). Compensate by running the fixed
        // schedule 20× faster in virtual terms so it still fires ~64 Hz in real time.
        fixed_time.set_timestep(Duration::from_secs_f64(1.0 / (64.0 / TIME_SCALE_MISSILE as f64)));
    } else {
        fixed_time.set_timestep(Duration::from_secs_f64(1.0 / 64.0));
        if move_target.active {
            time.set_relative_speed(1.0);
        } else {
            time.set_relative_speed(0.0);
        }
    }
}
