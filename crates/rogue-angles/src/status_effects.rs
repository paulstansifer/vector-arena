// Generic status-effect framework: the engine owns the shape (timed effects that ramp
// strength down before expiring, plus generic aggregation helpers), while the game supplies
// the vocabulary — its own `K` enum — and tells the engine which of its variants affect
// movement speed or vision via `StatusKind`. Everything else about a game's effects (damage
// multipliers, AI behavior, whatever) is the game's business, read back out of
// `StatusEffects<K>` directly; the engine never needs to know it exists. Composing the
// resulting speed/vision multipliers into `movement::MovementModifiers` — alongside any other
// modifier source a game has, e.g. a terrain-based slow zone — stays a game-side system; see
// the doc comment on `movement::MovementModifiers`.
use bevy::prelude::*;
use rand::RngCore;

const RAMP_DOWN_SECS: f32 = 0.5;

/// A game's own status-effect vocabulary.
pub trait StatusKind: Copy + Send + Sync + 'static {
    /// Multiplicative movement-speed contribution at full (1.0) strength, blended toward 1.0
    /// (no effect) as the effect's own strength ramps down. `None` if this kind doesn't affect
    /// movement speed.
    fn speed_factor(&self) -> Option<f32> { None }

    /// Multiplicative vision-radius contribution at full strength, blended the same way as
    /// `speed_factor`. `None` if this kind doesn't affect vision.
    fn vision_factor(&self) -> Option<f32> { None }

    /// Strength an instance of this kind starts at (before the end-of-duration ramp-down), as
    /// a fraction in [0, 1]. Most effects are full-strength for their whole duration; override
    /// for one that's only ever partial.
    fn base_strength(&self) -> f32 { 1.0 }

    /// Per-frame per-instance update hook, called on every active effect of this kind before
    /// it's checked for expiry — e.g. rotating a "confused" wander direction. No-op by default.
    fn tick(&mut self, _dt: f32, _rng: &mut dyn RngCore) {}
}

#[derive(Clone, Copy, Debug)]
pub struct ActiveStatusEffect<K> {
    pub kind: K,
    pub base_strength: f32,
    pub remaining: f32,
}

impl<K: StatusKind> ActiveStatusEffect<K> {
    pub fn new(kind: K, duration: f32) -> Self {
        let base_strength = kind.base_strength();
        Self { kind, base_strength, remaining: duration }
    }

    /// Strength in [0, 1]. Ramps from `base_strength` to 0 over the last `RAMP_DOWN_SECS`.
    pub fn effective_strength(&self) -> f32 {
        (self.remaining / RAMP_DOWN_SECS).clamp(0.0, 1.0) * self.base_strength
    }
}

/// A timed collection of status effects of a game-defined kind `K`, attachable to any entity
/// (player, monster, ...) that should be affected by them.
#[derive(Component, Clone)]
pub struct StatusEffects<K>(pub Vec<ActiveStatusEffect<K>>);

impl<K> Default for StatusEffects<K> {
    fn default() -> Self { Self(Vec::new()) }
}

impl<K: StatusKind> StatusEffects<K> {
    pub fn add(&mut self, kind: K, duration: f32) {
        // TODO: refresh/extend a matching existing effect instead of always stacking a new one.
        self.0.push(ActiveStatusEffect::new(kind, duration));
    }

    pub fn is_empty(&self) -> bool { self.0.is_empty() }

    /// The strongest active effect matching `pred`, or 0.0 if none are active.
    pub fn strength_of(&self, pred: impl Fn(&K) -> bool) -> f32 {
        self.0.iter().filter(|e| pred(&e.kind)).map(|e| e.effective_strength()).fold(0.0_f32, f32::max)
    }

    /// Combines every active effect's `param(kind)` (if any) into one multiplier: each
    /// contributing effect blends its full-strength `param` toward 1.0 (no effect) as its own
    /// strength ramps down, and all active contributions multiply together. This is the pattern
    /// `speed_factor`/`vision_factor` (and any game-only per-kind multiplier) are meant to use.
    pub fn multiplier_of(&self, param: impl Fn(&K) -> Option<f32>) -> f32 {
        self.0.iter().fold(1.0_f32, |acc, e| match param(&e.kind) {
            Some(p) => acc * (1.0 + (p - 1.0) * e.effective_strength()),
            None => acc,
        })
    }

    pub fn speed_multiplier(&self) -> f32 { self.multiplier_of(|k| k.speed_factor()) }

    pub fn vision_multiplier(&self) -> f32 { self.multiplier_of(|k| k.vision_factor()) }
}

/// Ticks every `StatusEffects<K>`'s durations down, running each active effect's `tick` hook,
/// and removing expired effects. Registered by `add_status_effects::<K>()`.
pub fn tick_status_effects<K: StatusKind>(time: Res<Time>, mut query: Query<&mut StatusEffects<K>>) {
    let dt = time.delta_secs();
    let mut rng = rand::thread_rng();

    for mut effects in &mut query {
        effects.0.retain_mut(|e| {
            e.remaining -= dt;
            if e.remaining <= 0.0 {
                return false;
            }
            e.kind.tick(dt, &mut rng);
            true
        });
    }
}

/// Registers the generic ticking system for a game's `StatusKind`. Call once per `K` a game
/// uses (most games only have one).
pub trait StatusEffectsAppExt {
    fn add_status_effects<K: StatusKind>(&mut self) -> &mut Self;
}

impl StatusEffectsAppExt for App {
    fn add_status_effects<K: StatusKind>(&mut self) -> &mut Self {
        self.add_systems(Update, tick_status_effects::<K>)
    }
}
