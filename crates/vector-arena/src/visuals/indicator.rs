// StateIndicator: an egui-drawn character label shown briefly on AI state change.
// egui-based, so it stays in the game crate until the presentation split moves
// UI rendering to the engine behind its own abstraction. HitFlash — the other
// half of the original indicator.rs — has no UI dependency and lives in
// `rogue_angles::visuals::indicator`.
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use rogue_angles::{
    fov::{CurrentFovState, is_currently_visible},
    hud::world_to_screen_pos,
};

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
        if !is_currently_visible(current_fov.as_deref(), pos) {
            continue;
        }
        let Some(mut screen_pos) = world_to_screen_pos(camera, camera_transform, transform.translation)
        else {
            continue;
        };
        screen_pos.y -= 18.0; // draw above the entity, not on top of it
        // Fade out over the last half-second:
        let alpha = (indicator.timer / 0.5).clamp(0.0, 1.0);
        painter.text(
            screen_pos,
            egui::Align2::CENTER_CENTER,
            indicator.character.to_string(),
            egui::FontId::monospace(16.0),
            egui::Color32::from_black_alpha((alpha * 255.0) as u8),
        );
    }
}
