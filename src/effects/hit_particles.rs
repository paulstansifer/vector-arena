// Burst particle effect when a magic missile strikes a monster or player.
//
// Triggered by MissileDamageDealt (fired from apply_damage_on_hit after the
// exact HP loss is known). Particle count = damage dealt, rounded. Each
// particle shoots outward at a random angle. The effect uses virtual time, so
// it will slow with bullet-time and pause when the player is idle — both cases
// are brief and visually acceptable.

use bevy::prelude::*;
use bevy_enoki::prelude::*;

use crate::{GameState, effects::projectile::MissileDamageDealt};

/// Real seconds for a burst to fully fade out.
const LIFETIME: f32 = 1.45;
/// Initial launch speed (pixels/s); varied ±40% per particle.
const INITIAL_SPEED: f32 = 60.0;
/// World-space size of each particle quad (pixels).
const PARTICLE_SIZE: f32 = 2.5;

pub struct HitParticlesPlugin;

impl Plugin for HitParticlesPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EnokiPlugin)
            .add_observer(spawn_hit_burst)
            .add_systems(Update, expire_hit_bursts);
    }
}

/// Marks an effect entity for real-time expiry so it cleans up even if virtual
/// time is paused (preventing frozen particle entities lingering until level end).
#[derive(Component)]
struct HitBurstExpiry(f32);

fn expire_hit_bursts(
    mut commands: Commands,
    time: Res<Time<Real>>,
    query: Query<(Entity, &HitBurstExpiry)>,
) {
    let now = time.elapsed_secs();
    for (entity, expiry) in &query {
        if now >= expiry.0 {
            commands.entity(entity).despawn();
        }
    }
}

fn spawn_hit_burst(
    trigger: On<MissileDamageDealt>,
    mut commands: Commands,
    mut effects: ResMut<Assets<Particle2dEffect>>,
    time: Res<Time<Real>>,
) {
    let event = trigger.event();
    let count = event.damage.round().max(1.0);
    let (r, g, b) = if event.fired_by_player { (0.2_f32, 0.4, 0.7) } else { (0.7_f32, 0.3, 0.2) };

    let effect = effects.add(create_hit_burst_effect(count, r, g, b));
    commands.spawn((
        DespawnOnExit(GameState::InLevel),
        // Real-time fallback in case virtual time is paused when the burst fires.
        HitBurstExpiry(time.elapsed_secs() + LIFETIME + 0.5),
        ParticleSpawner::<ColorParticle2dMaterial>::default(),
        ParticleEffectHandle(effect),
        OneShot::Despawn,
        Transform::from_translation(event.position.truncate().extend(crate::fov::MOVABLE_Z + 2.0)),
    ));
}

fn create_hit_burst_effect(count: f32, r: f32, g: f32, b: f32) -> Particle2dEffect {
    let color_curve = MultiCurve::new()
        .with_point(LinearRgba::new(r, g, b, 1.0), 0.0, None)
        .with_point(LinearRgba::new(r, g, b, 0.0), 1.0, None);

    Particle2dEffect {
        // spawn_rate near-zero so the burst fires on the first update tick.
        spawn_rate: 0.001,
        spawn_amount: count.ceil() as u32,
        emission_shape: EmissionShape::Point,
        lifetime: Rval::new(LIFETIME, 0.0),
        // randomness=1.0 rotates direction by ±π → full 360° spread.
        direction: Some(Rval::new(Vec2::Y, 1.0)),
        linear_speed: Some(Rval::new(INITIAL_SPEED, 0.4)),
        scale: Some(Rval::new(PARTICLE_SIZE, 0.0)),
        color_curve: Some(color_curve),
        ..Default::default()
    }
}
