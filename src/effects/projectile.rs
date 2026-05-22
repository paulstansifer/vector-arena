// Magic missiles, trails, knockback, and bullet-time.
// Missiles use the Missile physics layer (collides with Wall only); knockback
// against Dynamic bodies is applied manually by querying overlaps rather than
// via collision events.  `manage_time_scale` sets Time<Virtual> to 0.25× while
// any missile exists and adjusts the fixed-timestep period to keep physics at
// ~64 Hz real-time regardless of scale.
use avian2d::prelude::*;
use bevy::prelude::*;
use rand::Rng;
use std::{collections::HashSet, time::Duration};

use crate::{
    AGENT_RADIUS, GameLayer, GameState,
    fov::MOVABLE_Z,
    monster::{AlertedByMissile, Monster, Stats},
    player::{MoveTarget, Player},
    ui::MessageLog,
};

pub const MISSILE_SPEED: f32 = 3500.0;
pub const MISSILE_MAX_DISTANCE: f32 = 1000.0;
pub const MONSTER_FIRE_RANGE: f32 = 100.0;
const TIME_SCALE_MISSILE: f32 = 0.5;
const KNOCKBACK_SPEED: f32 = 600.0;
const KNOCKBACK_COOLDOWN: f32 = 0.15; // virtual seconds; prevents double-hits per pass
const MISSILE_DAMAGE: f32 = 5.0;
const HIT_FLASH_DURATION: f32 = 0.2; // virtual seconds
const TRAIL_CORE_HEIGHT: f32 = 1.0;
const TRAIL_GLOW_HEIGHT: f32 = 4.0;

#[derive(Resource)]
pub struct TrailMeshes {
    core: Handle<Mesh>,
    glow: Handle<Mesh>,
}

pub fn init_trail_meshes(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    commands.insert_resource(TrailMeshes {
        core: meshes.add(Rectangle::new(1.0, TRAIL_CORE_HEIGHT)),
        glow: meshes.add(Rectangle::new(1.0, TRAIL_GLOW_HEIGHT)),
    });
}

#[derive(Component)]
pub struct MissileTrail {
    source_missile: Entity,
    fired_by_player: bool,
    is_glow: bool,
    // 0.0 (head/newest) to 0.5 (tail/oldest) virtual seconds of extra life after missile dies.
    extra_lifetime: f32,
    // Virtual-time timestamp at which to despawn; None while the missile is still alive.
    expiration: Option<f32>,
}

#[derive(Component)]
pub struct MagicMissile {
    pub distance_traveled: f32,
    pub fired_by_player: bool,
    last_trail_pos: Option<Vec2>,
    last_trail_vel: Option<Vec2>,
}

#[derive(Component)]
pub struct KnockbackCooldown(f32);

#[derive(Component)]
pub struct HitFlash {
    timer: f32,
    duration: f32,
    base_color: Color,
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
        DespawnOnExit(GameState::InLevel),
        MagicMissile {
            distance_traveled: 0.0,
            fired_by_player,
            last_trail_pos: None,
            last_trail_vel: None,
        },
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

/// For each missile, find overlapping Dynamic-layer entities and apply knockback, damage, and a
/// white flash. A short per-entity cooldown prevents double-hits from simultaneous missiles
/// while allowing a bounced missile to hit again on a later pass.
pub fn apply_missile_knockback(
    mut commands: Commands,
    spatial_query: SpatialQuery,
    missiles: Query<(&Transform, &LinearVelocity, &MagicMissile)>,
    mut dynamic_query: Query<
        (
            &mut LinearVelocity,
            Option<&KnockbackCooldown>,
            Option<&mut Stats>,
            Option<&MeshMaterial2d<ColorMaterial>>,
            Option<&HitFlash>,
            Has<Player>,
            Has<Monster>,
        ),
        (Without<MagicMissile>, With<RigidBody>),
    >,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut message_log: ResMut<MessageLog>,
) {
    for (transform, missile_vel, missile) in missiles.iter() {
        let fired_by_player = missile.fired_by_player;
        let pos = transform.translation.truncate();
        let knockback_dir = missile_vel.0.normalize_or_zero();

        let hits: Vec<Entity> = spatial_query.shape_intersections(
            &Collider::circle(4.0),
            pos,
            0.0,
            &SpatialQueryFilter::from_mask(GameLayer::Dynamic),
        );

        for hit in hits {
            let Ok((
                mut vel,
                cooldown,
                mut stats_opt,
                mat_opt,
                existing_flash,
                is_player,
                is_monster,
            )) = dynamic_query.get_mut(hit)
            else {
                continue;
            };
            if cooldown.is_some_and(|c| c.0 > 0.0) {
                continue;
            }
            vel.0 += knockback_dir * KNOCKBACK_SPEED;
            commands.entity(hit).insert(KnockbackCooldown(KNOCKBACK_COOLDOWN));

            let Some(ref mut stats) = stats_opt else { continue };

            // Flash white, preserving the resting color across rapid re-hits.
            if let Some(mh) = mat_opt {
                let base_color = existing_flash
                    .map(|f| f.base_color)
                    .or_else(|| materials.get(&mh.0).map(|m| m.color));
                if let Some(base) = base_color {
                    if let Some(mat) = materials.get_mut(&mh.0) {
                        mat.color = Color::WHITE;
                    }
                    commands.entity(hit).insert(HitFlash {
                        timer: HIT_FLASH_DURATION,
                        duration: HIT_FLASH_DURATION,
                        base_color: base,
                    });
                }
            }

            let was_alive = stats.hp > 0.0;
            stats.hp -= MISSILE_DAMAGE;

            if is_monster {
                if fired_by_player {
                    commands.entity(hit).insert(AlertedByMissile);
                }
                message_log.push_repeating(
                    "The magic missile hits the monster",
                    hit,
                    if stats.hp <= 0.0 {
                        ", destroying it".to_string()
                    } else {
                        format!("; it now has {} hp", stats.hp)
                    },
                );
                if stats.hp <= 0.0 {
                    commands.entity(hit).despawn();
                }
            } else if is_player {
                if was_alive && stats.hp <= 0.0 {
                    message_log.push("Ouch!");
                }
                stats.hp = stats.hp.max(0.0);
            }
        }
    }
}

pub fn tick_knockback_cooldowns(time: Res<Time>, mut query: Query<&mut KnockbackCooldown>) {
    for mut cooldown in query.iter_mut() {
        cooldown.0 -= time.delta_secs();
    }
}

pub fn update_hit_flash(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut query: Query<(Entity, &mut HitFlash, &MeshMaterial2d<ColorMaterial>)>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    for (entity, mut flash, mat_handle) in query.iter_mut() {
        flash.timer -= time.delta_secs();
        if flash.timer <= 0.0 {
            if let Some(mat) = materials.get_mut(&mat_handle.0) {
                mat.color = flash.base_color;
            }
            commands.entity(entity).remove::<HitFlash>();
        } else {
            // t=1 at the moment of impact (white), t=0 when done (base_color)
            let t = flash.timer / flash.duration;
            let base = flash.base_color.to_srgba();
            let color = Color::srgba(
                base.red + t * (1.0 - base.red),
                base.green + t * (1.0 - base.green),
                base.blue + t * (1.0 - base.blue),
                base.alpha,
            );
            if let Some(mat) = materials.get_mut(&mat_handle.0) {
                mat.color = color;
            }
        }
    }
}

/// Returns the approximate bounce contact point if the missile's direction changed enough
/// between last_pos/last_dir and current_pos/current_dir to indicate a wall bounce.
/// Uses ray-ray intersection: the contact lies on both the forward ray from last_pos
/// and the backward ray from current_pos.
fn bounce_contact(
    last_pos: Vec2,
    last_dir: Vec2,
    current_pos: Vec2,
    current_dir: Vec2,
) -> Option<Vec2> {
    if last_dir.dot(current_dir) > 0.95 {
        return None; // directions nearly identical, no bounce
    }
    // Solve: last_pos + t*last_dir = current_pos - s*current_dir
    let diff = current_pos - last_pos;
    let det = last_dir.x * current_dir.y - last_dir.y * current_dir.x;
    if det.abs() < 1e-6 {
        return None;
    }
    let t = (diff.x * current_dir.y - diff.y * current_dir.x) / det;
    let s = (last_dir.x * diff.y - last_dir.y * diff.x) / det;
    if t < 0.0 || s < 0.0 {
        return None;
    }
    Some(last_pos + t * last_dir)
}

fn spawn_trail_segment(
    commands: &mut Commands,
    trail_meshes: &TrailMeshes,
    materials: &mut Assets<ColorMaterial>,
    missile_entity: Entity,
    fired_by_player: bool,
    from: Vec2,
    to: Vec2,
    extra_lifetime: f32,
) {
    let delta = to - from;
    let segment_len = delta.length();
    if segment_len < 0.5 {
        return;
    }
    let midpoint = from + delta * 0.5;
    let rotation = Quat::from_rotation_z(delta.y.atan2(delta.x));

    let (core_color, glow_color) = if fired_by_player {
        (Color::srgba(0.6, 0.8, 1.0, 1.0), Color::srgba(0.3, 0.5, 1.0, 0.4))
    } else {
        (Color::srgba(1.0, 0.6, 0.4, 1.0), Color::srgba(1.0, 0.3, 0.1, 0.4))
    };

    commands.spawn((
        DespawnOnExit(GameState::InLevel),
        MissileTrail {
            source_missile: missile_entity,
            fired_by_player,
            is_glow: false,
            extra_lifetime,
            expiration: None,
        },
        Mesh2d(trail_meshes.core.clone()),
        MeshMaterial2d(materials.add(ColorMaterial::from(core_color))),
        Transform::from_translation(midpoint.extend(MOVABLE_Z + 0.6))
            .with_rotation(rotation)
            .with_scale(Vec3::new(segment_len, 1.0, 1.0)),
    ));
    commands.spawn((
        DespawnOnExit(GameState::InLevel),
        MissileTrail {
            source_missile: missile_entity,
            fired_by_player,
            is_glow: true,
            extra_lifetime,
            expiration: None,
        },
        Mesh2d(trail_meshes.glow.clone()),
        MeshMaterial2d(materials.add(ColorMaterial::from(glow_color))),
        Transform::from_translation(midpoint.extend(MOVABLE_Z + 0.5))
            .with_rotation(rotation)
            .with_scale(Vec3::new(segment_len, 1.0, 1.0)),
    ));
}

pub fn spawn_missile_trails(
    mut query: Query<(Entity, &Transform, &LinearVelocity, &mut MagicMissile)>,
    trail_meshes: Res<TrailMeshes>,
    mut commands: Commands,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    for (missile_entity, transform, velocity, mut missile) in query.iter_mut() {
        let current_pos = transform.translation.truncate();
        let current_vel = velocity.0;

        let (last_pos, last_vel) = match (missile.last_trail_pos, missile.last_trail_vel) {
            (Some(p), Some(v)) => (p, v),
            _ => {
                missile.last_trail_pos = Some(current_pos);
                missile.last_trail_vel = Some(current_vel);
                continue;
            }
        };
        missile.last_trail_pos = Some(current_pos);
        missile.last_trail_vel = Some(current_vel);

        // Tail segments (spawned early) get up to 0.5 s extra life; head segments get 0.
        let t = (missile.distance_traveled / MISSILE_MAX_DISTANCE).clamp(0.0, 1.0);
        let extra_lifetime = (1.0 - t) * 0.5;

        let last_dir = last_vel.normalize_or_zero();
        let current_dir = current_vel.normalize_or_zero();

        if let Some(contact) = bounce_contact(last_pos, last_dir, current_pos, current_dir) {
            spawn_trail_segment(
                &mut commands,
                &trail_meshes,
                &mut materials,
                missile_entity,
                missile.fired_by_player,
                last_pos,
                contact,
                extra_lifetime,
            );
            spawn_trail_segment(
                &mut commands,
                &trail_meshes,
                &mut materials,
                missile_entity,
                missile.fired_by_player,
                contact,
                current_pos,
                extra_lifetime,
            );
        } else {
            spawn_trail_segment(
                &mut commands,
                &trail_meshes,
                &mut materials,
                missile_entity,
                missile.fired_by_player,
                last_pos,
                current_pos,
                extra_lifetime,
            );
        }
    }
}

pub fn update_missile_trails(
    mut commands: Commands,
    time: Res<Time>,
    missile_query: Query<Entity, With<MagicMissile>>,
    mut trail_query: Query<(Entity, &mut MissileTrail, &MeshMaterial2d<ColorMaterial>)>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let alive: HashSet<Entity> = missile_query.iter().collect();
    let now = time.elapsed_secs();

    for (entity, mut trail, mat_handle) in trail_query.iter_mut() {
        // When the source missile disappears, stamp the expiration time.
        if trail.expiration.is_none() && !alive.contains(&trail.source_missile) {
            trail.expiration = Some(now + trail.extra_lifetime);
        }

        let Some(expires_at) = trail.expiration else {
            continue; // missile still alive; trail stays fully opaque
        };

        let remaining = expires_at - now;
        if remaining <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }

        // Fade linearly from opaque to transparent over this segment's extra_lifetime.
        let fade = if trail.extra_lifetime > 0.0 { remaining / trail.extra_lifetime } else { 0.0 };
        let color = match (trail.fired_by_player, trail.is_glow) {
            (true, false) => Color::srgba(0.6, 0.8, 1.0, fade),
            (true, true) => Color::srgba(0.3, 0.5, 1.0, fade * 0.4),
            (false, false) => Color::srgba(1.0, 0.6, 0.4, fade),
            (false, true) => Color::srgba(1.0, 0.3, 0.1, fade * 0.4),
        };
        if let Some(mat) = materials.get_mut(&mat_handle.0) {
            mat.color = color;
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
    keyboard: Res<ButtonInput<KeyCode>>,
) {
    let any_missile = missile_query.iter().next().is_some();
    let spacebar_held = keyboard.pressed(KeyCode::Space);

    if any_missile {
        time.set_relative_speed(TIME_SCALE_MISSILE);
        // Physics would normally slow down for the time dialation, but the magic missles are so fast
        // that we need it to speed up in order for the bounces to work right.
        fixed_time.set_timestep(Duration::from_secs_f64(1.0 / (256.0 / TIME_SCALE_MISSILE as f64)));
    } else {
        fixed_time.set_timestep(Duration::from_secs_f64(1.0 / 64.0));
        if move_target.active || spacebar_held {
            time.set_relative_speed(1.0);
        } else {
            time.set_relative_speed(0.0);
        }
    }
}
