use std::collections::HashMap;

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiGlobalSettings, EguiPlugin, EguiPrimaryContextPass, egui};
use pyri_tooltip::prelude::*;

use geo::Contains;

use crate::{
    DungeonDepth, GameState,
    fov::ExplorationState,
    item::{Inventory, ItemKind, item_name},
    player::Player,
    sprite::SpriteEguiTextures,
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
            .add_systems(EguiPrimaryContextPass, crate::command_palette::palette_system)
            .add_systems(Update, crate::monster::refresh_monster_tooltips)
            .add_systems(Update, show_world_entity_tooltip);
    }
}

fn ui_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    player_query: Query<(&crate::monster::Stats, &Inventory, &Transform), With<Player>>,
    staircase_q: Query<&Transform, With<crate::Staircase>>,
    message_log: Res<MessageLog>,
    mut app_exit: MessageWriter<AppExit>,
    mut next_state: ResMut<NextState<GameState>>,
    depth: Res<DungeonDepth>,
    sprite_textures: Res<SpriteEguiTextures>,
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

    if render_message_bar(ctx, &message_log, ui_state.messages_expanded) {
        ui_state.messages_expanded = !ui_state.messages_expanded;
    }

    let item_counts = collect_item_counts(inventory);

    let near_staircase = if let Ok(stair_tf) = staircase_q.single() {
        player_tf.translation.truncate().distance(stair_tf.translation.truncate()) < 20.0
    } else {
        false
    };

    let hud = render_hud(ctx, stats, &item_counts, depth.0, near_staircase, &sprite_textures);
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

/// Top bar: latest message with optional expanded log. Returns true if the row was clicked.
fn render_message_bar(ctx: &egui::Context, log: &MessageLog, expanded: bool) -> bool {
    egui::TopBottomPanel::top("message_bar")
        .show(ctx, |ui| {
            let latest = log.iter().next_back().unwrap_or("—");

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
    depth: u32,
    near_staircase: bool,
    sprite_textures: &SpriteEguiTextures,
) -> HudActions {
    let mut actions = HudActions::default();
    egui::TopBottomPanel::bottom("hud").show(ctx, |ui| {
        ui.horizontal(|ui| {
            draw_stat_bar(
                ui,
                stats.hp / stats.max_hp,
                egui::Color32::from_rgb(180, 40, 40),
                &format!("HP {}/{}", stats.hp as i32, stats.max_hp as i32),
            );

            ui.separator();

            draw_stat_bar(
                ui,
                stats.mana / stats.max_mana,
                egui::Color32::from_rgb(40, 80, 200),
                &format!("MP {}/{}", stats.mana as i32, stats.max_mana as i32),
            );

            ui.separator();

            for (item, count) in item_counts {
                draw_item_icon(ui, *item, *count, sprite_textures);
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
    }
}

fn draw_item_icon(ui: &mut egui::Ui, item: ItemKind, count: u16, sprite_textures: &SpriteEguiTextures) {
    let size = BAR_HEIGHT;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    draw_item_icon_at(ui.painter_at(rect), rect, item, sprite_textures.get(item));

    if count > 1 {
        ui.painter().text(
            rect.right_bottom() + egui::vec2(-2.0, -2.0),
            egui::Align2::RIGHT_BOTTOM,
            count.to_string(),
            egui::FontId::proportional(10.0),
            egui::Color32::WHITE,
        );
    }

    if response.hovered() {
        response.on_hover_text(item_name(item, count));
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
            make_egui_tooltip(ctx, egui::Id::new("world_tooltip"), mouse_pos, |ui| {
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
