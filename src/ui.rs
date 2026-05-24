use std::collections::HashMap;

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiGlobalSettings, EguiPlugin, EguiPrimaryContextPass, egui};
use pyri_tooltip::prelude::*;

use geo::Contains;

use crate::{
    DungeonDepth, GameState,
    fov::ExplorationState,
    item::{Inventory, ItemDialogKind, ItemKind, ItemUseDialog, item_display_name},
    player::Player,
};

const BAR_WIDTH: f32 = 140.0;
const BAR_HEIGHT: f32 = 22.0; // 4px taller than egui's default 18px interact_size
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

#[derive(Resource, Default)]
struct UiState {
    messages_expanded: bool,
    menu_open: bool,
}

pub fn enable_ui_input_absorption(mut egui_settings: ResMut<EguiGlobalSettings>) {
    egui_settings.enable_absorb_bevy_input_system = true;
}

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((EguiPlugin::default(), TooltipPlugin::default()))
            .init_resource::<MessageLog>()
            .init_resource::<UiState>()
            .add_systems(EguiPrimaryContextPass, ui_system)
            .add_systems(Update, crate::monster::refresh_monster_tooltips)
            .add_systems(Update, show_world_entity_tooltip);
    }
}

fn ui_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    mut item_dialog: ResMut<ItemUseDialog>,
    player_query: Query<(&crate::monster::Stats, &Inventory, &Transform), With<Player>>,
    staircase_q: Query<&Transform, With<crate::Staircase>>,
    message_log: Res<MessageLog>,
    mut app_exit: MessageWriter<AppExit>,
    mut next_state: ResMut<NextState<GameState>>,
    depth: Res<DungeonDepth>,
) -> Result {
    let ctx = contexts.ctx_mut()?;

    ctx.style_mut(|style| {
        style.interaction.tooltip_delay = 0.0;
        style.interaction.show_tooltips_only_when_still = false;
        style.spacing.tooltip_width = 80.0;
    });

    let Ok((stats, inventory, player_tf)) = player_query.single() else {
        return Ok(());
    };

    let messages_expanded = ui_state.messages_expanded;
    let menu_open = ui_state.menu_open;

    // --- Top bar: most-recent message; full-width click expands log ---
    let top_clicked = egui::TopBottomPanel::top("message_bar")
        .show(ctx, |ui| {
            let latest = message_log.iter().next_back().unwrap_or("—");

            // Full-width clickable row for the summary line.
            let row_height = ui.spacing().interact_size.y;
            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), row_height),
                egui::Sense::click(),
            );
            ui.painter().text(
                rect.left_center() + egui::vec2(6.0, 0.0),
                egui::Align2::LEFT_CENTER,
                latest,
                egui::FontId::default(),
                ui.visuals().text_color(),
            );

            // Expanded log: chronological order, newest at the bottom.
            if messages_expanded {
                egui::ScrollArea::vertical().max_height(200.0).stick_to_bottom(true).show(
                    ui,
                    |ui| {
                        ui.set_min_width(ui.available_width());
                        for msg in message_log.iter() {
                            ui.label(msg);
                        }
                    },
                );
            }

            response.clicked()
        })
        .inner;

    if top_clicked {
        ui_state.messages_expanded = !messages_expanded;
    }

    // Pre-compute inventory counts outside the closure.
    let mut item_counts: Vec<(ItemKind, usize)> = Vec::new();
    for &item in &inventory.0 {
        if let Some(entry) = item_counts.iter_mut().find(|(k, _)| *k == item) {
            entry.1 += 1;
        } else {
            item_counts.push((item, 1));
        }
    }

    let (hp, max_hp, mana, max_mana) = (stats.hp, stats.max_hp, stats.mana, stats.max_mana);

    let near_staircase = if let Ok(stair_tf) = staircase_q.single() {
        player_tf.translation.truncate().distance(stair_tf.translation.truncate()) < 20.0
    } else {
        false
    };

    // --- Bottom bar: HP/mana bars, inventory icons, hamburger ---
    let mut toggle_menu = false;
    let mut do_descend = false;
    egui::TopBottomPanel::bottom("hud").show(ctx, |ui| {
        ui.horizontal(|ui| {
            draw_stat_bar(
                ui,
                hp / max_hp,
                egui::Color32::from_rgb(180, 40, 40),
                &format!("HP {}/{}", hp as i32, max_hp as i32),
            );

            ui.separator();

            draw_stat_bar(
                ui,
                mana / max_mana,
                egui::Color32::from_rgb(40, 80, 200),
                &format!("MP {}/{}", mana as i32, max_mana as i32),
            );

            ui.separator();

            for (item, count) in &item_counts {
                draw_item_icon(ui, *item, *count);
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("☰").clicked() {
                    toggle_menu = true;
                }
                ui.separator();
                ui.label(format!("Depth {}", depth.0));
                ui.separator();
                if ui.add_visible(near_staircase, egui::Button::new("Descend")).clicked() {
                    do_descend = true;
                }
            });
        });
    });

    if toggle_menu {
        ui_state.menu_open = !menu_open;
    }

    // --- Game menu modal ---
    let mut close_menu = false;
    let mut quit = false;
    let mut restart = false;
    if menu_open {
        egui::Window::new("Menu")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_min_width(120.0);
                if ui.button("Restart").clicked() {
                    restart = true;
                }
                ui.add_space(4.0);
                if ui.button("Quit").clicked() {
                    quit = true;
                }
                ui.add_space(4.0);
                if ui.button("Close").clicked() {
                    close_menu = true;
                }
            });
    }

    if close_menu {
        ui_state.menu_open = false;
    }
    if quit {
        app_exit.write(AppExit::Success);
    }
    if restart {
        next_state.set(GameState::Restart);
        ui_state.menu_open = false;
    }

    if do_descend {
        next_state.set(GameState::Descend);
    }

    // --- Item use dialog (quaff / read) ---
    if let Some(dialog_kind) = item_dialog.open {
        let title = match dialog_kind {
            ItemDialogKind::Potions => "Quaff a Potion",
            ItemDialogKind::Scrolls => "Read a Scroll",
        };

        let relevant_items: Vec<(ItemKind, usize)> = item_counts
            .iter()
            .filter(|(k, _)| match dialog_kind {
                ItemDialogKind::Potions => matches!(k, ItemKind::Potion(_)),
                ItemDialogKind::Scrolls => matches!(k, ItemKind::Scroll(_)),
            })
            .copied()
            .collect();

        let mut cancel_clicked = false;
        let mut item_to_use: Option<ItemKind> = None;

        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_min_width(200.0);

                if relevant_items.is_empty() {
                    ui.label("You have none.");
                } else {
                    for (item, count) in &relevant_items {
                        ui.horizontal(|ui| {
                            draw_item_icon(ui, *item, *count);
                            let label = format!("{} (x{})", item_display_name(*item), count);
                            if ui.button(label).clicked() {
                                item_to_use = Some(*item);
                            }
                        });
                    }
                }

                ui.add_space(4.0);
                if ui.button("Cancel").clicked() {
                    cancel_clicked = true;
                }
            });

        if let Some(item) = item_to_use {
            item_dialog.pending_use = Some(item);
        }
        if cancel_clicked {
            item_dialog.open = None;
            item_dialog.pending_use = None;
        }
    }

    Ok(())
}

fn draw_stat_bar(ui: &mut egui::Ui, ratio: f32, color: egui::Color32, label: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(BAR_WIDTH, BAR_HEIGHT), egui::Sense::hover());
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

fn draw_item_icon(ui: &mut egui::Ui, item: ItemKind, count: usize) {
    let size = BAR_HEIGHT; // square icon, same height as the bars
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let center = rect.center();

    painter.rect_filled(rect, 2.0_f32, egui::Color32::from_rgb(40, 40, 40));

    match item {
        ItemKind::Potion(_) => {
            // Upward-pointing triangle, matching the in-game RegularPolygon mesh.
            let r = size * 0.32;
            let pts = vec![
                egui::pos2(center.x, center.y - r),
                egui::pos2(center.x + r * 0.866, center.y + r * 0.5),
                egui::pos2(center.x - r * 0.866, center.y + r * 0.5),
            ];
            painter.add(egui::Shape::convex_polygon(
                pts,
                egui::Color32::from_rgb(51, 217, 76), // matches Color::srgb(0.2, 0.85, 0.3)
                egui::Stroke::NONE,
            ));
        }
        ItemKind::Scroll(_) => {
            // Small square, matching the in-game Rectangle mesh.
            let half = size * 0.28;
            let sq = egui::Rect::from_center_size(center, egui::vec2(half * 2.0, half * 2.0));
            painter.rect_filled(sq, 1.0_f32, egui::Color32::from_rgb(204, 204, 191)); // matches Color::srgb(0.8, 0.8, 0.75)
        }
    }

    if count > 1 {
        painter.text(
            rect.right_bottom() + egui::vec2(-2.0, -2.0),
            egui::Align2::RIGHT_BOTTOM,
            count.to_string(),
            egui::FontId::proportional(10.0),
            egui::Color32::WHITE,
        );
    }

    // Show tooltip on hover
    if response.hovered() {
        response.on_hover_text(item_display_name(item));
    }
}

fn show_world_entity_tooltip(
    mut contexts: EguiContexts,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    entity_query: Query<(&Transform, &WorldTooltip)>,
    windows: Query<&Window>,
    exploration_state: Option<Res<ExplorationState>>,
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
        if let Some(ref exp) = exploration_state {
            if exp.0.contains(&geo::Point::new(pos.x, pos.y)) {
                continue;
            }
        }
        if pos.distance(world_pos) < hover_distance {
            make_egui_tooltop(ctx, egui::Id::new("world_tooltip"), mouse_pos, |ui| {
                ui.label(&tooltip.0);
            });
            return Ok(());
        }
    }

    Ok(())
}

fn make_egui_tooltop(
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
    use bevy::prelude::Entity;

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
