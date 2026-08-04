// Ambient particle effect for torpor zones.
//
// Each dot is a Sprite entity with a TorporDot component storing its anchor
// position and random drift phases. A real-time system recomputes the transform
// each frame using a sum of sines at incommensurate frequencies, so the drift
// continues even while the game's virtual time is paused (player idle).
//
// This plugin is registered by the binaries that actually render (the `game`
// binary and the headless screenshot runner) rather than by `GamePlugin`, so the
// GPU-less integration tests are unaffected.

use std::f32::consts::TAU;

use bevy::{
    asset::RenderAssetUsages,
    image::ImageSampler,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};

use rogue_angles::{LevelEntity, dungeon::terrain::SlowZoneMarker};

/// The fill color of torpor zones (matches the zone mesh material in `game.rs`).
/// Particle colors are derived from this so they read as "part of" the zone.
pub const TORPOR_ZONE_COLOR: Srgba = Srgba::new(0.3, 0.7, 1.0, 0.35);

/// Roughly one particle per this many square pixels of zone area.
const PARTICLE_DENSITY: f32 = 1.0 / 520.0;

/// Diameter of each dot, in world pixels.
const DOT_DIAMETER: f32 = 1.5;
/// Vertical spacing between the highlight and shadow dot centers, in world pixels.
const DOT_SPACING: f32 = 1.5;

const AMP: f32 = 5.0;
const F1: f32 = 0.21 * 4.0;
const F2: f32 = 0.13 * 4.0;
const F3: f32 = 0.17 * 4.0;
const F4: f32 = 0.19 * 4.0;

pub struct TorporParticlesPlugin;

impl Plugin for TorporParticlesPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_particle_texture)
            .add_systems(Update, (spawn_particles_for_new_zones, update_torpor_dots));
    }
}

/// Shared two-dot texture used by every torpor zone's dot sprites.
#[derive(Resource)]
struct TorporParticleTexture(Handle<Image>);

/// Per-dot drift state; the update system drives Transform from this + real time.
#[derive(Component)]
struct TorporDot {
    base_pos: Vec3,
    phase_x: f32,
    phase_y: f32,
}

fn setup_particle_texture(mut images: ResMut<Assets<Image>>, mut commands: Commands) {
    commands.insert_resource(TorporParticleTexture(images.add(build_two_dot_texture())));
}

fn spawn_particles_for_new_zones(
    mut commands: Commands,
    texture: Res<TorporParticleTexture>,
    zones: Query<&SlowZoneMarker, Added<SlowZoneMarker>>,
) {
    let quad = particle_quad_size();
    let z = rogue_angles::fov::TERRAIN_Z + 0.2;

    for zone in &zones {
        let area = (2.0 * zone.half_size.x) * (2.0 * zone.half_size.y);
        let count = (area * PARTICLE_DENSITY).ceil().max(1.0) as u32;

        for _ in 0..count {
            let anchor = Vec2::new(
                (rand::random::<f32>() - 0.5) * 2.0 * zone.half_size.x,
                (rand::random::<f32>() - 0.5) * 2.0 * zone.half_size.y,
            );
            let base_pos = (zone.center + anchor).extend(z);

            commands.spawn((
                LevelEntity,
                TorporDot {
                    base_pos,
                    phase_x: rand::random::<f32>() * TAU,
                    phase_y: rand::random::<f32>() * TAU,
                },
                Sprite { image: texture.0.clone(), custom_size: Some(quad), ..default() },
                Transform::from_translation(base_pos),
            ));
        }
    }
}

fn update_torpor_dots(time: Res<Time<Real>>, mut query: Query<(&TorporDot, &mut Transform)>) {
    let t = time.elapsed_secs();
    for (dot, mut transform) in &mut query {
        let drift_x =
            AMP * ((F1 * t + dot.phase_x).sin() + 0.5 * (F2 * t + 2.7 * dot.phase_x).sin());
        let drift_y =
            AMP * ((F4 * t + dot.phase_y).cos() + 0.5 * (F3 * t + 1.7 * dot.phase_y).sin());
        transform.translation = dot.base_pos + Vec3::new(drift_x, drift_y, 0.0);
    }
}

/// World-space size of the particle billboard. The texture is laid out so the
/// dots occupy a fixed fraction of the quad (see `build_two_dot_texture`); this
/// size makes them render at `DOT_DIAMETER` / `DOT_SPACING` pixels.
fn particle_quad_size() -> Vec2 { Vec2::new(DOT_DIAMETER / 0.75, DOT_SPACING / 0.375) }

/// Build the 32x64 RGBA texture containing a whiter highlight dot above a darker
/// shadow dot. Drawn with anti-aliased edges and stored in sRGB space.
fn build_two_dot_texture() -> Image {
    const W: i32 = 32;
    const H: i32 = 64;
    const RADIUS: f32 = 12.0;
    let top_center = Vec2::new(16.0, 20.0);
    let bottom_center = Vec2::new(16.0, 44.0);

    let base = TORPOR_ZONE_COLOR;
    let highlight = lerp_rgb(base, 1.0, 1.0, 1.0, 0.85);
    let shadow = (base.red * 0.75, base.green * 0.75, base.blue * 0.75);
    let dot_alpha = 0.9;

    let mut data = vec![0u8; (W * H * 4) as usize];
    for y in 0..H {
        for x in 0..W {
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let cov_top = (RADIUS + 0.5 - p.distance(top_center)).clamp(0.0, 1.0);
            let cov_bot = (RADIUS + 0.5 - p.distance(bottom_center)).clamp(0.0, 1.0);

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
    image.sampler = ImageSampler::linear();
    image
}

fn lerp_rgb(c: Srgba, r: f32, g: f32, b: f32, t: f32) -> (f32, f32, f32) {
    (c.red + (r - c.red) * t, c.green + (g - c.green) * t, c.blue + (b - c.blue) * t)
}
