// Status effects for players and monsters.
// Each effect has a total duration and tracks remaining time. Effective strength
// is 1.0 until the last 0.5 seconds, then ramps linearly down to 0.
use avian2d::prelude::*;
use bevy::prelude::*;
use rand::Rng;

use crate::{AGENT_RADIUS, GameLayer};

const RAMP_DOWN_SECS: f32 = 0.5;

#[derive(Clone, Debug)]
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

#[derive(Clone, Debug)]
pub struct ActiveStatusEffect {
    pub kind: StatusEffect,
    pub base_strength: f32,
    pub remaining: f32,
}

impl ActiveStatusEffect {
    pub fn new(kind: StatusEffect, duration: f32) -> Self {
        let base_strength = if matches!(kind, StatusEffect::Confused { .. }) { 0.5 } else { 1.0 };
        Self { kind, base_strength, remaining: duration }
    }

    /// Strength in [0, 1]. Ramps from 1 → 0 over the last RAMP_DOWN_SECS.
    pub fn effective_strength(&self) -> f32 {
        (self.remaining / RAMP_DOWN_SECS).clamp(0.0, 1.0) * self.base_strength
    }
}

#[derive(Component, Default, Clone)]
pub struct StatusEffects(pub Vec<ActiveStatusEffect>);

impl StatusEffects {
    pub fn add(&mut self, kind: StatusEffect, duration: f32) {
        // TODO: add to/cancel existing matching status effects
        self.0.push(ActiveStatusEffect::new(kind, duration));
    }

    /// Returns `(strength, wander_dir)` for the strongest active Confused effect, if any.
    pub fn confused_strength_and_dir(&self) -> Option<(f32, Vec2)> {
        self.0
            .iter()
            .filter_map(|e| {
                if let StatusEffect::Confused { wander_dir } = e.kind {
                    if s > 0.0 { Some((s, wander_dir)) } else { None }
                } else {
                    None
                }
            })
            .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
    }

    pub fn blind_strength(&self) -> f32 {
        self.0
            .iter()
            .filter(|e| matches!(e.kind, StatusEffect::Blind))
            .map(|e| e.effective_strength())
            .fold(0.0_f32, f32::max)
    }

    /// Combined speed multiplier (multiplicative). 1.0 if none active.
    pub fn speed_multiplier(&self) -> f32 {
        self.0.iter().fold(1.0_f32, |acc, e| {
            if let StatusEffect::SpeedMod(param) = e.kind {
                let s = e.effective_strength();
                acc * (1.0 + (param - 1.0) * s)
            } else {
                acc
            }
        })
    }

    /// Combined missile damage multiplier. 1.0 if none active.
    pub fn missile_multiplier(&self) -> f32 {
        self.0.iter().fold(1.0_f32, |acc, e| {
            if let StatusEffect::MissileMod(param) = e.kind {
                let s = e.effective_strength();
                acc * (1.0 + (param - 1.0) * s)
            } else {
                acc
            }
        })
    }

    /// Returns the effective strength of the strongest active Confused effect, if any.
    /// (No movement cap — used for missile mis-aim.)
    pub fn confusion_strength(&self) -> f32 {
        self.0
            .iter()
            .filter_map(|e| {
                if matches!(e.kind, StatusEffect::Confused { .. }) {
                    let s = e.effective_strength();
                    if s > 0.0 { Some(s) } else { None }
                } else {
                    None
                }
            })
            .fold(0.0_f32, f32::max)
    }

    pub fn displacing_strength(&self) -> f32 {
        self.0
            .iter()
            .filter(|e| matches!(e.kind, StatusEffect::Displacing))
            .map(|e| e.effective_strength())
            .fold(0.0_f32, f32::max)
    }

    pub fn is_empty(&self) -> bool { self.0.is_empty() }
}

/// Ticks durations down, advances confused wander directions, and removes expired effects.
pub fn tick_status_effects(time: Res<Time>, mut query: Query<&mut StatusEffects>) {
    let dt = time.delta_secs();
    let mut rng = rand::thread_rng();

    for mut effects in query.iter_mut() {
        effects.0.retain_mut(|e| {
            e.remaining -= dt;
            if e.remaining <= 0.0 {
                return false;
            }
            // Random walk: rotate wander_dir by up to ±π rad/s
            if let StatusEffect::Confused { ref mut wander_dir } = e.kind {
                let angle = rng.gen_range(-std::f32::consts::PI..std::f32::consts::PI) * dt * 8.0;
                let (sin, cos) = angle.sin_cos();
                *wander_dir = Vec2::new(
                    wander_dir.x * cos - wander_dir.y * sin,
                    wander_dir.x * sin + wander_dir.y * cos,
                )
                .normalize_or_zero();
                if *wander_dir == Vec2::ZERO {
                    let a = rng.gen_range(0.0..std::f32::consts::TAU);
                    *wander_dir = Vec2::new(a.cos(), a.sin());
                }
            }
            true
        });
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
        let Some((strength, wander_dir)) = effects.confused_strength_and_dir() else { continue };
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
