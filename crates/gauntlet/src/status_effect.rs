// This game's status-effect vocabulary, built on the engine's generic
// `rogue_angles::status_effects` framework — the same framework `vector-arena` uses for a
// much larger effect set, here reduced to the one thing a dig-shot's knockback needs.
use bevy::prelude::*;
use rogue_angles::{dungeon::terrain::SlowZoneMultiplier, movement::MovementModifiers};

pub type StatusEffects = rogue_angles::status_effects::StatusEffects<StatusEffect>;

#[derive(Clone, Copy, Debug)]
pub enum StatusEffect {
    /// Scales movement speed. `param` is the full-strength multiplier (< 1.0 = slowed).
    Slowed(f32),
}

impl StatusEffect {
    pub fn label(&self) -> &'static str {
        match self {
            StatusEffect::Slowed(_) => "slowed",
        }
    }

    pub fn color_rgb(&self) -> (u8, u8, u8) {
        match self {
            StatusEffect::Slowed(_) => (110, 110, 110),
        }
    }
}

impl rogue_angles::status_effects::StatusKind for StatusEffect {
    fn speed_factor(&self) -> Option<f32> {
        let StatusEffect::Slowed(p) = self;
        Some(*p)
    }
}

/// Writes the engine's narrow `MovementModifiers` from this game's `StatusEffects` (just
/// `Slowed`) and the engine's own `SlowZoneMultiplier` (a terrain effect, folded into speed
/// here since it's a second, unrelated modifier source), so `nav::apply_nav_velocity` never
/// needs to know this game's status-effect types. Runs for player and monsters alike.
pub fn sync_movement_modifiers(
    mut query: Query<(&StatusEffects, Option<&SlowZoneMultiplier>, &mut MovementModifiers)>,
) {
    for (effects, slow_zone, mut modifiers) in &mut query {
        modifiers.speed_multiplier =
            effects.speed_multiplier() * slow_zone.map(|s| s.get()).unwrap_or(1.0);
    }
}
