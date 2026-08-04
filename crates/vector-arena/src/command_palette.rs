// The command palette's egui rendering and click bridging. The state machine
// itself — typing, navigation, target resolution, SystemId dispatch — lives
// entirely in `rogue_angles::palette`; this file only draws it and forwards
// world clicks, since both need a UI framework the engine doesn't depend on.
use bevy::prelude::*;
use bevy_egui::{egui, egui::PointerButton};

use rogue_angles::palette::{
    self, CommandPaletteState, CommandPaletteWatchesClicks, CurrentPaletteEntries, EntryOutcome,
    PaletteEntry, Target,
};

use crate::{item::item_for_icon_id, sprite::SpriteEguiTextures};

pub struct CommandPalettePlugin;

impl Plugin for CommandPalettePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CommandPaletteState>()
            .init_resource::<CommandPaletteWatchesClicks>()
            .init_resource::<CurrentPaletteEntries>()
            .init_resource::<palette::PaletteRegistry>()
            .init_resource::<palette::EntityLabels>()
            .init_resource::<palette::DefaultEntityAction>()
            .add_systems(Update, palette::assign_entity_labels)
            .add_systems(Update, palette::release_entity_labels)
            .add_systems(Update, palette::close_on_escape)
            .add_systems(Update, palette::handle_palette_keyboard.after(palette::close_on_escape))
            .add_systems(
                Update,
                palette::update_palette_entries.after(palette::handle_palette_keyboard),
            )
            .add_systems(Update, palette::update_watches_clicks.after(palette::update_palette_entries));
    }
}

fn farthest_corner_anchor(player_screen: Vec2, screen_size: Vec2) -> egui::Align2 {
    match (player_screen.x > screen_size.x / 2.0, player_screen.y > screen_size.y / 2.0) {
        (false, false) => egui::Align2::RIGHT_BOTTOM,
        (true, false) => egui::Align2::LEFT_BOTTOM,
        (false, true) => egui::Align2::RIGHT_TOP,
        (true, true) => egui::Align2::LEFT_TOP,
    }
}

fn palette_row_label(key: &str, description: &str, is_complete: bool, is_selected: bool) -> egui::text::LayoutJob {
    let suffix = if is_complete { if is_selected { "[ENTER]" } else { "       " } } else { "" };
    let mut job = egui::text::LayoutJob::default();
    job.append(&format!("{key:<5}{suffix}"), 0.0, egui::text::TextFormat {
        font_id: egui::FontId::monospace(13.0),
        color: egui::Color32::WHITE,
        ..Default::default()
    });
    job.append(description, 6.0, egui::text::TextFormat {
        font_id: egui::FontId::proportional(13.0),
        color: egui::Color32::WHITE,
        italics: true,
        ..Default::default()
    });
    job
}

/// Draws the palette window (input line + completion rows) and forwards row
/// clicks into the engine's state machine via a queued command.
pub fn palette_system(
    mut contexts: bevy_egui::EguiContexts,
    palette: Res<CommandPaletteState>,
    watches_clicks: Res<CommandPaletteWatchesClicks>,
    entries: Res<CurrentPaletteEntries>,
    player_query: Query<&Transform, With<crate::player::Player>>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    sprite_textures: Res<SpriteEguiTextures>,
    mut commands: Commands,
) -> Result {
    if !palette.open {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;

    let Ok(player_tf) = player_query.single() else { return Ok(()) };

    let screen_rect = ctx.viewport_rect();
    let screen_size = Vec2::new(screen_rect.width(), screen_rect.height());
    let anchor = camera_query
        .single()
        .ok()
        .and_then(|(cam, tf)| cam.world_to_viewport(tf, player_tf.translation).ok())
        .map(|p| farthest_corner_anchor(p, screen_size))
        .unwrap_or(egui::Align2::RIGHT_BOTTOM);

    let frame = egui::Frame::window(&ctx.style())
        .fill(egui::Color32::from_rgb(20, 20, 30))
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(70, 70, 110)));

    let mut clicked_entry: Option<PaletteEntry> = None;

    egui::Window::new("##cmd_palette")
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .anchor(anchor, [8.0, 8.0])
        .min_width(280.0)
        .frame(frame)
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new(if palette.input.is_empty() {
                    "type a command…".to_string()
                } else {
                    palette.input.clone()
                })
                .monospace()
                .color(egui::Color32::WHITE),
            );
            ui.separator();

            if watches_clicks.0 {
                ui.label(
                    egui::RichText::new("select an onscreen target letter or click to target")
                        .italics()
                        .color(egui::Color32::from_rgb(140, 140, 170))
                        .size(11.0),
                );
                return;
            }

            if entries.0.is_empty() {
                ui.label(
                    egui::RichText::new("nothing applicable").italics().color(egui::Color32::WHITE),
                );
                return;
            }

            let selected_key = palette.input.trim();
            let row_height = 26.0;

            for entry in &entries.0 {
                let is_selected = entry.key == selected_key;
                let is_complete =
                    matches!(entry.outcome, EntryOutcome::Run | EntryOutcome::RunTarget { .. });
                let row_size = egui::vec2(ui.available_width(), row_height);
                let (row_rect, _) = ui.allocate_exact_size(row_size, egui::Sense::hover());

                if is_selected {
                    ui.painter().rect_filled(
                        row_rect,
                        0.0_f32,
                        egui::Color32::from_rgb(50, 50, 110),
                    );
                }

                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(row_rect)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );

                let icon_size = 22.0;
                let (icon_rect, _) = child
                    .allocate_exact_size(egui::vec2(icon_size, icon_size), egui::Sense::hover());
                if let Some(icon) = entry.icon
                    && let Some(item) = item_for_icon_id(icon)
                {
                    crate::ui::draw_item_icon_at(
                        child.painter_at(icon_rect),
                        icon_rect,
                        item,
                        sprite_textures.get(item),
                    );
                }

                child.label(palette_row_label(&entry.key, &entry.description, is_complete, is_selected));

                let clicked = ui.input(|inp| {
                    inp.pointer.hover_pos().is_some_and(|p| row_rect.contains(p))
                        && inp.pointer.button_clicked(PointerButton::Primary)
                });
                if clicked {
                    clicked_entry = Some(entry.clone());
                }
            }
        });

    if let Some(entry) = clicked_entry {
        commands.queue(move |world: &mut World| {
            palette::select_entry(world, &entry);
        });
    }

    Ok(())
}

/// Forwards a genuine world click (not over any egui area) into the engine's
/// target picker while `CommandPaletteWatchesClicks` is set.
pub fn handle_world_click_for_palette(
    mut contexts: bevy_egui::EguiContexts,
    watches_clicks: Res<CommandPaletteWatchesClicks>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    mut commands: Commands,
) -> Result {
    if !watches_clicks.0 {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    if ctx.is_pointer_over_area() || !ctx.input(|i| i.pointer.button_clicked(PointerButton::Primary))
    {
        return Ok(());
    }
    let Some(egui::Pos2 { x, y }) = ctx.pointer_latest_pos() else { return Ok(()) };
    let Ok((cam, cam_tf)) = camera_query.single() else { return Ok(()) };
    let Ok(world_pos) = cam.viewport_to_world_2d(cam_tf, Vec2::new(x, y)) else { return Ok(()) };

    commands.queue(move |world: &mut World| {
        palette::submit_click_target(world, Target::Point(world_pos));
    });
    Ok(())
}

