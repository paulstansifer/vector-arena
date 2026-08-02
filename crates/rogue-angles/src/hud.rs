// Generic HUD chrome: a scrolling message log, proximity-based world tooltips, and a
// labeled progress-bar primitive for stat displays. None of this knows what a game's stats,
// items, or messages actually mean — it's just the reusable shape every HUD in this kind of
// game ends up needing. The demo's stat/inventory panels (HP/MP bars, item icons, depth,
// menu) stay entirely game-side; this module only supplies the pieces under them.
use std::collections::HashMap;

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiGlobalSettings, egui};
use geo::Contains;

use crate::fov::CurrentFovState;

const BAR_ROUNDING: u8 = 3;

#[derive(Component, Default)]
pub struct WorldTooltip(pub String);

#[derive(Resource, Default)]
pub struct MessageLog {
    // Empty strings are tombstones for entries that were moved to the end.
    messages: Vec<String>,
    // (prefix, entity) -> (repeat_count, index into messages)
    repeating: HashMap<(String, Entity), (usize, usize)>,
}

impl MessageLog {
    pub fn push(&mut self, msg: impl Into<String>) { self.messages.push(msg.into()); }

    /// Push a collapsible message. Repeated calls with the same `prefix`+`entity`
    /// key move the entry to the end and show a repeat count: `"{prefix} (3x){suffix}"`.
    pub fn push_repeating(
        &mut self,
        prefix: impl Into<String>,
        entity: Entity,
        suffix: impl Into<String>,
    ) {
        let prefix = prefix.into();
        let suffix = suffix.into();
        let key = (prefix.clone(), entity);
        if let Some((count, idx)) = self.repeating.get_mut(&key) {
            *count += 1;
            let c = *count;
            self.messages[*idx] = String::new(); // tombstone old slot
            *idx = self.messages.len();
            self.messages.push(format!("{prefix} ({c}x){suffix}"));
        } else {
            let idx = self.messages.len();
            self.messages.push(format!("{prefix}{suffix}"));
            self.repeating.insert(key, (1, idx));
        }
    }

    pub fn clear(&mut self) {
        self.messages.clear();
        self.repeating.clear();
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &str> {
        self.messages.iter().filter(|s| !s.is_empty()).map(|s| s.as_str())
    }
}

pub fn enable_ui_input_absorption(mut egui_settings: ResMut<EguiGlobalSettings>) {
    egui_settings.enable_absorb_bevy_input_system = true;
}

/// A labeled progress bar (rounded rect fill + centered text) — HP/MP/boredom-style stat
/// display. `ratio` is clamped to [0, 1]; `color` is the fill color at full width.
pub fn draw_stat_bar(ui: &mut egui::Ui, width: f32, height: f32, ratio: f32, color: egui::Color32, label: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, BAR_ROUNDING, egui::Color32::from_rgb(40, 40, 40));

    let clamped = ratio.clamp(0.0, 1.0);
    if clamped > 0.0 {
        let fill_w = rect.width() * clamped;
        let fill_rect = egui::Rect::from_min_size(rect.min, egui::vec2(fill_w, rect.height()));
        let rounding = if clamped >= 1.0 {
            egui::CornerRadius::same(BAR_ROUNDING)
        } else {
            egui::CornerRadius { nw: BAR_ROUNDING, sw: BAR_ROUNDING, ne: 0, se: 0 }
        };
        painter.rect_filled(fill_rect, rounding, color);
    }

    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::default(),
        egui::Color32::WHITE,
    );
}

/// Shows a floating tooltip near the cursor for any `WorldTooltip`-carrying entity within
/// `hover_distance` of the mouse's world position — and currently visible per
/// `fov::CurrentFovState`, if present. Skips entirely if there's no window, camera, or cursor.
pub fn show_world_entity_tooltip(
    mut contexts: EguiContexts,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    entity_query: Query<(&Transform, &WorldTooltip)>,
    windows: Query<&Window>,
    current_fov: Option<Res<CurrentFovState>>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    let window = match windows.single() {
        Ok(w) => w,
        Err(_) => return Ok(()),
    };

    let Some(mouse_pos) = window.cursor_position() else {
        return Ok(());
    };

    let (camera, camera_transform) = match camera_query.single() {
        Ok(cam) => cam,
        Err(_) => return Ok(()),
    };

    let world_pos = match camera.viewport_to_world_2d(camera_transform, mouse_pos) {
        Ok(pos) => pos,
        Err(_) => return Ok(()),
    };

    let hover_distance = 20.0;
    for (transform, tooltip) in entity_query.iter() {
        let pos = transform.translation.truncate();
        if pos.distance(world_pos) < hover_distance {
            if let Some(ref fov) = current_fov
                && !fov.0.contains(&geo::Point::new(pos.x, pos.y))
            {
                continue;
            }
            make_egui_tooltip(ctx, egui::Id::new(("world_tooltip", tooltip.0.as_str())), mouse_pos, |ui| {
                ui.label(&tooltip.0);
            });
            return Ok(());
        }
    }

    Ok(())
}

fn make_egui_tooltip(
    ctx: &egui::Context,
    id: egui::Id,
    cursor_pos: Vec2,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    egui::Area::new(id)
        .kind(egui::UiKind::Popup)
        .order(egui::Order::Tooltip)
        .pivot(egui::Align2::RIGHT_BOTTOM)
        .fixed_pos(egui::pos2(cursor_pos.x - 4.0, cursor_pos.y - 4.0))
        .default_width(ctx.style().spacing.tooltip_width)
        .sense(egui::Sense::hover())
        .show(ctx, |ui| {
            egui::Frame::popup(&ctx.style()).show(ui, add_contents);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(n: u32) -> Entity { Entity::from_raw_u32(n).unwrap() }

    #[test]
    fn test_push_repeating_first_call_no_count() {
        let mut log = MessageLog::default();
        let e = entity(1);
        log.push_repeating("hit", e, " for 5");
        let msgs: Vec<&str> = log.iter().collect();
        assert_eq!(msgs, vec!["hit for 5"]);
    }

    #[test]
    fn test_push_repeating_second_call_shows_count() {
        let mut log = MessageLog::default();
        let e = entity(1);
        log.push_repeating("hit", e, " for 5");
        log.push_repeating("hit", e, " for 5");
        let msgs: Vec<&str> = log.iter().collect();
        assert_eq!(msgs, vec!["hit (2x) for 5"]);
    }

    #[test]
    fn test_push_repeating_moves_entry_to_end() {
        let mut log = MessageLog::default();
        let e = entity(1);
        log.push("first");
        log.push_repeating("hit", e, "!");
        log.push("middle");
        log.push_repeating("hit", e, "!");
        let msgs: Vec<&str> = log.iter().collect();
        assert_eq!(msgs, vec!["first", "middle", "hit (2x)!"]);
    }

    #[test]
    fn test_push_repeating_different_entities_not_collapsed() {
        let mut log = MessageLog::default();
        let e1 = entity(1);
        let e2 = entity(2);
        log.push_repeating("hit", e1, "!");
        log.push_repeating("hit", e2, "!");
        assert_eq!(log.iter().count(), 2);
    }

    #[test]
    fn test_iter_skips_tombstones() {
        let mut log = MessageLog::default();
        let e = entity(1);
        log.push("a");
        log.push_repeating("x", e, "");
        log.push("b");
        log.push_repeating("x", e, ""); // creates tombstone at index 1
        // "a", tombstone, "b", "x (2x)" → iter should yield 3 live entries
        assert_eq!(log.iter().count(), 3);
    }
}
