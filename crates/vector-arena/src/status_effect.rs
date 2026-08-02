// This game's status-effect vocabulary, built on the engine's generic
// `rogue_angles::status_effects` framework. The engine owns timing/ramping and the tick
// system (registered via `add_status_effects::<StatusEffect>()` in `game.rs`); this file only
// supplies the concrete effect kinds and the aggregates that are this game's business, not the
// engine's (missile damage, confusion wander direction, blindness, displacement).
use avian2d::prelude::*;
use bevy::prelude::*;
use rand::{Rng, RngCore};

use rogue_angles::{
    AGENT_RADIUS, GameLayer, dungeon::terrain::SlowZoneMultiplier, movement::MovementModifiers,
    status_effects::StatusKind,
};

pub type StatusEffects = rogue_angles::status_effects::StatusEffects<StatusEffect>;

#[derive(Clone, Copy, Debug)]
pub enum StatusEffect {
    /// Blends movement direction toward a slowly-turning random walk direction.
    Confused { wander_dir: Vec2 },
    /// Shrinks FOV radius (player) or seek range (monster).
    Blind,
    /// Scales movement speed. `param` is the full-strength multiplier: 0.5 = slowed, 2.0 = hasted.
    SpeedMod(f32),
    /// Scales magic missile damage. `param`: 0.5 = drained, 2.0 = boosted.
    MissileMod(f32),
    /// Chance to deflect incoming missiles with perpendicular knockback.
    Displacing,
}

impl StatusEffect {
    pub fn label(&self) -> &'static str {
        match self {
            StatusEffect::Confused { .. } => "confused",
            StatusEffect::Blind => "blind",
            StatusEffect::SpeedMod(p) if *p > 1.0 => "hasted",
            StatusEffect::SpeedMod(_) => "slowed",
            StatusEffect::MissileMod(p) if *p > 1.0 => "boosted",
            StatusEffect::MissileMod(_) => "drained",
            StatusEffect::Displacing => "displacing",
        }
    }

    /// Returns an (r, g, b) color for use in UI indicators.
    pub fn color_rgb(&self) -> (u8, u8, u8) {
        match self {
            StatusEffect::Confused { .. } => (230, 150, 50),
            StatusEffect::Blind => (150, 150, 160),
            StatusEffect::SpeedMod(p) if *p > 1.0 => (80, 220, 100),
            StatusEffect::SpeedMod(_) => (190, 100, 30),
            StatusEffect::MissileMod(p) if *p > 1.0 => (100, 180, 255),
            StatusEffect::MissileMod(_) => (130, 90, 200),
            StatusEffect::Displacing => (50, 210, 200),
        }
    }
}

impl StatusKind for StatusEffect {
    fn speed_factor(&self) -> Option<f32> {
        if let StatusEffect::SpeedMod(p) = self { Some(*p) } else { None }
    }

    /// Blindness isn't a per-kind "full vision at 0.0" multiplier target the way SpeedMod
    /// pushes toward a target speed — it just linearly zeroes vision as its own strength ramps
    /// up. Both are the same blend (`1 + (factor - 1) * strength`) with `factor = 0.0`.
    fn vision_factor(&self) -> Option<f32> {
        if let StatusEffect::Blind = self { Some(0.0) } else { None }
    }

    fn base_strength(&self) -> f32 {
        if matches!(self, StatusEffect::Confused { .. }) { 0.5 } else { 1.0 }
    }

    fn tick(&mut self, dt: f32, rng: &mut dyn RngCore) {
        if let StatusEffect::Confused { wander_dir } = self {
            let angle = Rng::gen_range(rng, -std::f32::consts::PI..std::f32::consts::PI) * dt * 8.0;
            let (sin, cos) = angle.sin_cos();
            *wander_dir = Vec2::new(
                wander_dir.x * cos - wander_dir.y * sin,
                wander_dir.x * sin + wander_dir.y * cos,
            )
            .normalize_or_zero();
            if *wander_dir == Vec2::ZERO {
                let a = Rng::gen_range(rng, 0.0..std::f32::consts::TAU);
                *wander_dir = Vec2::new(a.cos(), a.sin());
            }
        }
    }
}

/// Returns `(strength, wander_dir)` for the strongest active Confused effect, if any. Needs
/// direct access to `StatusEffect::Confused`'s payload, so it can't be a generic
/// `StatusEffects<K>` method the way `strength_of`/`multiplier_of` are.
pub fn confused_strength_and_dir(effects: &StatusEffects) -> Option<(f32, Vec2)> {
    effects
        .0
        .iter()
        .filter_map(|e| {
            if let StatusEffect::Confused { wander_dir } = e.kind {
                let s = e.effective_strength();
                if s > 0.0 { Some((s, wander_dir)) } else { None }
            } else {
                None
            }
        })
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
}

pub fn blind_strength(effects: &StatusEffects) -> f32 {
    effects.strength_of(|k| matches!(k, StatusEffect::Blind))
}

/// Combined missile damage multiplier. 1.0 if none active. Game-only: the engine doesn't know
/// what a missile is.
pub fn missile_multiplier(effects: &StatusEffects) -> f32 {
    effects.multiplier_of(|k| if let StatusEffect::MissileMod(p) = k { Some(*p) } else { None })
}

/// Effective strength of the strongest active Confused effect, if any (no movement cap — used
/// for missile mis-aim).
pub fn confusion_strength(effects: &StatusEffects) -> f32 {
    effects.strength_of(|k| matches!(k, StatusEffect::Confused { .. }))
}

pub fn displacing_strength(effects: &StatusEffects) -> f32 {
    effects.strength_of(|k| matches!(k, StatusEffect::Displacing))
}

/// Writes the engine's narrow `MovementModifiers` component from this game's `StatusEffects`
/// (speed/vision multipliers) and `SlowZoneMultiplier` (a terrain effect, folded into speed
/// here since it's a second, unrelated modifier source the engine has no generic way to
/// combine), so `nav::apply_nav_velocity` and `fov::update_fov` never need to know this game's
/// status-effect types. Every agent that needs steering or FOV (player and monsters alike)
/// gets `MovementModifiers` at spawn time; see `populate_level.rs`.
pub fn sync_movement_modifiers(
    mut query: Query<(&StatusEffects, Option<&SlowZoneMultiplier>, &mut MovementModifiers)>,
) {
    for (effects, torpor, mut modifiers) in &mut query {
        modifiers.speed_multiplier =
            effects.speed_multiplier() * torpor.map(|t| t.get()).unwrap_or(1.0);
        modifiers.vision_multiplier = effects.vision_multiplier();
    }
}

/// Applies the Confused status effect to all entities that have both
/// `StatusEffects` and `LinearVelocity`. Blends the velocity direction with the
/// wander direction weighted by strength, but tries to avoid heading straight into walls.
pub fn apply_confusion_to_velocity(
    mut query: Query<(&Transform, &mut LinearVelocity, &StatusEffects)>,
    spatial_query: SpatialQuery,
) {
    for (transform, mut vel, effects) in query.iter_mut() {
        let Some((strength, wander_dir)) = confused_strength_and_dir(effects) else { continue };
        let speed = vel.0.length();
        if speed < 0.01 {
            continue;
        }
        let current_dir = vel.0 / speed;
        let blended = current_dir.lerp(wander_dir, strength).normalize_or_zero();
        if blended == Vec2::ZERO {
            continue;
        }
        let pos = transform.translation.truncate();
        let wall_ahead = Dir2::new(blended).is_ok_and(|dir| {
            spatial_query
                .cast_ray(
                    pos,
                    dir,
                    AGENT_RADIUS * 2.5,
                    true,
                    &SpatialQueryFilter::from_mask(GameLayer::Wall),
                )
                .is_some()
        });
        if !wall_ahead {
            vel.0 = blended * speed;
        }
    }
}
