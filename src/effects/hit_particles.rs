// Burst particle effect when a magic missile strikes a monster or player.
//
// Triggered by MissileDamageDealt (fired from apply_damage_on_hit after the
// exact HP loss is known). Particle count = damage dealt, rounded. Each
// particle shoots outward at a random angle and decelerates exponentially
// using real time, so the effect plays at normal speed even under time
// dilation from missile slow-motion.

use std::f32::consts::TAU;

use bevy::prelude::*;
use bevy_hanabi::prelude::*;

use crate::{GameState, effects::projectile::MissileDamageDealt};

/// Real seconds for a burst to fully fade out.
const LIFETIME: f32 = 1.45;
/// Initial launch speed at the fast end (pixels/s); varied ±40% per particle.
const INITIAL_SPEED: f32 = 300.0;
/// Exponential drag coefficient (pixels/s² per pixel/s). See position formula below.
const DRAG: f32 = 5.0;
/// World-space size of each particle quad (pixels).
const PARTICLE_SIZE: f32 = 2.5;

pub struct HitParticlesPlugin;

impl Plugin for HitParticlesPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(spawn_hit_burst).add_systems(Update, expire_hit_bursts);
    }
}

/// Marks an effect entity for real-time expiry (independent of virtual-time pause).
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
    mut effects: ResMut<Assets<EffectAsset>>,
    time: Res<Time<Real>>,
) {
    let event = trigger.event();
    let count = event.damage.round().max(1.0);
    let (r, g, b) =
        if event.fired_by_player { (0.2_f32, 0.4, 0.7) } else { (0.7_f32, 0.3, 0.2) };

    let effect = effects.add(create_hit_burst_effect(count, r, g, b));
    commands.spawn((
        DespawnOnExit(GameState::InLevel),
        HitBurstExpiry(time.elapsed_secs() + LIFETIME + 0.1),
        ParticleEffect::new(effect),
        Transform::from_translation(
            event.position.truncate().extend(crate::fov::MOVABLE_Z + 2.0),
        ),
    ));
}

fn create_hit_burst_effect(count: f32, r: f32, g: f32, b: f32) -> EffectAsset {
    let writer = ExprWriter::new();

    // --- Init: random angle + speed → stored as velocity in (F32_0, F32_1).
    // Birth real-time is stored in F32_2 so the update pass can compute particle age.
    let angle = writer.rand(ScalarType::Float).mul(writer.lit(TAU));
    // Speed in [0.6 * INITIAL_SPEED, 1.4 * INITIAL_SPEED]
    let speed =
        writer.rand(ScalarType::Float).mul(writer.lit(0.8_f32)).add(writer.lit(0.6_f32)).mul(writer.lit(INITIAL_SPEED));
    let init_vel_x =
        SetAttributeModifier::new(Attribute::F32_0, angle.clone().cos().mul(speed.clone()).expr());
    let init_vel_y = SetAttributeModifier::new(Attribute::F32_1, angle.sin().mul(speed).expr());

    let birth_time = writer.push(Expr::BuiltIn(BuiltInExpr::new(BuiltInOperator::RealTime)));
    let init_birth_time = SetAttributeModifier::new(Attribute::F32_2, birth_time.expr());

    // --- Update: position from exponential-decay velocity integral, color fades.
    //
    // For a particle whose velocity decays as v(t) = v0 * exp(-DRAG * t):
    //   position(t) = v0 * (1 - exp(-DRAG * t)) / DRAG
    //
    // We drive everything from real_time (not virtual) so the burst animates at
    // normal speed even when missiles trigger slow-motion time dilation.
    let real_time = writer.push(Expr::BuiltIn(BuiltInExpr::new(BuiltInOperator::RealTime)));
    let age = real_time.sub(writer.attr(Attribute::F32_2));

    let decay = age.clone().mul(writer.lit(-DRAG)).exp(); // exp(-DRAG * age)
    let pos_scale = writer.lit(1.0_f32).sub(decay).mul(writer.lit(1.0 / DRAG));
    let pos = writer
        .attr(Attribute::F32_0)
        .mul(pos_scale.clone())
        .vec3(writer.attr(Attribute::F32_1).mul(pos_scale), writer.lit(0.0_f32));
    let update_pos = SetAttributeModifier::new(Attribute::POSITION, pos.expr());

    // Alpha goes from 1.0 (born) to 0.0 (at LIFETIME). pack4x8unorm clamps to [0,1].
    let alpha = writer.lit(1.0_f32).sub(age.div(writer.lit(LIFETIME)));
    let color_vec = writer.lit(Vec3::new(r, g, b)).vec4_xyz_w(alpha);
    let update_color = SetAttributeModifier::new(Attribute::COLOR, color_vec.pack4x8unorm().expr());

    let module = writer.finish();

    EffectAsset::new(count.ceil() as u32 + 1, SpawnerSettings::once(count.into()), module)
        .with_name("hit_burst")
        .with_simulation_space(SimulationSpace::Local)
        .with_simulation_condition(SimulationCondition::Always)
        // Position is fully computed each frame from real_time; no velocity integration.
        .with_motion_integration(MotionIntegration::None)
        .init(init_vel_x)
        .init(init_vel_y)
        .init(init_birth_time)
        .update(update_pos)
        .update(update_color)
        .render(SetSizeModifier { size: Vec3::splat(PARTICLE_SIZE).into() })
}
