use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use pyri_tooltip::prelude::*;

use avian2d::{diagnostics::ui::PhysicsDiagnosticsUiSettings, prelude::RigidBody};

use rogue_angles::{
    fov::CurrentFovState,
    hud::{MessageLog, draw_stat_bar, show_world_entity_tooltip},
    palette::CommandPaletteState,
};

use crate::{
    DungeonDepth, GameState,
    item::{
        Inventory, ItemIdentities, ItemKind, WAND_COOLDOWN_SECS, WandCooldowns, item_display_name,
    },
    player::{BOREDOM_MAX, Boredom, Player, tick_boredom},
    sprite::SpriteEguiTextures,
    status_effect::StatusEffects,
};

const BAR_WIDTH: f32 = 140.0;
const BAR_HEIGHT: f32 = 22.0; // 4px taller than egui's default 18px interact_size

/// Exact height of the top message bar (content + egui panel frame margins).
pub const TOP_PANEL_HEIGHT: f32 = 30.0;
/// Exact height of the bottom HUD bar; used as `exact_height` so world alignment is guaranteed.
pub const BOTTOM_PANEL_HEIGHT: f32 = 34.0;

#[derive(Resource, Default)]
struct UiState {
    messages_expanded: bool,
    menu_open: bool,
    perf_overlay: bool,
}

struct PerfStats {
    fov_vertices: usize,
    exp_vertices: usize,
    phys_objects: usize,
}

fn count_mp_vertices(mp: &rogue_angles::util::safegeo::SafeMultiPolygon) -> usize {
    mp.iter()
        .flat_map(|p| std::iter::once(p.exterior()).chain(p.interiors()))
        .map(|ls| ls.coords().count())
        .sum()
}

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((EguiPlugin::default(), TooltipPlugin::default()))
            .init_resource::<MessageLog>()
            .init_resource::<UiState>()
            .init_resource::<Boredom>()
            .add_systems(EguiPrimaryContextPass, ui_system)
            .add_systems(EguiPrimaryContextPass, crate::command_palette::palette_system)
            .add_systems(
                EguiPrimaryContextPass,
                crate::command_palette::handle_world_click_for_palette
                    .after(crate::command_palette::palette_system),
            )
            .add_systems(EguiPrimaryContextPass, crate::goto::render_goto_markers)
            .add_systems(EguiPrimaryContextPass, crate::monster::render_monster_markers)
            .add_systems(Update, crate::monster::refresh_monster_tooltips)
            .add_systems(Update, show_world_entity_tooltip)
            .add_systems(Update, tick_boredom.run_if(in_state(GameState::InLevel)))
            .add_systems(Update, toggle_perf_overlay);
    }
}

fn toggle_perf_overlay(
    keys: Res<ButtonInput<KeyCode>>,
    mut ui_state: ResMut<UiState>,
    mut physics_diag: ResMut<PhysicsDiagnosticsUiSettings>,
) {
    if keys.just_pressed(KeyCode::KeyP) && keys.pressed(KeyCode::ControlLeft) {
        ui_state.perf_overlay = !ui_state.perf_overlay;
        physics_diag.enabled = ui_state.perf_overlay;
    }
}

fn ui_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    player_query: Query<
        (&crate::monster::Stats, &Inventory, &Transform, Option<&StatusEffects>),
        With<Player>,
    >,
    staircase_q: Query<&Transform, With<crate::Staircase>>,
    message_log: Res<MessageLog>,
    mut app_exit: MessageWriter<AppExit>,
    mut next_state: ResMut<NextState<GameState>>,
    current_game_state: Res<State<GameState>>,
    depth: Res<DungeonDepth>,
    sprite_textures: Res<SpriteEguiTextures>,
    mut identities: ResMut<ItemIdentities>,
    wand_cooldowns: Res<WandCooldowns>,
    mut palette: ResMut<CommandPaletteState>,
    boredom: Res<Boredom>,
    current_fov: Option<Res<CurrentFovState>>,
    rigid_body_query: Query<(), With<RigidBody>>,
) -> Result {
    let ctx = contexts.ctx_mut()?;

    ctx.style_mut(|style| {
        style.interaction.tooltip_delay = 0.0;
        style.interaction.show_tooltips_only_when_still = false;
        style.spacing.tooltip_width = 200.0;
    });

    let Ok((stats, inventory, player_tf, player_effects)) = player_query.single() else {
        return Ok(());
    };

    let perf_stats = if ui_state.perf_overlay {
        Some(PerfStats {
            fov_vertices: current_fov.as_ref().map(|f| count_mp_vertices(&f.0)).unwrap_or(0),
            exp_vertices: 0,
            phys_objects: rigid_body_query.iter().count(),
        })
    } else {
        None
    };

    if render_message_bar(ctx, &message_log, ui_state.messages_expanded, perf_stats.as_ref()) {
        ui_state.messages_expanded = !ui_state.messages_expanded;
    }

    if *current_game_state.get() == GameState::GameOver {
        let result = render_game_over(ctx);
        if result.open_inventory {
            for &item in &inventory.0 {
                identities.identify(item);
            }
            palette.open = true;
            palette.input = "e ".to_string();
            palette.selected_idx = 0;
        }
        if result.restart {
            next_state.set(GameState::Restart);
        }
        return Ok(());
    }

    let item_counts = collect_item_counts(inventory);

    let near_staircase = if let Ok(stair_tf) = staircase_q.single() {
        player_tf.translation.truncate().distance(stair_tf.translation.truncate()) < 20.0
    } else {
        false
    };

    let hud = render_hud(
        ctx,
        stats,
        &item_counts,
        player_effects,
        depth.0,
        near_staircase,
        &sprite_textures,
        &identities,
        &wand_cooldowns,
        &boredom,
    );
    if hud.toggle_menu {
        ui_state.menu_open = !ui_state.menu_open;
    }
    if hud.descend {
        next_state.set(GameState::Descend);
    }

    if ui_state.menu_open {
        let menu = render_menu(ctx);
        if menu.close {
            ui_state.menu_open = false;
        }
        if menu.quit {
            app_exit.write(AppExit::Success);
        }
        if menu.restart {
            next_state.set(GameState::Restart);
            ui_state.menu_open = false;
        }
    }

    Ok(())
}

#[derive(Default)]
struct GameOverActions {
    open_inventory: bool,
    restart: bool,
}

fn render_game_over(ctx: &egui::Context) -> GameOverActions {
    let mut actions = GameOverActions::default();

    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(egui::Color32::from_rgba_unmultiplied(30, 30, 30, 200)))
        .show(ctx, |ui| {
            let full_rect = ui.available_rect_before_wrap();
            let mut centered = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(full_rect)
                    .layout(egui::Layout::top_down(egui::Align::Center)),
            );

            let content_height = 120.0;
            let top_pad = (full_rect.height() - content_height).max(0.0) / 2.0;
            centered.add_space(top_pad);

            centered.label(
                egui::RichText::new("Game Over")
                    .size(36.0)
                    .color(egui::Color32::from_rgb(220, 60, 60))
                    .strong(),
            );
            centered.add_space(16.0);

            if centered
                .add(egui::Button::new("Restart").min_size(egui::vec2(120.0, 28.0)))
                .clicked()
            {
                actions.restart = true;
            }
            centered.add_space(8.0);

            if centered
                .add(egui::Button::new("View Inventory").min_size(egui::vec2(120.0, 28.0)))
                .clicked()
            {
                actions.open_inventory = true;
            }
        });

    actions
}

/// Top bar: latest message with optional expanded log. Returns true if the row was clicked.
/// When `perf` is Some, the top bar shows performance stats instead of the message log.
fn render_message_bar(
    ctx: &egui::Context,
    log: &MessageLog,
    expanded: bool,
    perf: Option<&PerfStats>,
) -> bool {
    let panel = egui::TopBottomPanel::top("message_bar");
    // Lock the panel to an exact height when collapsed so the level edge aligns precisely.
    // When expanded, let the panel grow naturally to show the full log.
    let panel = if expanded { panel } else { panel.exact_height(TOP_PANEL_HEIGHT) };
    // In perf mode suppress the panel's own background so we can paint only the right half.
    let panel_fill = ctx.style().visuals.panel_fill;
    let panel = if perf.is_some() { panel.frame(egui::Frame::new()) } else { panel };
    panel
        .show(ctx, |ui| {
            let row_height = ui.spacing().interact_size.y;
            let full_width = ui.available_width();
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(full_width, row_height), egui::Sense::click());

            if let Some(p) = perf {
                // Paint background only on the right half; left half stays transparent.
                let text_rect = egui::Rect::from_min_size(
                    egui::pos2(rect.min.x + full_width / 2.0, rect.min.y),
                    egui::vec2(full_width / 2.0, rect.height()),
                );
                ui.painter().rect_filled(text_rect, 0.0, panel_fill);
                let text = format!(
                    "fov v:{}  exp v:{}  phys o:{}",
                    p.fov_vertices, p.exp_vertices, p.phys_objects
                );
                ui.painter().text(
                    text_rect.left_center() + egui::vec2(6.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    &text,
                    egui::FontId::default(),
                    ui.visuals().text_color(),
                );
            } else {
                let latest = log.iter().next_back().unwrap_or("—");
                ui.painter().text(
                    rect.left_center() + egui::vec2(6.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    latest,
                    egui::FontId::default(),
                    ui.visuals().text_color(),
                );

                if expanded {
                    egui::ScrollArea::vertical().max_height(200.0).stick_to_bottom(true).show(
                        ui,
                        |ui| {
                            ui.set_min_width(ui.available_width());
                            for msg in log.iter() {
                                ui.label(msg);
                            }
                        },
                    );
                }
            }

            response.clicked()
        })
        .inner
}

fn collect_item_counts(inventory: &Inventory) -> Vec<(ItemKind, u16)> {
    let mut counts: Vec<(ItemKind, u16)> = Vec::new();
    for &item in &inventory.0 {
        if let Some(entry) = counts.iter_mut().find(|(k, _)| *k == item) {
            entry.1 += 1;
        } else {
            counts.push((item, 1));
        }
    }
    counts
}

#[derive(Default)]
struct HudActions {
    toggle_menu: bool,
    descend: bool,
}

/// Bottom bar: HP/MP bars, inventory icons, depth, descend button, hamburger.
fn render_hud(
    ctx: &egui::Context,
    stats: &crate::monster::Stats,
    item_counts: &[(ItemKind, u16)],
    effects: Option<&StatusEffects>,
    depth: u32,
    near_staircase: bool,
    sprite_textures: &SpriteEguiTextures,
    identities: &ItemIdentities,
    wand_cooldowns: &WandCooldowns,
    boredom: &Boredom,
) -> HudActions {
    let mut actions = HudActions::default();
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

            ui.separator();

            draw_stat_bar(
                ui,
                BAR_WIDTH,
                BAR_HEIGHT,
                stats.mana / stats.max_mana,
                egui::Color32::from_rgb(40, 80, 200),
                &format!("MP {}/{}", stats.mana as i32, stats.max_mana as i32),
            );

            ui.separator();

            draw_stat_bar(
                ui,
                BAR_WIDTH,
                BAR_HEIGHT,
                boredom.seconds / BOREDOM_MAX,
                egui::Color32::from_rgb(110, 110, 110),
                &format!("Boredom: {}s", boredom.seconds as u32),
            );

            if let Some(effects) = effects {
                for e in &effects.0 {
                    ui.separator();
                    let (r, g, b) = e.kind.color_rgb();
                    let secs = e.remaining.ceil() as u32;
                    ui.colored_label(
                        egui::Color32::from_rgb(r, g, b),
                        format!("{} {}s", e.kind.label(), secs),
                    );
                }
            }

            ui.separator();

            for (item, count) in item_counts {
                draw_item_icon(ui, *item, *count, sprite_textures, identities, wand_cooldowns);
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("☰").clicked() {
                    actions.toggle_menu = true;
                }
                ui.separator();
                ui.label(format!("Depth {}", depth));
                ui.separator();
                if ui.add_visible(near_staircase, egui::Button::new("Descend")).clicked() {
                    actions.descend = true;
                }
            });
        });
    });
    actions
}

#[derive(Default)]
struct MenuActions {
    close: bool,
    quit: bool,
    restart: bool,
}

fn render_menu(ctx: &egui::Context) -> MenuActions {
    let mut actions = MenuActions::default();
    egui::Window::new("Menu")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_min_width(120.0);
            if ui.button("Restart").clicked() {
                actions.restart = true;
            }
            ui.add_space(4.0);
            if ui.button("Quit").clicked() {
                actions.quit = true;
            }
            ui.add_space(4.0);
            if ui.button("Close").clicked() {
                actions.close = true;
            }
        });
    actions
}

/// Draw the item shape into an exact rect (shared by HUD and palette).
/// Uses `texture` (SVG-rasterized) when available, otherwise falls back to primitives.
pub fn draw_item_icon_at(
    painter: egui::Painter,
    rect: egui::Rect,
    item: ItemKind,
    texture: Option<&egui::TextureHandle>,
) {
    painter.rect_filled(rect, 2.0_f32, egui::Color32::from_rgb(40, 40, 40));
    if let Some(tex) = texture {
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        painter.image(tex.id(), rect, uv, egui::Color32::WHITE);
        return;
    }
    let center = rect.center();
    let size = rect.width().min(rect.height());
    match item {
        ItemKind::Potion(_) => {
            let r = size * 0.32;
            let pts = vec![
                egui::pos2(center.x, center.y - r),
                egui::pos2(center.x + r * 0.866, center.y + r * 0.5),
                egui::pos2(center.x - r * 0.866, center.y + r * 0.5),
            ];
            painter.add(egui::Shape::convex_polygon(
                pts,
                egui::Color32::from_rgb(51, 217, 76),
                egui::Stroke::NONE,
            ));
        }
        ItemKind::Scroll(_) => {
            let half = size * 0.28;
            let sq = egui::Rect::from_center_size(center, egui::vec2(half * 2.0, half * 2.0));
            painter.rect_filled(sq, 1.0_f32, egui::Color32::from_rgb(204, 204, 191));
        }
        ItemKind::Wand(_) => {
            let r = size * 0.25;
            painter.circle_filled(center, r, egui::Color32::from_rgb(180, 100, 40));
        }
    }
}

fn draw_item_icon(
    ui: &mut egui::Ui,
    item: ItemKind,
    count: u16,
    sprite_textures: &SpriteEguiTextures,
    identities: &ItemIdentities,
    wand_cooldowns: &WandCooldowns,
) {
    let size = BAR_HEIGHT;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    draw_item_icon_at(ui.painter_at(rect), rect, item, sprite_textures.get(item));

    match item {
        ItemKind::Wand(gem) => {
            let ready = wand_cooldowns.ready_count(gem, count);
            if ready >= 2 {
                ui.painter().text(
                    rect.right_bottom() + egui::vec2(-2.0, -2.0),
                    egui::Align2::RIGHT_BOTTOM,
                    ready.to_string(),
                    egui::FontId::proportional(10.0),
                    egui::Color32::WHITE,
                );
            } else if ready == 0 {
                if let Some(remaining) = wand_cooldowns.shortest_remaining(gem) {
                    draw_cooldown_arc(ui.painter_at(rect), rect, remaining / WAND_COOLDOWN_SECS);
                }
            }
            // ready == 1: no badge
        }
        _ => {
            if count > 1 {
                ui.painter().text(
                    rect.right_bottom() + egui::vec2(-2.0, -2.0),
                    egui::Align2::RIGHT_BOTTOM,
                    count.to_string(),
                    egui::FontId::proportional(10.0),
                    egui::Color32::WHITE,
                );
            }
        }
    }

    if response.hovered() {
        response.on_hover_text(item_display_name(item, count, identities));
    }
}

fn draw_cooldown_arc(painter: egui::Painter, rect: egui::Rect, fraction: f32) {
    // Filled pie-slice in the bottom-right corner, same position as the count badge.
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

