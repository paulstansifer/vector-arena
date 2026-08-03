// Generic HUD chrome: a scrolling message log with its top-bar renderer, proximity-based
// world tooltips, and two drawing primitives — a labeled progress bar and an icon-slot with
// badge/cooldown/tooltip chrome. None of this knows what a game's stats, items, or messages
// actually mean — it's just the reusable shape every HUD in this kind of game ends up
// needing. The demo's bottom stat/inventory bar (HP/MP bars, item iteration, depth, menu)
// stays entirely game-side; this module only supplies the pieces it's built from.
use std::collections::HashMap;

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiGlobalSettings, egui};

use crate::fov::{CurrentFovState, is_currently_visible};

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

/// A collapsible top message bar: the latest pushed message when collapsed, the full
/// scrolling log when `expanded`. Locks to `collapsed_height` when collapsed so the level
/// edge below it aligns precisely; grows to fit when expanded. Stateless — returns `true` if
/// the row was clicked, and the game decides what that means (typically toggling `expanded`
/// for next frame).
pub fn render_message_bar(
    ctx: &egui::Context,
    log: &MessageLog,
    expanded: bool,
    collapsed_height: f32,
) -> bool {
    let panel = egui::TopBottomPanel::top("message_bar");
    let panel = if expanded { panel } else { panel.exact_height(collapsed_height) };
    panel
        .show(ctx, |ui| {
            let row_height = ui.spacing().interact_size.y;
            let full_width = ui.available_width();
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(full_width, row_height), egui::Sense::click());

            let latest = log.iter().next_back().unwrap_or("—");
            ui.painter().text(
                rect.left_center() + egui::vec2(6.0, 0.0),
                egui::Align2::LEFT_CENTER,
                latest,
                egui::FontId::default(),
                ui.visuals().text_color(),
            );

            if expanded {
                egui::ScrollArea::vertical().max_height(200.0).stick_to_bottom(true).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    for msg in log.iter() {
                        ui.label(msg);
                    }
                });
            }

            response.clicked()
        })
        .inner
}

/// One "icon slot" in a row or grid of item-like things (inventory, ability belt, ...): a
/// fixed-size square that the caller draws into via `draw_icon`, with an optional bottom-right
/// text badge, an optional cooldown-remaining pie-slice overlay, and a hover tooltip. The
/// engine only draws this chrome — it has no idea what's actually in the square.
pub fn draw_icon_slot(
    ui: &mut egui::Ui,
    size: f32,
    draw_icon: impl FnOnce(egui::Painter, egui::Rect),
    badge: Option<&str>,
    cooldown_remaining: Option<f32>,
    tooltip: Option<&str>,
) -> egui::Response {
    let (rect, mut response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    draw_icon(ui.painter_at(rect), rect);

    if let Some(frac) = cooldown_remaining
        && frac > 0.0
    {
        draw_cooldown_arc(ui.painter_at(rect), rect, frac);
    }

    if let Some(badge) = badge {
        ui.painter().text(
            rect.right_bottom() + egui::vec2(-2.0, -2.0),
            egui::Align2::RIGHT_BOTTOM,
            badge,
            egui::FontId::proportional(10.0),
            egui::Color32::WHITE,
        );
    }

    if let Some(tip) = tooltip {
        response = response.on_hover_text(tip);
    }

    response
}

/// Filled pie-slice arc in the bottom-right corner of `rect`, sweeping clockwise from the top
/// as `fraction` grows from 0 to 1. Used by `draw_icon_slot` for cooldown countdowns.
fn draw_cooldown_arc(painter: egui::Painter, rect: egui::Rect, fraction: f32) {
    let r = 5.0_f32;
    let center = rect.right_bottom() + egui::vec2(-r - 2.0, -r - 2.0);
    let n = 24usize;
    let span = std::f32::consts::TAU * fraction.clamp(0.0, 1.0);
    let start = -std::f32::consts::FRAC_PI_2;
    let mut points = vec![center];
    points.extend((0..=n).map(|i| {
        let a = start + span * (i as f32 / n as f32);
        egui::pos2(center.x + r * a.cos(), center.y + r * a.sin())
    }));
    painter.add(egui::Shape::Path(egui::epaint::PathShape {
        points,
        closed: true,
        fill: egui::Color32::from_rgba_unmultiplied(255, 255, 255, 200),
        stroke: egui::epaint::PathStroke::NONE,
    }));
}

/// Projects `world_pos` through `camera` into an egui screen position ready to draw an overlay
/// marker/label at — `None` if the point is behind the camera, or lands off the top/left of the
/// viewport (a negative coordinate; bevy's projection doesn't itself distinguish "off-screen"
/// from "valid but negative," so overlays that don't want to draw off-canvas need this check).
/// The common shape behind "draw a marker over this world entity" — every such overlay in this
/// kind of game ends up wanting a screen position, not a raw `Vec2`.
pub fn world_to_screen_pos(
    camera: &Camera,
    camera_transform: &GlobalTransform,
    world_pos: Vec3,
) -> Option<egui::Pos2> {
    let vp = camera.world_to_viewport(camera_transform, world_pos).ok()?;
    (vp.x >= 0.0 && vp.y >= 0.0).then(|| egui::Pos2::new(vp.x, vp.y))
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
            if !is_currently_visible(current_fov.as_deref(), pos) {
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
