// Narrow, engine-owned data that game-side status-effect systems write into and
// engine systems (nav steering, FOV) read from. This keeps `nav.rs` and `fov.rs`
// from depending on any game-defined status-effect type: the game computes
// whatever aggregate scalars its own effects imply and writes them here once
// per frame, before the engine systems that consume them run.
use bevy::prelude::*;

/// Marks the entity the engine's FOV system computes visibility from. Exactly
/// one entity should carry this — typically the player, but the engine doesn't
/// assume that; it only assumes "whoever has `Viewer`".
#[derive(Component)]
pub struct Viewer;

/// Scalar multipliers a game's status-effect system computes and writes each
/// frame. Defaults are the "no effect" identity (full speed, full vision).
#[derive(Component, Clone, Copy)]
pub struct MovementModifiers {
    /// Multiplies steering speed in `nav::apply_nav_velocity`.
    pub speed_multiplier: f32,
    /// Multiplies FOV radius in `fov::update_fov`. 1.0 = full radius, 0.0 = blind.
    pub vision_multiplier: f32,
}

impl Default for MovementModifiers {
    fn default() -> Self { Self { speed_multiplier: 1.0, vision_multiplier: 1.0 } }
}
