// Monster marker component and AI state machine. Unlike vector-arena's monsters, these
// never take damage or die — they're a permanent hazard to dodge, not a target to kill.
// Sleeping monsters ignore the player; wandering ones path to a random nearby location;
// seeking ones chase within line-of-sight; a lunge is a telegraphed (LungeWindup), fast,
// committed dash (Lunging) that deals contact damage once, then a cooldown (Tired) before
// the monster can lunge again.
use bevy::prelude::*;
use bevy_landmass::prelude::*;
use geo::Intersects;

use rogue_angles::{
    AGENT_RADIUS,
    dungeon::terrain::{DungeonState, random_near},
    hud::WorldTooltip,
    util::safegeo::SafeMultiPolygon,
    visuals::indicator::HitFlash,
};

use crate::player::{Player, Stats, deal_damage_to_player};

pub const MONSTER_SPEED: f32 = 120.0;
const MONSTER_WANDER_SPEED: f32 = MONSTER_SPEED * 0.5;
const MONSTER_WANDER_RANGE: f32 = 200.0;
const WANDER_ARRIVE_DIST: f32 = 40.0;
pub const MONSTER_SEEK_RANGE: f32 = 260.0;

/// Distance at which a seeking monster commits to a lunge.
const LUNGE_RANGE: f32 = 75.0;
/// Telegraph: the monster freezes before dashing, so a lunge is dodgeable.
const WINDUP_SECS: f32 = 0.3;
const LUNGE_SPEED: f32 = 480.0;
const LUNGE_DISTANCE: f32 = 130.0;
const LUNGE_DURATION: f32 = 0.45;
/// Distance at which a lunging monster's contact registers as a hit.
const CONTACT_RADIUS: f32 = AGENT_RADIUS * 2.0 + 6.0;
pub const LUNGE_DAMAGE: f32 = 12.0;
const HIT_FLASH_DURATION: f32 = 0.2;

#[derive(Component)]
pub struct Monster;

#[derive(Component, Clone)]
pub enum MonsterState {
    Sleeping { timer: f32 },
    Wandering { target: Vec2 },
    Seeking { timer: f32 },
    /// Frozen, telegraphing a lunge toward `aim_point` (the player's position when the
    /// windup started — not updated once committed, so a lunge is whiffable).
    LungeWindup { timer: f32, aim_point: Vec2 },
    /// Dashing toward a fixed `target` point at `LUNGE_SPEED`. `hit_applied` prevents a
    /// single lunge from damaging the player more than once.
    Lunging { timer: f32, target: Vec2, hit_applied: bool },
    Tired { timer: f32 },
}

impl MonsterState {
    pub fn label(&self) -> &'static str {
        match self {
            MonsterState::Sleeping { .. } => "sleeping",
            MonsterState::Wandering { .. } => "wandering",
            MonsterState::Seeking { .. } => "seeking",
            MonsterState::LungeWindup { .. } => "winding up",
            MonsterState::Lunging { .. } => "lunging",
            MonsterState::Tired { .. } => "tired",
        }
    }
}

/// Advance `state` by one frame. Returns `Some(damage_direction)` the one frame a lunge
/// lands a hit, so the caller can apply damage/knockback without this function touching
/// `Stats` directly (keeping it a pure, easily unit-testable state machine).
#[allow(clippy::too_many_arguments)]
fn tick_state(
    state: &mut MonsterState,
    target: &mut AgentTarget2d,
    settings: &mut AgentSettings,
    monster_pos: Vec2,
    player_entity: Entity,
    player_pos: Vec2,
    dist_to_player: f32,
    has_los: bool,
    playable_area: &SafeMultiPolygon,
    dt: f32,
    rng: &mut impl rand::Rng,
) -> bool {
    if has_los
        && dist_to_player < LUNGE_RANGE
        && matches!(state, MonsterState::Seeking { .. })
    {
        *state = MonsterState::LungeWindup { timer: WINDUP_SECS, aim_point: player_pos };
    } else if has_los && !matches!(state, MonsterState::Seeking { .. } | MonsterState::LungeWindup { .. } | MonsterState::Lunging { .. }) {
        *state = MonsterState::Seeking { timer: rng.gen_range(2.0..3.0) };
    }

    let mut hit_landed = false;

    match state {
        MonsterState::Sleeping { timer } => {
            *target = AgentTarget2d::None;
            settings.desired_speed = 0.0;
            settings.max_speed = MONSTER_SPEED * 1.2;
            *timer -= dt;
            if *timer <= 0.0 {
                *state = wander_somewhere(monster_pos, playable_area, rng);
            }
        }
        MonsterState::Wandering { target: wander_pos } => {
            let wt = *wander_pos;
            *target = AgentTarget2d::Point(wt);
            settings.desired_speed = MONSTER_WANDER_SPEED;
            settings.max_speed = MONSTER_WANDER_SPEED * 1.2;
            if monster_pos.distance(wt) < WANDER_ARRIVE_DIST {
                *state = MonsterState::Sleeping { timer: rng.gen_range(2.0..5.0) };
            }
        }
        MonsterState::Seeking { timer } => {
            *target = AgentTarget2d::Entity(player_entity);
            settings.desired_speed = MONSTER_SPEED;
            settings.max_speed = MONSTER_SPEED * 1.2;
            *timer -= dt;
            if *timer <= 0.0 && !has_los {
                *state = MonsterState::Tired { timer: rng.gen_range(1.0..2.0) };
            }
        }
        MonsterState::LungeWindup { timer, aim_point } => {
            *target = AgentTarget2d::None;
            settings.desired_speed = 0.0;
            *timer -= dt;
            if *timer <= 0.0 {
                let dir = (*aim_point - monster_pos).normalize_or(Vec2::Y);
                let lunge_target = monster_pos + dir * LUNGE_DISTANCE;
                *state =
                    MonsterState::Lunging { timer: LUNGE_DURATION, target: lunge_target, hit_applied: false };
            }
        }
        MonsterState::Lunging { timer, target: lunge_target, hit_applied } => {
            *target = AgentTarget2d::Point(*lunge_target);
            settings.desired_speed = LUNGE_SPEED;
            settings.max_speed = LUNGE_SPEED * 1.2;
            if !*hit_applied && dist_to_player < CONTACT_RADIUS {
                *hit_applied = true;
                hit_landed = true;
            }
            *timer -= dt;
            if *timer <= 0.0 {
                *state = MonsterState::Tired { timer: rng.gen_range(1.0..2.0) };
            }
        }
        MonsterState::Tired { timer } => {
            *target = AgentTarget2d::None;
            settings.desired_speed = 0.0;
            settings.max_speed = MONSTER_SPEED * 1.2;
            *timer -= dt;
            if *timer <= 0.0 {
                *state = if rng.gen_bool(0.5) {
                    wander_somewhere(monster_pos, playable_area, rng)
                } else {
                    MonsterState::Sleeping { timer: rng.gen_range(2.0..5.0) }
                };
            }
        }
    }

    hit_landed
}

fn wander_somewhere(
    origin: Vec2,
    playable_area: &SafeMultiPolygon,
    rng: &mut impl rand::Rng,
) -> MonsterState {
    match random_near(playable_area, origin, 50.0, MONSTER_WANDER_RANGE, rng) {
        Some(target) => MonsterState::Wandering { target },
        None => MonsterState::Sleeping { timer: rng.gen_range(2.0..5.0) },
    }
}

pub fn refresh_monster_tooltips(mut query: Query<(&MonsterState, &mut WorldTooltip), With<Monster>>) {
    for (state, mut tooltip) in query.iter_mut() {
        tooltip.0 = format!("Monster [{}]", state.label());
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update_monster_ai(
    time: Res<Time>,
    player_query: Single<
        (Entity, &Transform, &mut Stats, Option<&MeshMaterial2d<ColorMaterial>>),
        With<Player>,
    >,
    mut monster_query: Query<
        (Entity, &Transform, &mut MonsterState, &mut AgentTarget2d, &mut AgentSettings),
        With<Monster>,
    >,
    dungeon_state: Res<DungeonState>,
    mut commands: Commands,
    materials: Res<Assets<ColorMaterial>>,
) {
    let (player_entity, player_transform, mut player_stats, player_mat) = player_query.into_inner();
    let player_pos = player_transform.translation.truncate();
    let solid_rock = &dungeon_state.solid_rock;
    let dt = time.delta_secs();

    for (_entity, transform, mut state, mut target, mut settings) in monster_query.iter_mut() {
        let monster_pos = transform.translation.truncate();
        let dist_to_player = monster_pos.distance(player_pos);
        let has_los = dist_to_player < MONSTER_SEEK_RANGE && {
            let seg =
                geo::Line::new(geo::Coord { x: monster_pos.x, y: monster_pos.y }, geo::Coord {
                    x: player_pos.x,
                    y: player_pos.y,
                });
            !solid_rock.intersects(&seg)
        };

        let hit_landed = tick_state(
            &mut state,
            &mut target,
            &mut settings,
            monster_pos,
            player_entity,
            player_pos,
            dist_to_player,
            has_los,
            &dungeon_state.playable_area,
            dt,
            &mut rand::thread_rng(),
        );

        if hit_landed {
            deal_damage_to_player(&mut player_stats, LUNGE_DAMAGE);
            if let Some(mh) = player_mat
                && let Some(mat) = materials.get(&mh.0)
            {
                commands.entity(player_entity).try_insert(HitFlash {
                    timer: HIT_FLASH_DURATION,
                    duration: HIT_FLASH_DURATION,
                    base_color: mat.color,
                });
            }
        }
    }
}
