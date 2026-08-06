// egui HUD, built almost entirely from `rogue_angles::hud` primitives: this game has no
// inventory or command palette, so there's very little left that's actually this game's own
// — just the HP bar's numbers, the depth readout, and the win/game-over screens.
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};

use rogue_angles::hud::{
    MessageLog, draw_stat_bar, enable_ui_input_absorption, render_message_bar,
    show_world_entity_tooltip,
};

use crate::{DungeonDepth, GameState, player::Stats};

const BAR_WIDTH: f32 = 160.0;
const BAR_HEIGHT: f32 = 22.0;

/// Exact height of the top message bar.
pub const TOP_PANEL_HEIGHT: f32 = 30.0;
/// Exact height of the bottom HUD bar.
pub const BOTTOM_PANEL_HEIGHT: f32 = 34.0;

#[derive(Resource, Default)]
struct UiState {
    messages_expanded: bool,
    menu_open: bool,
}

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EguiPlugin::default())
            .init_resource::<MessageLog>()
            .init_resource::<UiState>()
            .add_systems(Startup, enable_ui_input_absorption)
            .add_systems(EguiPrimaryContextPass, ui_system)
            .add_systems(Update, show_world_entity_tooltip);
    }
}

fn ui_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    player_query: Query<&Stats, With<crate::player::Player>>,
    message_log: Res<MessageLog>,
    mut app_exit: MessageWriter<AppExit>,
    mut next_state: ResMut<NextState<GameState>>,
    current_game_state: Res<State<GameState>>,
    depth: Res<DungeonDepth>,
) -> Result {
    let ctx = contexts.ctx_mut()?;

    if render_message_bar(ctx, &message_log, ui_state.messages_expanded, TOP_PANEL_HEIGHT) {
        ui_state.messages_expanded = !ui_state.messages_expanded;
    }

    match *current_game_state.get() {
        GameState::GameOver => {
            if render_end_screen(ctx, "You Died", egui::Color32::from_rgb(220, 60, 60)).restart {
                next_state.set(GameState::Restart);
            }
            return Ok(());
        }
        GameState::Won => {
            if render_end_screen(ctx, "You escaped with the Amulet of Winning!", egui::Color32::from_rgb(230, 190, 40))
                .restart
            {
                next_state.set(GameState::Restart);
            }
            return Ok(());
        }
        _ => {}
    }

    let Ok(stats) = player_query.single() else { return Ok(()) };

    let mut toggle_menu = false;
    egui::TopBottomPanel::bottom("hud").exact_height(BOTTOM_PANEL_HEIGHT).show(ctx, |ui| {
        ui.horizontal(|ui| {
            draw_stat_bar(
                ui,
                BAR_WIDTH,
                BAR_HEIGHT,
                stats.hp / stats.max_hp,
                egui::Color32::from_rgb(180, 40, 40),
                &format!("HP {}/{}", stats.hp as i32, stats.max_hp as i32),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("☰").clicked() {
                    toggle_menu = true;
                }
                ui.separator();
                ui.label(format!("Depth {}", depth.0));
            });
        });
    });
    if toggle_menu {
        ui_state.menu_open = !ui_state.menu_open;
    }

    if ui_state.menu_open {
        let mut close = false;
        egui::Window::new("Menu")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_min_width(120.0);
                if ui.button("Restart").clicked() {
                    next_state.set(GameState::Restart);
                    close = true;
                }
                ui.add_space(4.0);
                if ui.button("Quit").clicked() {
                    app_exit.write(AppExit::Success);
                }
                ui.add_space(4.0);
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        if close {
            ui_state.menu_open = false;
        }
    }

    Ok(())
}

#[derive(Default)]
struct EndScreenActions {
    restart: bool,
}

fn render_end_screen(ctx: &egui::Context, headline: &str, color: egui::Color32) -> EndScreenActions {
    let mut actions = EndScreenActions::default();
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(egui::Color32::from_rgba_unmultiplied(30, 30, 30, 200)))
        .show(ctx, |ui| {
            let full_rect = ui.available_rect_before_wrap();
            let mut centered = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(full_rect)
                    .layout(egui::Layout::top_down(egui::Align::Center)),
            );
            let top_pad = (full_rect.height() - 100.0).max(0.0) / 2.0;
            centered.add_space(top_pad);
            centered.label(egui::RichText::new(headline).size(28.0).color(color).strong());
            centered.add_space(16.0);
            if centered.add(egui::Button::new("Restart").min_size(egui::vec2(120.0, 28.0))).clicked() {
                actions.restart = true;
            }
        });
    actions
}
