use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiGlobalSettings, EguiPlugin, EguiPrimaryContextPass, egui};

use crate::{
    DungeonDepth, GameState,
    item::{Inventory, ItemKind},
    player::Player,
};

const BAR_WIDTH: f32 = 140.0;
const BAR_HEIGHT: f32 = 22.0; // 4px taller than egui's default 18px interact_size
const BAR_ROUNDING: u8 = 3;

#[derive(Component, Default, Clone, Copy)]
pub struct PlayerStats {
    pub hp: f32,
    pub max_hp: f32,
    pub mana: f32,
    pub max_mana: f32,
}

#[derive(Resource, Default)]
pub struct MessageLog {
    pub messages: Vec<String>,
}

impl MessageLog {
    pub fn push(&mut self, msg: impl Into<String>) { self.messages.push(msg.into()); }
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
        app.add_plugins(EguiPlugin::default())
            .init_resource::<MessageLog>()
            .init_resource::<UiState>()
            .add_systems(EguiPrimaryContextPass, ui_system);
    }
}

fn ui_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    player_query: Query<(&PlayerStats, &Inventory, &Transform), With<Player>>,
    staircase_q: Query<&Transform, With<crate::Staircase>>,
    message_log: Res<MessageLog>,
    mut app_exit: MessageWriter<AppExit>,
    mut next_state: ResMut<NextState<GameState>>,
    depth: Res<DungeonDepth>,
) -> Result {
    let ctx = contexts.ctx_mut()?;

    let Ok((stats, inventory, player_tf)) = player_query.single() else {
        return Ok(());
    };

    let messages_expanded = ui_state.messages_expanded;
    let menu_open = ui_state.menu_open;

    // --- Top bar: most-recent message; full-width click expands log ---
    let top_clicked = egui::TopBottomPanel::top("message_bar")
        .show(ctx, |ui| {
            let latest = message_log.messages.last().map(|s| s.as_str()).unwrap_or("—");

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
                        for msg in message_log.messages.iter() {
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
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
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
}
