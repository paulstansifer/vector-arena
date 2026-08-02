// Monster marker component, AI state machine, and tooltip refresh.
// `MonsterState` drives behavior each frame: sleeping monsters ignore the player,
// wandering ones path to a random nearby location at half speed, seeking ones chase
// the player, tired ones rest briefly, and distracted ones path to a fixed point.
use rogue_angles::util::safegeo::SafeMultiPolygon;
use avian2d::prelude::LinearVelocity;
use bevy::prelude::*;
use bevy_landmass::prelude::*;
use geo::{Contains, Intersects};

use bevy_egui::egui;

use rogue_angles::{
    dungeon::terrain::{DungeonState, random_near},
    fov::CurrentFovState,
    hud::WorldTooltip,
    palette::{CommandPaletteState, EntityLabels, TargetDescription, Targetable},
};

use crate::{
    item::ItemKind,
    player::{ExplorationGoal, MoveTarget, Player},
    status_effect::{StatusEffects, blind_strength},
    visuals::indicator::StateIndicator,
};

pub const MONSTER_SPEED: f32 = 140.0;
pub const MONSTER_MAX_HP: f32 = 20.0;
pub const MONSTER_SEEK_RANGE: f32 = 300.0;
const MONSTER_WANDER_SPEED: f32 = MONSTER_SPEED * 0.5;
const MONSTER_WANDER_RANGE: f32 = 200.0;
pub const WANDER_ARRIVE_DIST: f32 = 100.0;
const FOCUS_DIST: f32 = 250.0;
/// Don't get tired if the player is this close.

#[derive(Component, Default, Clone, Copy)]
pub struct Stats {
    pub hp: f32,
    pub max_hp: f32,
    pub mana: f32,
    pub max_mana: f32,
}

#[derive(Component)]
pub struct Monster;

/// Inserted by the projectile system when a player-fired missile hits this monster.
/// The AI system reads and removes this each frame to trigger a Seeking transition.
#[derive(Component)]
pub struct AlertedByMissile;

/// Inserted when a monster fires a magic missile. Keeps the monster stationary until
/// the timer (virtual seconds) reaches zero.
#[derive(Component)]
pub struct MonsterShootFreeze(pub f32);

/// Item this monster will drop when killed. If absent, the monster drops nothing.
#[derive(Component)]
pub struct MonsterDrop(pub ItemKind);

#[derive(Component, Clone)]
pub enum MonsterState {
    /// Not moving; after timer expires may wander or sleep again.
    Sleeping { timer: f32 },
    /// Pathing to a random location ≤200 units away at half speed; falls asleep on arrival.
    Wandering { target: Vec2 },
    /// Chasing the player; transitions to Tired after its timer runs out.
    Seeking { timer: f32 },
    /// Pathing to a fixed location (future use).
    Distracted { target: Vec2 },
    /// Resting after seeking; then randomly wanders or sleeps.
    Tired { timer: f32 },
}

impl MonsterState {
    pub fn label(&self) -> &'static str {
        match self {
            MonsterState::Sleeping { .. } => "sleeping",
            MonsterState::Wandering { .. } => "wandering",
            MonsterState::Seeking { .. } => "seeking",
            MonsterState::Distracted { .. } => "distracted",
            MonsterState::Tired { .. } => "tired",
        }
    }
}

/// Advance `state` by one frame, updating the agent target and settings to match.
///
/// `should_seek` (LOS + range, or missile alert) overrides any non-seeking state.
/// When no transition occurs the state is mutated in place (timer decremented),
/// so there is no need for an Option return — the "no change" path simply falls through.
fn tick_state(
    state: &mut MonsterState,
    target: &mut AgentTarget2d,
    settings: &mut AgentSettings,
    monster_pos: Vec2,
    player_entity: Entity,
    dist_to_player: f32,
    should_seek: bool,
    blind_strength: f32,
    playable_area: &SafeMultiPolygon,
    dt: f32,
    rng: &mut impl rand::Rng,
) {
    // Blind monsters immediately exit seek mode when the effect is active.
    if blind_strength > 0.0 && matches!(state, MonsterState::Seeking { .. }) {
        *state = MonsterState::Tired { timer: rng.gen_range(0.0..0.5) };
    }

    if should_seek && !matches!(state, MonsterState::Seeking { .. }) {
        *state = MonsterState::Seeking { timer: rng.gen_range(2.0..3.0) };
    }

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
            if *timer <= 0.0 && dist_to_player > FOCUS_DIST {
                *state = MonsterState::Tired { timer: rng.gen_range(2.0..3.0) };
            }
        }
        MonsterState::Distracted { target: dist_pos } => {
            let dp = *dist_pos;
            *target = AgentTarget2d::Point(dp);
            settings.desired_speed = MONSTER_SPEED;
            settings.max_speed = MONSTER_SPEED * 1.2;
            if monster_pos.distance(dp) < WANDER_ARRIVE_DIST {
                *state = if rng.gen_bool(0.5) {
                    wander_somewhere(monster_pos, playable_area, rng)
                } else {
                    MonsterState::Sleeping { timer: rng.gen_range(2.0..5.0) }
                };
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
}

pub fn refresh_monster_tooltips(
    mut query: Query<
        (
            Entity,
            &Stats,
            &MonsterState,
            &mut WorldTooltip,
            &mut TargetDescription,
            Option<&StatusEffects>,
        ),
        With<Monster>,
    >,
    labels: Res<EntityLabels>,
) {
    for (entity, stats, state, mut tooltip, mut target_desc, effects) in query.iter_mut() {
        let letter_prefix =
            labels.letter_for(entity).map(|l| format!("[{l}] ")).unwrap_or_default();
        let mut text = format!(
            "{}HP: {}/{} [{}]",
            letter_prefix,
            stats.hp as i32,
            stats.max_hp as i32,
            state.label()
        );
        if let Some(effects) = effects {
            for e in &effects.0 {
                let secs = e.remaining.ceil() as u32;
                text.push_str(&format!(" | {} {}s", e.kind.label(), secs));
            }
        }
        tooltip.0 = text;
        // Also shown next to the monster's letter in the palette's target picker.
        target_desc.0 = format!("{} {}/{} HP", state.label(), stats.hp as i32, stats.max_hp as i32);
    }
}

pub fn update_monster_ai(
    time: Res<Time>,
    player_query: Single<
        (Entity, &Transform, &mut MoveTarget, &mut AgentTarget2d, &mut LinearVelocity),
        (With<Player>, Without<Monster>),
    >,
    mut monster_query: Query<
        (
            Entity,
            &Transform,
            &mut MonsterState,
            &mut AgentTarget2d,
            &mut AgentSettings,
            Option<&AlertedByMissile>,
            Option<&StatusEffects>,
            Option<&mut MonsterShootFreeze>,
            &mut LinearVelocity,
        ),
        With<Monster>,
    >,
    dungeon_state: Res<DungeonState>,
    mut commands: Commands,
) {
    let (
        player_entity,
        player_transform,
        mut player_move_target,
        mut player_agent_target,
        mut player_velocity,
    ) = player_query.into_inner();
    let player_pos = player_transform.translation.truncate();
    let solid_rock = &dungeon_state.solid_rock;
    let dt = time.delta_secs();

    for (
        entity,
        transform,
        mut state,
        mut target,
        mut settings,
        alerted,
        effects,
        maybe_freeze,
        mut vel,
    ) in monster_query.iter_mut()
    {
        let monster_pos = transform.translation.truncate();

        let missile_alerted = alerted.is_some();
        if missile_alerted {
            commands.entity(entity).remove::<AlertedByMissile>();
        }

        let blind_strength = effects.map(blind_strength).unwrap_or(0.0);

        // Blind monsters have a reduced seek range (scaled toward 0).
        let effective_seek_range = MONSTER_SEEK_RANGE * (1.0 - blind_strength);
        let dist_to_player = monster_pos.distance(player_pos);
        let has_los = dist_to_player < effective_seek_range && {
            let seg =
                geo::Line::new(geo::Coord { x: monster_pos.x, y: monster_pos.y }, geo::Coord {
                    x: player_pos.x,
                    y: player_pos.y,
                });
            !solid_rock.intersects(&seg)
        };

        let old_label = state.label();
        tick_state(
            &mut state,
            &mut target,
            &mut settings,
            monster_pos,
            player_entity,
            dist_to_player,
            missile_alerted || has_los,
            blind_strength,
            &dungeon_state.playable_area,
            dt,
            &mut rand::thread_rng(),
        );
        // When a monster newly enters Seeking (the chase state, not Distracted), interrupt
        // the player's current path; they may want to change plans!
        if old_label != "seeking" && matches!(&*state, MonsterState::Seeking { .. }) {
            player_move_target.active = false;
            *player_agent_target = AgentTarget2d::None;
            player_velocity.0 = Vec2::ZERO;
            commands.entity(player_entity).remove::<ExplorationGoal>();
        }

        if state.label() != old_label {
            let ch = match &*state {
                MonsterState::Seeking { .. } | MonsterState::Distracted { .. } => '!',
                MonsterState::Sleeping { .. } => 's',
                MonsterState::Tired { .. } => 't',
                MonsterState::Wandering { .. } => 'w',
            };
            commands.entity(entity).insert(StateIndicator::new(ch, 1.5));
        }

        if let Some(mut freeze) = maybe_freeze {
            freeze.0 -= dt;
            settings.desired_speed = 0.0;
            vel.0 = Vec2::ZERO;
            if freeze.0 <= 0.0 {
                commands.entity(entity).remove::<MonsterShootFreeze>();
            }
        }
    }
}

/// Pick a navigable wandering destination (or sleep if rejection sampling fails)
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

pub fn render_monster_markers(
    palette: Res<CommandPaletteState>,
    labels: Res<EntityLabels>,
    monster_query: Query<(Entity, &Transform), (With<Monster>, With<Targetable>)>,
    mut egui_context: bevy_egui::EguiContexts,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    current_fov: Option<Res<CurrentFovState>>,
) {
    if !palette.open {
        return;
    }

    let Ok((camera, camera_transform)) = camera_query.single() else { return };
    let Ok(ctx) = egui_context.ctx_mut() else { return };

    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("monster_markers"),
    ));

    for (entity, transform) in monster_query.iter() {
        let Some(letter) = labels.letter_for(entity) else { continue };
        let pos = transform.translation.truncate();
        if let Some(ref fov) = current_fov
            && !fov.0.contains(&geo::Point::new(pos.x, pos.y))
        {
            continue;
        }
        let Ok(viewport_pos) = camera.world_to_viewport(camera_transform, transform.translation)
        else {
            continue;
        };
        if viewport_pos.x < 0.0 || viewport_pos.y < 0.0 {
            continue;
        }
        let screen_pos = egui::Pos2::new(viewport_pos.x, viewport_pos.y);
        painter.circle_filled(
            screen_pos,
            10.0,
            egui::Color32::from_rgba_unmultiplied(220, 60, 60, 150),
        );
        painter.text(
            screen_pos,
            egui::Align2::CENTER_CENTER,
            letter.to_string(),
            egui::FontId::monospace(12.0),
            egui::Color32::WHITE,
        );
    }
}
