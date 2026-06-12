// Visual-only short-term effects: HitFlash (color flash on missile hit) and
// StateIndicator (character label on AI state change).
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::fov::CurrentFovState;
use geo::Contains;

// ── HitFlash ────────────────────────────────────────────────────────────────

#[derive(Component)]
pub struct HitFlash {
    pub timer: f32,
    pub duration: f32,
    pub base_color: Color,
}

pub fn update_hit_flash(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut query: Query<(Entity, &mut HitFlash, &MeshMaterial2d<ColorMaterial>)>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    for (entity, mut flash, mat_handle) in query.iter_mut() {
        flash.timer -= time.delta_secs();
        if flash.timer <= 0.0 {
            if let Some(mat) = materials.get_mut(&mat_handle.0) {
                mat.color = flash.base_color;
            }
            commands.entity(entity).remove::<HitFlash>();
        } else {
            let t = flash.timer / flash.duration;
            let base = flash.base_color.to_srgba();
            let color = Color::srgba(
                base.red + t * (1.0 - base.red),
                base.green + t * (1.0 - base.green),
                base.blue + t * (1.0 - base.blue),
                base.alpha,
            );
            if let Some(mat) = materials.get_mut(&mat_handle.0) {
                mat.color = color;
            }
        }
    }
}

// ── StateIndicator ──────────────────────────────────────────────────────────

#[derive(Component)]
pub struct StateIndicator {
    pub character: char,
    timer: f32,
}

impl StateIndicator {
    pub fn new(character: char, duration: f32) -> Self { Self { character, timer: duration } }
}

pub fn tick_state_indicators(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut query: Query<(Entity, &mut StateIndicator)>,
) {
    for (entity, mut ind) in query.iter_mut() {
        ind.timer -= time.delta_secs();
        if ind.timer <= 0.0 {
            commands.entity(entity).remove::<StateIndicator>();
        }
    }
}

pub fn render_state_indicators(
    query: Query<(&Transform, &StateIndicator)>,
    mut egui_context: EguiContexts,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    current_fov: Option<Res<CurrentFovState>>,
) {
    let Ok((camera, camera_transform)) = camera_query.single() else { return };
    let Ok(ctx) = egui_context.ctx_mut() else { return };

    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("state_indicators"),
    ));

    for (transform, indicator) in query.iter() {
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
        // Fade out over the last half-second:
        let alpha = (indicator.timer / 0.5).clamp(0.0, 1.0);
        let screen_pos = egui::Pos2::new(viewport_pos.x, viewport_pos.y - 18.0);
        painter.text(
            screen_pos,
            egui::Align2::CENTER_CENTER,
            indicator.character.to_string(),
            egui::FontId::monospace(16.0),
            egui::Color32::from_black_alpha((alpha * 255.0) as u8),
        );
    }
}
