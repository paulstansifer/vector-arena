// Command palette: extensible spacebar-triggered command entry.
// Other modules register PaletteProviders to contribute commands;
// the UI renders completions and writes pending_command as a String.
use avian2d::prelude::LinearVelocity;
use bevy::{input::keyboard::Key, prelude::*};
use bevy_egui::egui;
use bevy_landmass::prelude::AgentTarget2d;

use crate::{
    goto::{self, GotoState},
    item::{Inventory, ItemKind},
    player::{ExplorationGoal, MoveTarget, Player},
    sprite::SpriteEguiTextures,
};

pub struct PaletteEntry {
    /// Full command string if is_complete; prefix otherwise.
    pub key: String,
    pub description: String,
    pub icon: Option<ItemKind>,
    /// true = executing this entry runs the action; false = navigates deeper.
    pub is_complete: bool,
}

#[derive(Resource, Default)]
pub struct CommandPaletteState {
    pub open: bool,
    pub input: String,
    pub selected_idx: usize,
    /// Set by the UI when an entry is activated; consumed by handler systems.
    pub pending_command: Option<String>,
}

pub trait PaletteProvider: Send + Sync + 'static {
    fn completions(&self, input: &str, inventory: &Inventory) -> Vec<PaletteEntry>;
}

#[derive(Resource, Default)]
pub struct CommandPaletteRegistry {
    providers: Vec<Box<dyn PaletteProvider>>,
}

impl CommandPaletteRegistry {
    pub fn register(&mut self, provider: Box<dyn PaletteProvider>) {
        self.providers.push(provider);
    }

    pub fn completions(&self, input: &str, inventory: &Inventory) -> Vec<PaletteEntry> {
        self.providers.iter().flat_map(|p| p.completions(input, inventory)).collect()
    }
}

pub struct CommandPalettePlugin;

impl Plugin for CommandPalettePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CommandPaletteState>()
            .init_resource::<CommandPaletteRegistry>()
            .add_systems(Update, open_palette_system);
    }
}

pub fn open_palette_system(
    keyboard: Res<ButtonInput<Key>>,
    mut state: ResMut<CommandPaletteState>,
    mut player_query: Query<
        (Entity, &mut MoveTarget, &mut AgentTarget2d, &mut LinearVelocity),
        With<Player>,
    >,
    mut commands: Commands,
) {
    if !state.open {
        // Space opens with blank input; a letter key opens pre-filled with "[letter] "
        // so completions for that command show immediately on the first frame.
        let open_with = if keyboard.just_pressed(Key::Space) {
            Some(String::new())
        } else {
            keyboard
                .get_just_pressed()
                .find_map(|k| if let Key::Character(ch) = k { Some(ch) } else { None })
                .map(|ch| format!("{ch} "))
        };

        if let Some(initial_input) = open_with {
            state.open = true;
            state.input = initial_input;
            state.selected_idx = 0;

            if let Ok((entity, mut move_target, mut agent_target, mut velocity)) =
                player_query.single_mut()
            {
                move_target.active = false;
                *agent_target = AgentTarget2d::None;
                velocity.0 = Vec2::ZERO;
                commands.entity(entity).remove::<ExplorationGoal>();
            }
        }
    } else if keyboard.just_pressed(Key::Escape) {
        state.open = false;
        state.input.clear();
    }
}

pub fn palette_system(
    mut contexts: bevy_egui::EguiContexts,
    mut palette: ResMut<CommandPaletteState>,
    registry: Res<CommandPaletteRegistry>,
    goto_state: Res<GotoState>,
    player_query: Query<(&Inventory, &Transform), With<Player>>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    sprite_textures: Res<SpriteEguiTextures>,
) -> Result {
    let ctx = contexts.ctx_mut()?;

    let Ok((inventory, player_tf)) = player_query.single() else {
        return Ok(());
    };

    if palette.open {
        let mut completions = registry.completions(&palette.input, inventory);
        completions.extend(goto::goto_completions(&palette.input, &goto_state));

        let screen_rect = ctx.viewport_rect();
        let screen_size = Vec2::new(screen_rect.width(), screen_rect.height());
        let anchor = camera_query
            .single()
            .ok()
            .and_then(|(cam, tf)| cam.world_to_viewport(tf, player_tf.translation).ok())
            .map(|p| farthest_corner_anchor(p, screen_size))
            .unwrap_or(egui::Align2::RIGHT_BOTTOM);

        match render_command_palette(ctx, &mut palette, &completions, anchor, &sprite_textures) {
            PaletteUiAction::Navigate(s) => {
                palette.input = s;
                palette.selected_idx = 0;
            }
            PaletteUiAction::Execute(cmd) => {
                palette.pending_command = Some(cmd);
                palette.open = false;
                palette.input.clear();
            }
            PaletteUiAction::Close => {
                palette.open = false;
                palette.input.clear();
            }
            PaletteUiAction::None => {}
        }
    }
    Ok(())
}

pub fn letter_to_idx(c: char) -> Option<usize> {
    c.is_ascii_lowercase().then(|| (c as u8 - b'a') as usize)
}

fn farthest_corner_anchor(player_screen: Vec2, screen_size: Vec2) -> egui::Align2 {
    match (player_screen.x > screen_size.x / 2.0, player_screen.y > screen_size.y / 2.0) {
        (false, false) => egui::Align2::RIGHT_BOTTOM,
        (true, false) => egui::Align2::LEFT_BOTTOM,
        (false, true) => egui::Align2::RIGHT_TOP,
        (true, true) => egui::Align2::LEFT_TOP,
    }
}

enum PaletteUiAction {
    None,
    Navigate(String),
    Execute(String),
    Close,
}

fn sync_selection(input: &str, completions: &[PaletteEntry]) -> usize {
    let trimmed = input.trim();
    completions.iter().position(|e| e.key == trimmed).unwrap_or(0)
}

fn move_cursor_to_end(ctx: &egui::Context, te_id: egui::Id, len: usize) {
    if let Some(mut state) = egui::widgets::text_edit::TextEditState::load(ctx, te_id) {
        let ccursor = egui::text::CCursor::new(len);
        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(ccursor)));
        state.store(ctx, te_id);
    }
}

fn render_command_palette(
    ctx: &egui::Context,
    palette: &mut CommandPaletteState,
    completions: &[PaletteEntry],
    anchor: egui::Align2,
    sprite_textures: &SpriteEguiTextures,
) -> PaletteUiAction {
    let n = completions.len();
    let te_id = egui::Id::new("##palette_input");

    // ── Keyboard handling before rendering ──
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        return PaletteUiAction::Close;
    }

    let mut cursor_to_end = false;

    if ctx.input(|i| i.key_pressed(egui::Key::ArrowDown)) && n > 0 {
        let current = sync_selection(&palette.input, completions);
        let new_idx = (current + 1) % n;
        palette.input = completions[new_idx].key.clone();
        palette.selected_idx = new_idx;
        cursor_to_end = true;
    }
    if ctx.input(|i| i.key_pressed(egui::Key::ArrowUp)) && n > 0 {
        let current = sync_selection(&palette.input, completions);
        let new_idx = (current + n - 1) % n;
        palette.input = completions[new_idx].key.clone();
        palette.selected_idx = new_idx;
        cursor_to_end = true;
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Enter)) && !completions.is_empty() {
        let idx = sync_selection(&palette.input, completions).min(n - 1);
        let entry = &completions[idx];
        return if entry.is_complete {
            PaletteUiAction::Execute(entry.key.clone())
        } else {
            PaletteUiAction::Navigate(entry.key.clone() + " ")
        };
    }

    if cursor_to_end {
        move_cursor_to_end(ctx, te_id, palette.input.chars().count());
    }

    let mut action = PaletteUiAction::None;

    let frame = egui::Frame::window(&ctx.style())
        .fill(egui::Color32::from_rgb(20, 20, 30))
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(70, 70, 110)));

    egui::Window::new("##cmd_palette")
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .anchor(anchor, [8.0, 8.0])
        .min_width(280.0)
        .frame(frame)
        .show(ctx, |ui| {
            let old_input = palette.input.clone();
            let old_len = old_input.len();

            let te_response = ui.add(
                egui::TextEdit::singleline(&mut palette.input)
                    .id(te_id)
                    .hint_text("type a command…")
                    .desired_width(f32::INFINITY),
            );
            te_response.request_focus();

            if palette.input != old_input {
                // Auto-space: if input grew and exactly matches a non-complete entry key
                if palette.input.len() > old_len {
                    let trimmed = palette.input.trim_end().to_string();
                    if completions.iter().any(|e| !e.is_complete && e.key == trimmed) {
                        palette.input = trimmed + " ";
                        move_cursor_to_end(ctx, te_id, palette.input.chars().count());
                    }
                }
                palette.selected_idx = sync_selection(&palette.input, completions);
            }

            ui.separator();

            if completions.is_empty() {
                ui.label(
                    egui::RichText::new("nothing applicable").italics().color(egui::Color32::WHITE),
                );
                return;
            }

            let selected_idx = palette.selected_idx.min(n - 1);
            let row_height = 26.0;
            let mut clicked_idx: Option<usize> = None;
            let mut hovered_idx: Option<usize> = None;

            for (i, entry) in completions.iter().enumerate() {
                let is_selected = selected_idx == i;
                let row_size = egui::vec2(ui.available_width(), row_height);
                let (row_rect, row_response) =
                    ui.allocate_exact_size(row_size, egui::Sense::click());

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
                if let Some(item) = entry.icon {
                    crate::ui::draw_item_icon_at(
                        child.painter_at(icon_rect),
                        icon_rect,
                        item,
                        sprite_textures.get(item),
                    );
                }

                child.label(palette_row_label(
                    &entry.key,
                    &entry.description,
                    entry.is_complete,
                    is_selected,
                ));

                if row_response.hovered() {
                    hovered_idx = Some(i);
                }
                if row_response.clicked() {
                    clicked_idx = Some(i);
                }
            }

            if let Some(idx) = hovered_idx {
                palette.selected_idx = idx;
            }
            if let Some(idx) = clicked_idx {
                let entry = &completions[idx];
                action = if entry.is_complete {
                    PaletteUiAction::Execute(entry.key.clone())
                } else {
                    PaletteUiAction::Navigate(entry.key.clone() + " ")
                };
            }
        });

    action
}

fn palette_row_label(
    key: &str,
    description: &str,
    is_complete: bool,
    is_selected: bool,
) -> egui::text::LayoutJob {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{ItemCommandProvider, PotionColor, ScrollName};

    /// Inventory: Red×2, Readme×2, Blue×1, Agents×1 (interleaved so global order is
    /// Red='a', Readme='b', Blue='c', Agents='d').
    fn test_inventory() -> Inventory {
        Inventory(vec![
            ItemKind::Potion(PotionColor::Red),
            ItemKind::Potion(PotionColor::Red),
            ItemKind::Scroll(ScrollName::Readme),
            ItemKind::Potion(PotionColor::Blue),
            ItemKind::Scroll(ScrollName::Readme),
            ItemKind::Scroll(ScrollName::Agents),
        ])
    }

    fn keys(entries: &[PaletteEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.key.as_str()).collect()
    }

    /// Simulate arrow-down: advance selection by one (wrapping), return the new input string.
    fn press_down(input: &str, completions: &[PaletteEntry]) -> String {
        let n = completions.len();
        assert!(n > 0, "press_down called with empty completions");
        let current = completions.iter().position(|e| e.key == input.trim()).unwrap_or(0);
        completions[(current + 1) % n].key.clone()
    }

    #[test]
    fn quaff_uses_global_letters() {
        let inv = test_inventory();
        let entries = ItemCommandProvider.completions("q ", &inv);
        assert_eq!(keys(&entries), ["q a", "q c"]);
        assert_eq!(entries[0].description, "2 Red potions");
        assert_eq!(entries[1].description, "a Blue potion");
    }

    #[test]
    fn read_uses_global_letters() {
        let inv = test_inventory();
        let entries = ItemCommandProvider.completions("r ", &inv);
        assert_eq!(keys(&entries), ["r b", "r d"]);
        assert_eq!(entries[0].description, "2 scrolls titled 'Readme'");
        assert_eq!(entries[1].description, "a scroll titled 'Agents'");
    }

    /// down → space → down should produce the same selected command as typing "r d".
    #[test]
    fn down_space_down_equals_r_d() {
        let inv = test_inventory();
        let p = ItemCommandProvider;

        let root = p.completions("", &inv);
        let after_down = press_down("", &root); // → "r"

        // Space typed by user (no auto-space because "r" is the key of a non-complete root entry,
        // but completions at this point are already read entries since ["r"] matches the sub-menu)
        let spaced = after_down + " "; // "r "
        let read = p.completions(&spaced, &inv);
        let final_input = press_down(&spaced, &read); // → "r d"

        assert_eq!(final_input, "r d");

        // Confirm "r d" typed directly selects the same entry
        let sel = read.iter().position(|e| e.key == final_input).unwrap();
        assert_eq!(read[sel].key, "r d");
        assert!(read[sel].is_complete);
    }

    /// Typing 'x' then backspace restores empty input; the same down-space-down sequence
    /// then produces "r d" identically.
    #[test]
    fn x_backspace_then_down_space_down_equals_r_d() {
        let inv = test_inventory();
        let p = ItemCommandProvider;

        // 'x' matches nothing
        assert!(p.completions("x", &inv).is_empty());
        // After backspace we're back at "" with root completions
        let root = p.completions("", &inv);
        assert_eq!(root.len(), 2);

        let after_down = press_down("", &root);
        let spaced = after_down + " ";
        let read = p.completions(&spaced, &inv);
        let final_input = press_down(&spaced, &read);

        assert_eq!(final_input, "r d");
    }
}
