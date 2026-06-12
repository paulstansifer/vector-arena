// Magic missiles, trails, and knockback.
// Missiles use the Missile physics layer (collides with Wall only); knockback
// against Dynamic bodies is applied manually by querying overlaps rather than
// via collision events.  Global time dilation while missiles are in flight is
// handled separately in `crate::time_scale`.
use avian2d::prelude::*;
use bevy::prelude::*;
use rand::Rng;
use std::collections::HashSet;

use crate::{
    AGENT_RADIUS, GameLayer, GameState,
    command_palette::{CommandPaletteState, PaletteCommand, PaletteCommandKind, PaletteRegistry},
    dungeon::terrain::TorporMultiplier,
    effects::crumble_terrain::Rubble,
    fov,
    indicator::HitFlash,
    item::{Item, ItemKind, item_name},
    monster::{AlertedByMissile, Monster, MonsterDrop, Stats},
    player::Player,
    sprite::{SpriteParam, SvgSprite, potion_hex, scroll_letter},
    status_effect::StatusEffects,
    ui::{MessageLog, WorldTooltip},
};

pub const MISSILE_KEY: &str = "z";

pub const MISSILE_SPEED: f32 = 3500.0;
pub const MISSILE_MAX_DISTANCE: f32 = 1000.0;
pub const MONSTER_FIRE_RANGE: f32 = 100.0;
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

/// Shared missile mesh/materials, built once at startup. Every missile is an identical 4px
/// circle in one of two fixed colors, so there's no reason to allocate fresh handles per shot.
#[derive(Resource)]
pub struct MissileAssets {
    mesh: Handle<Mesh>,
    player_material: Handle<ColorMaterial>,
    monster_material: Handle<ColorMaterial>,
}

pub fn init_trail_meshes(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    commands.insert_resource(TrailMeshes {
        core: meshes.add(Rectangle::new(1.0, TRAIL_CORE_HEIGHT)),
        glow: meshes.add(Rectangle::new(1.0, TRAIL_GLOW_HEIGHT)),
    });
    commands.insert_resource(MissileAssets {
        mesh: meshes.add(Circle::new(4.0)),
        player_material: materials.add(ColorMaterial::from(Color::srgb(0.4, 0.6, 1.0))),
        monster_material: materials.add(ColorMaterial::from(Color::srgb(1.0, 0.4, 0.2))),
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
    pub damage_multiplier: f32,
    last_trail_pos: Option<Vec2>,
    last_trail_vel: Option<Vec2>,
}

#[derive(Component)]
pub struct KnockbackCooldown(f32);

#[derive(Component)]
pub struct MonsterShootTimer(pub f32);

impl Default for MonsterShootTimer {
    fn default() -> Self { Self::new() }
}

impl MonsterShootTimer {
    pub fn new() -> Self {
        let mut rng = rand::thread_rng();
        Self(rng.gen_range(1.0..2.0))
    }
}

impl MagicMissile {
    pub fn new(
        fired_by_player: bool,
        initial_pos: Vec2,
        initial_vel: Vec2,
        damage_multiplier: f32,
    ) -> Self {
        Self {
            distance_traveled: 0.0,
            fired_by_player,
            damage_multiplier,
            last_trail_pos: Some(initial_pos),
            last_trail_vel: Some(initial_vel),
        }
    }
}

fn spawn_missile(
    commands: &mut Commands,
    missile_assets: &MissileAssets,
    origin: Vec2,
    direction: Vec2,
    fired_by_player: bool,
    damage_multiplier: f32,
) {
    let spawn_pos = origin + direction * (AGENT_RADIUS + 6.0);
    let material = if fired_by_player {
        missile_assets.player_material.clone()
    } else {
        missile_assets.monster_material.clone()
    };

    commands.spawn((
        DespawnOnExit(GameState::InLevel),
        MagicMissile::new(fired_by_player, spawn_pos, direction * MISSILE_SPEED, damage_multiplier),
        TorporMultiplier(1.0),
        Mesh2d(missile_assets.mesh.clone()),
        MeshMaterial2d(material),
        Transform::from_translation(spawn_pos.extend(fov::MOVABLE_Z + 1.0)),
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

pub fn register_missile_command(mut registry: ResMut<PaletteRegistry>) {
    registry.commands.push(PaletteCommand {
        key: MISSILE_KEY.to_string(),
        description: "Zap magic missile".to_string(),
        icon: None,
        kind: PaletteCommandKind::LocationTarget { target_verb: "Zap at" },
    });
}

/// Fires a player missile toward the resolved target stored in palette.pending_target.
pub fn execute_missile_command(
    mut palette: ResMut<CommandPaletteState>,
    player_query: Query<(&Transform, Option<&StatusEffects>), With<Player>>,
    mut commands: Commands,
    missile_assets: Res<MissileAssets>,
) {
    if palette.pending_command.as_deref() != Some(MISSILE_KEY) {
        return;
    }
    palette.pending_command = None;
    let Some(target_pos) = palette.pending_target.take() else { return };
    let Ok((player_tf, player_effects)) = player_query.single() else { return };
    let player_pos = player_tf.translation.truncate();
    let direction = (target_pos - player_pos).normalize_or_zero();
    if direction == Vec2::ZERO {
        return;
    }
    let damage_multiplier = player_effects.map(|e| e.missile_multiplier()).unwrap_or(1.0);
    spawn_missile(&mut commands, &missile_assets, player_pos, direction, true, damage_multiplier);
}

pub fn monster_fire_missiles(
    time: Res<Time>,
    player_query: Single<&Transform, With<Player>>,
    mut monster_query: Query<
        (&Transform, &mut MonsterShootTimer, Option<&StatusEffects>),
        With<Monster>,
    >,
    mut commands: Commands,
    missile_assets: Res<MissileAssets>,
) {
    let player_pos = player_query.translation.truncate();

    for (transform, mut timer, effects) in monster_query.iter_mut() {
        timer.0 -= time.delta_secs();
        if timer.0 > 0.0 {
            continue;
        }

        *timer = MonsterShootTimer::new();

        let monster_pos = transform.translation.truncate();
        if monster_pos.distance(player_pos) > MONSTER_FIRE_RANGE {
            continue;
        }

        let direction = (player_pos - monster_pos).normalize_or_zero();
        let damage_multiplier = effects.map(|e| e.missile_multiplier()).unwrap_or(1.0);
        spawn_missile(
            &mut commands,
            &missile_assets,
            monster_pos,
            direction,
            false,
            damage_multiplier,
        );
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

// TODO: this doesn't belong here (except that missiles are currently the main thing affected)
pub fn apply_torpor_to_non_agents(
    mut query: Query<(&TorporMultiplier, &mut LinearVelocity), Without<bevy_landmass::Agent2d>>,
) {
    for (torpor, mut vel) in query.iter_mut() {
        let dir = vel.0.normalize_or_zero();
        if dir != Vec2::ZERO {
            vel.0 = dir * MISSILE_SPEED * torpor.get();
        }
    }
}

#[derive(Event)]
pub struct MissileHitEvent {
    pub hit_entity: Entity,
    pub direction: Vec2,
    pub fired_by_player: bool,
    pub damage_multiplier: f32,
}

/// Fired when a missile is deflected by the Displacing status effect.
/// The entity receives perpendicular knockback but no damage.
#[derive(Event)]
pub struct MissileDodgedEvent {
    pub hit_entity: Entity,
    pub direction: Vec2,
}

/// For each missile, sweep the collision shape along the path the missile travelled this frame
/// (including bounces) and trigger a `MissileHitEvent` for each Dynamic entity it passed through.
/// Must run before `spawn_missile_trails` so both systems read the same
/// `last_trail_pos`/`last_trail_vel` before the trail system advances them.
///
/// A short per-entity cooldown prevents re-hits from simultaneous missiles while still
/// allowing a bounced missile to hit on a later pass.
pub fn detect_missile_hits(
    mut commands: Commands,
    spatial_query: SpatialQuery,
    missiles: Query<(&Transform, &LinearVelocity, &MagicMissile)>,
    cooldown_query: Query<&KnockbackCooldown>,
    status_query: Query<&StatusEffects>,
) {
    let mut rng = rand::thread_rng();
    for (transform, missile_vel, missile) in missiles.iter() {
        let fired_by_player = missile.fired_by_player;
        let damage_multiplier = missile.damage_multiplier;
        let current_pos = transform.translation.truncate();
        let current_vel = missile_vel.0;
        let current_dir = current_vel.normalize_or_zero();

        // Build the swept segments for this frame using the same logic as spawn_missile_trails.
        // Each entry is (segment_origin, unit_direction, length).
        let segments: Vec<(Vec2, Vec2, f32)> = if let (Some(last_pos), Some(last_vel)) =
            (missile.last_trail_pos, missile.last_trail_vel)
        {
            let last_dir = last_vel.normalize_or_zero();
            if let Some(contact) = bounce_contact(last_pos, last_dir, current_pos, current_dir) {
                vec![
                    (last_pos, last_dir, (contact - last_pos).length()),
                    (contact, current_dir, (current_pos - contact).length()),
                ]
            } else {
                vec![(last_pos, current_dir, (current_pos - last_pos).length())]
            }
        } else {
            // No previous position recorded yet; fall back to a point check.
            vec![(current_pos, current_dir, 0.0)]
        };

        for (seg_origin, seg_dir, seg_len) in segments {
            let hits: Vec<Entity> = if seg_len < 0.5 {
                // Zero/near-zero movement: overlap check at the origin point.
                spatial_query.shape_intersections(
                    &Collider::circle(4.0),
                    seg_origin,
                    0.0,
                    &SpatialQueryFilter::from_mask(GameLayer::Dynamic),
                )
            } else {
                let Ok(dir2) = Dir2::new(seg_dir) else { continue };
                spatial_query
                    .shape_hits(
                        &Collider::circle(4.0),
                        seg_origin,
                        0.0,
                        dir2,
                        20,
                        &ShapeCastConfig::from_max_distance(seg_len),
                        &SpatialQueryFilter::from_mask(GameLayer::Dynamic),
                    )
                    .into_iter()
                    .map(|h| h.entity)
                    .collect()
            };

            for hit in hits {
                if cooldown_query.get(hit).is_ok_and(|c| c.0 > 0.0) {
                    continue;
                }
                let disp_strength =
                    status_query.get(hit).map(|e| e.displacing_strength()).unwrap_or(0.0);
                if disp_strength > 0.0 && rng.gen_range(0.0_f32..1.0) < disp_strength {
                    // Displaced away from missile: apply perpendicular knockback, no damage.
                    let perp = Vec2::new(-seg_dir.y, seg_dir.x);
                    let sign = if rng.gen_range(0.0_f32..1.0) > 0.5 { 1.0 } else { -1.0 };
                    commands
                        .trigger(MissileDodgedEvent { hit_entity: hit, direction: perp * sign });
                } else {
                    commands.trigger(MissileHitEvent {
                        hit_entity: hit,
                        direction: seg_dir,
                        fired_by_player,
                        damage_multiplier,
                    });
                }
            }
        }
    }
}

/// Shared by the hit and dodge observers: shove the entity along `direction` and start its
/// re-hit cooldown.
fn apply_knockback(
    commands: &mut Commands,
    query: &mut Query<&mut LinearVelocity, (Without<MagicMissile>, With<RigidBody>)>,
    hit_entity: Entity,
    direction: Vec2,
) {
    let Ok(mut vel) = query.get_mut(hit_entity) else { return };
    vel.0 += direction * KNOCKBACK_SPEED;
    commands.entity(hit_entity).try_insert(KnockbackCooldown(KNOCKBACK_COOLDOWN));
}

pub fn apply_knockback_on_hit(
    trigger: On<MissileHitEvent>,
    mut commands: Commands,
    mut query: Query<&mut LinearVelocity, (Without<MagicMissile>, With<RigidBody>)>,
) {
    let event = trigger.event();
    apply_knockback(&mut commands, &mut query, event.hit_entity, event.direction);
}

pub fn apply_dodge(
    trigger: On<MissileDodgedEvent>,
    mut commands: Commands,
    mut query: Query<&mut LinearVelocity, (Without<MagicMissile>, With<RigidBody>)>,
) {
    let event = trigger.event();
    apply_knockback(&mut commands, &mut query, event.hit_entity, event.direction);
}

pub fn apply_hit_flash_on_hit(
    trigger: On<MissileHitEvent>,
    mut commands: Commands,
    query: Query<
        (Option<&MeshMaterial2d<ColorMaterial>>, Option<&HitFlash>),
        // (a) it would be weird for rubble to hit-flash
        // (b) all rubble shares the same texture, and it gets stuck!
        (Without<MagicMissile>, With<RigidBody>, Without<Rubble>),
    >,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    let event = trigger.event();
    let Ok((mat_opt, existing_flash)) = query.get(event.hit_entity) else { return };
    let Some(mh) = mat_opt else { return };
    // Preserve the resting color across rapid re-hits.
    let base_color =
        existing_flash.map(|f| f.base_color).or_else(|| materials.get(&mh.0).map(|m| m.color));
    if let Some(base) = base_color {
        if let Some(mat) = materials.get_mut(&mh.0) {
            mat.color = Color::WHITE;
        }
        commands.entity(event.hit_entity).try_insert(HitFlash {
            timer: HIT_FLASH_DURATION,
            duration: HIT_FLASH_DURATION,
            base_color: base,
        });
    }
}

pub fn apply_damage_on_hit(
    trigger: On<MissileHitEvent>,
    mut commands: Commands,
    mut query: Query<
        (Option<&mut Stats>, Has<Player>, Has<Monster>, &Transform, Option<&MonsterDrop>),
        (Without<MagicMissile>, With<RigidBody>),
    >,
    mut message_log: ResMut<MessageLog>,
    mut monster_letters: ResMut<crate::command_palette::LetterMap>,
) {
    let event = trigger.event();
    let Ok((mut stats_opt, is_player, is_monster, transform, drop_opt)) =
        query.get_mut(event.hit_entity)
    else {
        return;
    };
    let Some(ref mut stats) = stats_opt else { return };

    let was_alive = stats.hp > 0.0;
    stats.hp -= MISSILE_DAMAGE * event.damage_multiplier;

    if is_monster {
        if event.fired_by_player {
            commands.entity(event.hit_entity).insert(AlertedByMissile);
        }
        message_log.push_repeating(
            "The magic missile hits the monster",
            event.hit_entity,
            if stats.hp <= 0.0 {
                ", destroying it".to_string()
            } else {
                format!("; it now has {} hp", stats.hp)
            },
        );
        if stats.hp <= 0.0 {
            if let Some(drop) = drop_opt {
                let kind = drop.0;
                let pos = transform.translation.truncate().extend(fov::ON_FLOOR_Z);
                let (svg_path, param) = match kind {
                    ItemKind::Potion(color) => {
                        ("sprites/potion.svg".to_string(), SpriteParam::Color(potion_hex(color)))
                    }
                    ItemKind::Scroll(name) => (
                        "sprites/scroll.svg".to_string(),
                        SpriteParam::Text(scroll_letter(name).to_string()),
                    ),
                };
                commands.spawn((
                    DespawnOnExit(GameState::InLevel),
                    Item(kind),
                    WorldTooltip(item_name(kind, 1).to_string()),
                    SvgSprite { svg_path, param: Some(param) },
                    Transform::from_translation(pos).with_scale(Vec3::splat(0.4)),
                ));
            }
            monster_letters.release_monster(event.hit_entity);
            commands.entity(event.hit_entity).despawn();
        }
    } else if is_player {
        if was_alive && stats.hp <= 0.0 {
            message_log.push("Ouch!");
        }
        stats.hp = stats.hp.max(0.0);
    }
}

pub fn tick_knockback_cooldowns(time: Res<Time>, mut query: Query<&mut KnockbackCooldown>) {
    for mut cooldown in query.iter_mut() {
        cooldown.0 -= time.delta_secs();
    }
}

fn trail_color(fired_by_player: bool, is_glow: bool, alpha: f32) -> Color {
    match (fired_by_player, is_glow) {
        (true, false) => Color::srgba(0.6, 0.8, 1.0, alpha),
        (true, true) => Color::srgba(0.3, 0.5, 1.0, alpha * 0.4),
        (false, false) => Color::srgba(1.0, 0.6, 0.4, alpha),
        (false, true) => Color::srgba(1.0, 0.3, 0.1, alpha * 0.4),
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

    let core_color = trail_color(fired_by_player, false, 1.0);
    let glow_color = trail_color(fired_by_player, true, 1.0);

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
        Transform::from_translation(midpoint.extend(fov::MOVABLE_Z + 0.6))
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
        Transform::from_translation(midpoint.extend(fov::MOVABLE_Z + 0.5))
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
        let color = trail_color(trail.fired_by_player, trail.is_glow, fade);
        if let Some(mat) = materials.get_mut(&mat_handle.0) {
            mat.color = color;
        }
    }
}
