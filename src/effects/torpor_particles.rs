// Ambient particle effect for torpor zones, built on bevy_hanabi.
//
// Each particle renders as two stacked dots (a whiter-than-the-zone highlight on
// top and a slightly-darker-than-the-zone shadow 1.5px below it) that drift
// slowly with Brownian-like motion. The motion is driven by `real_time` rather
// than the simulation clock, so particles keep wandering even while the game's
// virtual time is paused (e.g. while the player is idle).
//
// Because this plugin pulls in hanabi's render-side machinery (which needs a GPU
// device), it is registered by the binaries that actually render (the `game`
// binary and the headless screenshot runner) rather than by `GamePlugin`, so the
// GPU-less integration tests are unaffected.

use crate::{GameState, dungeon::terrain::TorporZoneParticles};
use bevy::{
    asset::RenderAssetUsages,
    image::ImageSampler,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use bevy_hanabi::prelude::*;

/// The fill color of torpor zones (matches the zone mesh material in `game.rs`).
/// Particle colors are derived from this so they read as "part of" the zone.
pub const TORPOR_ZONE_COLOR: Srgba = Srgba::new(0.3, 0.7, 1.0, 0.35);

/// Roughly one particle per this many square pixels of zone area.
const PARTICLE_DENSITY: f32 = 1.0 / 520.0;

/// Diameter of each dot, in world pixels.
const DOT_DIAMETER: f32 = 1.5;
/// Vertical spacing between the highlight and shadow dot centers, in world pixels.
const DOT_SPACING: f32 = 1.5;

pub struct TorporParticlesPlugin;

impl Plugin for TorporParticlesPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(HanabiPlugin)
            .add_systems(Startup, setup_particle_texture)
            .add_systems(Update, spawn_particles_for_new_zones);
    }
}

/// Shared two-dot texture used by every torpor zone's particle effect.
#[derive(Resource)]
struct TorporParticleTexture(Handle<Image>);

fn setup_particle_texture(mut images: ResMut<Assets<Image>>, mut commands: Commands) {
    commands.insert_resource(TorporParticleTexture(images.add(build_two_dot_texture())));
}

/// Spawn a particle effect for each torpor zone as soon as its marker appears.
fn spawn_particles_for_new_zones(
    mut commands: Commands,
    mut effects: ResMut<Assets<EffectAsset>>,
    texture: Res<TorporParticleTexture>,
    zones: Query<&TorporZoneParticles, Added<TorporZoneParticles>>,
) {
    for zone in &zones {
        let area = (2.0 * zone.half_size.x) * (2.0 * zone.half_size.y);
        let count = (area * PARTICLE_DENSITY).ceil().max(1.0);

        let effect = effects.add(create_torpor_effect(zone.half_size, count));

        commands.spawn((
            DespawnOnExit(GameState::InLevel),
            ParticleEffect::new(effect),
            EffectMaterial { images: vec![texture.0.clone()] },
            // Local-space simulation: particle positions are offsets from this
            // transform, which sits at the zone center, just above the zone mesh.
            Transform::from_translation(zone.center.extend(crate::fov::TERRAIN_Z + 0.2)),
        ));
    }
}

/// Build the per-zone effect asset. `half_size` is the half-extent of the zone's
/// bounding box; particles are seeded uniformly across that box. (Today's zones
/// are rectangles, so the box matches the zone exactly; for future arbitrary
/// shapes this is bounding-box sampling, which may scatter a few particles
/// outside the polygon.)
///
/// Particles are spawned once and live forever, drifting in real time; this
/// keeps each zone populated and gently moving even while the game is paused
/// (no virtual-time delta is needed for spawning or motion).
fn create_torpor_effect(half_size: Vec2, count: f32) -> EffectAsset {
    use std::f32::consts::TAU;

    let writer = ExprWriter::new();

    // --- Init: pick a fixed anchor in the bounding box and random drift phases.
    let rand_centered =
        |scale: f32| writer.rand(ScalarType::Float).sub(writer.lit(0.5)).mul(writer.lit(scale));
    let init_anchor_x =
        SetAttributeModifier::new(Attribute::F32_0, rand_centered(2.0 * half_size.x).expr());
    let init_anchor_y =
        SetAttributeModifier::new(Attribute::F32_1, rand_centered(2.0 * half_size.y).expr());
    let init_phase_x = SetAttributeModifier::new(
        Attribute::F32_2,
        writer.rand(ScalarType::Float).mul(writer.lit(TAU)).expr(),
    );
    let init_phase_y = SetAttributeModifier::new(
        Attribute::F32_3,
        writer.rand(ScalarType::Float).mul(writer.lit(TAU)).expr(),
    );
    // Start each particle at its anchor.
    let init_pos = SetAttributeModifier::new(
        Attribute::POSITION,
        writer.attr(Attribute::F32_0).vec3(writer.attr(Attribute::F32_1), writer.lit(0.0)).expr(),
    );
    // White base color; the texture supplies the dots' actual colors via Modulate.
    let init_color =
        SetAttributeModifier::new(Attribute::COLOR, writer.lit(Vec4::ONE).pack4x8unorm().expr());

    // --- Update: recompute POSITION = anchor + slow Brownian-like drift.
    // We sum sines at incommensurate (real-time) frequencies, with a per-particle
    // random phase, to get smooth wandering. Driving this from `real_time` keeps
    // it moving even when the game's virtual clock is paused.
    let real_time = writer.push(Expr::BuiltIn(BuiltInExpr::new(BuiltInOperator::RealTime)));
    const AMP: f32 = 5.0; // max ~7.5px drift from anchor (sum of the two terms)
    const F1: f32 = 0.21 * 4.0; // rad/s
    const F2: f32 = 0.13 * 4.0;
    const F3: f32 = 0.17 * 4.0;
    const F4: f32 = 0.19 * 4.0;

    let phase_x = writer.attr(Attribute::F32_2);
    let drift_x = real_time
        .clone()
        .mul(writer.lit(F1))
        .add(phase_x.clone())
        .sin()
        .add(
            real_time
                .clone()
                .mul(writer.lit(F2))
                .add(phase_x.mul(writer.lit(2.7)))
                .sin()
                .mul(writer.lit(0.5)),
        )
        .mul(writer.lit(AMP));

    let phase_y = writer.attr(Attribute::F32_3);
    let drift_y = real_time
        .clone()
        .mul(writer.lit(F4))
        .add(phase_y.clone())
        .cos()
        .add(
            real_time
                .mul(writer.lit(F3))
                .add(phase_y.mul(writer.lit(1.7)))
                .sin()
                .mul(writer.lit(0.5)),
        )
        .mul(writer.lit(AMP));

    let pos = writer
        .attr(Attribute::F32_0)
        .add(drift_x)
        .vec3(writer.attr(Attribute::F32_1).add(drift_y), writer.lit(0.0));
    let update_pos = SetAttributeModifier::new(Attribute::POSITION, pos.expr());

    let mut module = writer.finish();
    module.add_texture_slot("dots");

    let dot_texture = ParticleTextureModifier {
        texture_slot: module.lit(0u32),
        sample_mapping: ImageSampleMapping::Modulate,
    };

    // Quad sized so the texture's dots render at the requested pixel sizes.
    let quad = particle_quad_size();

    EffectAsset::new(count.ceil() as u32 + 1, SpawnerSettings::once(count.into()), module)
        .with_name("torpor_particles")
        .with_simulation_space(SimulationSpace::Local)
        // Keep simulating (and drifting) even when the zone is off-screen or the
        // game's virtual clock is paused.
        .with_simulation_condition(SimulationCondition::Always)
        // Position is fully recomputed each frame from real_time; no velocity
        // integration (and no VELOCITY attribute) is involved.
        .with_motion_integration(MotionIntegration::None)
        .init(init_anchor_x)
        .init(init_anchor_y)
        .init(init_phase_x)
        .init(init_phase_y)
        .init(init_pos)
        .init(init_color)
        .update(update_pos)
        .render(SetSizeModifier { size: quad.extend(1.0).into() })
        .render(dot_texture)
}

/// World-space size of the particle billboard. The texture is laid out so the
/// dots occupy a fixed fraction of the quad (see `build_two_dot_texture`); this
/// size makes them render at `DOT_DIAMETER` / `DOT_SPACING` pixels.
fn particle_quad_size() -> Vec2 {
    // Texture: dots have diameter = 3/4 of the texture width, and the dot centers
    // are 3/8 of the texture height apart (see the layout constants below).
    Vec2::new(DOT_DIAMETER / 0.75, DOT_SPACING / 0.375)
}

/// Build the 32x64 RGBA texture containing a whiter highlight dot above a darker
/// shadow dot. Drawn with anti-aliased edges and stored in sRGB space.
fn build_two_dot_texture() -> Image {
    const W: i32 = 32;
    const H: i32 = 64;
    // Dot radius = 12/32 of width; centers at y = 20 and 44 (24px = 3/8 of H apart).
    const RADIUS: f32 = 12.0;
    let top_center = Vec2::new(16.0, 20.0);
    let bottom_center = Vec2::new(16.0, 44.0);

    let base = TORPOR_ZONE_COLOR;
    // Whiter than the zone: lerp the zone color toward white.
    let highlight = lerp_rgb(base, 1.0, 1.0, 1.0, 0.85);
    // Slightly darker than the zone.
    let shadow = (base.red * 0.75, base.green * 0.75, base.blue * 0.75);
    let dot_alpha = 0.9;

    let mut data = vec![0u8; (W * H * 4) as usize];
    for y in 0..H {
        for x in 0..W {
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            // Anti-aliased coverage: full inside, ramps to 0 over a 1px edge.
            let cov_top = (RADIUS + 0.5 - p.distance(top_center)).clamp(0.0, 1.0);
            let cov_bot = (RADIUS + 0.5 - p.distance(bottom_center)).clamp(0.0, 1.0);

            // Composite the highlight (top) over the shadow (bottom).
            let a_top = dot_alpha * cov_top;
            let a_bot = dot_alpha * cov_bot;
            let out_a = a_top + a_bot * (1.0 - a_top);
            let (r, g, b) = if out_a > 0.0 {
                let blend = |t: f32, bo: f32| (t * a_top + bo * a_bot * (1.0 - a_top)) / out_a;
                (
                    blend(highlight.0, shadow.0),
                    blend(highlight.1, shadow.1),
                    blend(highlight.2, shadow.2),
                )
            } else {
                (0.0, 0.0, 0.0)
            };

            let idx = ((y * W + x) * 4) as usize;
            data[idx] = (r * 255.0).round() as u8;
            data[idx + 1] = (g * 255.0).round() as u8;
            data[idx + 2] = (b * 255.0).round() as u8;
            data[idx + 3] = (out_a * 255.0).round() as u8;
        }
    }

    let mut image = Image::new(
        Extent3d { width: W as u32, height: H as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    // Smooth the heavily-minified dots.
    image.sampler = ImageSampler::linear();
    image
}

/// Linear interpolation of an `Srgba`'s RGB toward `(r, g, b)` by `t`.
fn lerp_rgb(c: Srgba, r: f32, g: f32, b: f32, t: f32) -> (f32, f32, f32) {
    (c.red + (r - c.red) * t, c.green + (g - c.green) * t, c.blue + (b - c.blue) * t)
}
