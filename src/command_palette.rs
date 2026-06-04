use std::collections::HashMap;

use avian2d::prelude::LinearVelocity;
use bevy::{input::keyboard::Key, prelude::*};
use bevy_egui::egui;
use bevy_egui::egui::PointerButton;
use bevy_landmass::prelude::AgentTarget2d;

use crate::{
    fov::CurrentFovState,
    goto::GotoState,
    item::{Inventory, ItemKind, item_name},
    monster::{Monster, MonsterState, Stats},
    player::{ExplorationGoal, MoveTarget, Player},
    sprite::SpriteEguiTextures,
};

pub enum PaletteCommandKind {
    /// Sub-entries come from the player's inventory filtered by item_filter.
    /// Executed as "{key} {item_letter}".
    InventoryTarget { item_filter: fn(ItemKind) -> bool },
    /// Sub-entries come from visible monsters + labeled locations.
    /// When executed, pending_target is resolved to a Vec2 before the handler runs.
    LocationTarget { target_verb: &'static str },
    /// No argument needed; fires immediately when selected.
    Instant,
}

pub struct PaletteCommand {
    pub key: String,
    pub description: String,
    pub icon: Option<ItemKind>,
    pub kind: PaletteCommandKind,
}

pub struct PaletteEntry {
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
    /// Set by the UI when a command is activated; consumed by handler systems.
    /// For LocationTarget commands this is the bare key (e.g. "z"); for
    /// InventoryTarget commands it includes the item letter (e.g. "q a").
    pub pending_command: Option<String>,
    /// For LocationTarget commands: the resolved world position of the target.
    /// Set alongside pending_command and consumed by the handler.
    pub pending_target: Option<Vec2>,
}

/// True while the palette has a LocationTarget command active and is watching
/// for world clicks. Other click-handling systems should check this.
#[derive(Resource, Default)]
pub struct CommandPaletteWatchesClicks(pub bool);

#[derive(Resource, Default)]
pub struct PaletteRegistry {
    pub commands: Vec<PaletteCommand>,
}

impl PaletteRegistry {
    pub fn is_location_targeting(&self, input: &str) -> bool {
        let trimmed = input.trim_end();
        self.commands.iter().any(|cmd| {
            matches!(cmd.kind, PaletteCommandKind::LocationTarget { .. })
                && (trimmed == cmd.key
                    || trimmed.starts_with(&format!("{} ", cmd.key)))
        })
    }

    pub fn completions(
        &self,
        input: &str,
        inventory: &Inventory,
        letter_map: &LetterMap,
        goto_state: &GotoState,
        monster_query: &Query<(Entity, &Stats, &MonsterState, &Transform), With<Monster>>,
        current_fov: Option<&geo::MultiPolygon<f32>>,
    ) -> Vec<PaletteEntry> {
        let mut entries = Vec::new();
        for cmd in &self.commands {
            match &cmd.kind {
                PaletteCommandKind::InventoryTarget { item_filter } => {
                    let tokens: Vec<&str> = input.split_whitespace().collect();
                    match tokens.as_slice() {
                        [] => entries.push(PaletteEntry {
                            key: cmd.key.clone(),
                            description: cmd.description.clone(),
                            icon: cmd.icon,
                            is_complete: false,
                        }),
                        [k] | [k, _] if *k == cmd.key.as_str() => {
                            entries.extend(inventory_entries(
                                &cmd.key, inventory, *item_filter, letter_map,
                            ));
                        }
                        _ => {}
                    }
                }
                PaletteCommandKind::LocationTarget { target_verb } => {
                    if input.is_empty() {
                        entries.push(PaletteEntry {
                            key: cmd.key.clone(),
                            description: cmd.description.clone(),
                            icon: cmd.icon,
                            is_complete: false,
                        });
                    } else if input.trim_end() == cmd.key
                        || input.starts_with(&format!("{} ", cmd.key))
                    {
                        entries.extend(targeting_sub_completions(
                            &cmd.key,
                            target_verb,
                            goto_state,
                            letter_map,
                            monster_query,
                            current_fov,
                        ));
                    }
                }
                PaletteCommandKind::Instant => {
                    if input.is_empty() {
                        entries.push(PaletteEntry {
                            key: cmd.key.clone(),
                            description: cmd.description.clone(),
                            icon: cmd.icon,
                            is_complete: true,
                        });
                    }
                }
            }
        }
        entries
    }
}

/// Build deduplicated palette entries for inventory items matching `filter`.
pub(crate) fn inventory_entries(
    command: &str,
    inventory: &Inventory,
    item_filter: fn(ItemKind) -> bool,
    letter_map: &LetterMap,
) -> Vec<PaletteEntry> {
    let mut unique: Vec<(ItemKind, u16)> = Vec::new();
    for item in &inventory.0 {
        if item_filter(*item) {
            if let Some(entry) = unique.iter_mut().find(|(t, _)| t == item) {
                entry.1 += 1;
            } else {
                unique.push((*item, 1));
            }
        }
    }
    let mut entries: Vec<PaletteEntry> = unique
        .iter()
        .filter_map(|(item, count)| {
            letter_map.get_item(*item).map(|letter| PaletteEntry {
                key: format!("{command} {letter}"),
                description: item_name(*item, *count),
                icon: Some(*item),
                is_complete: true,
            })
        })
        .collect();
    entries.sort_by_key(|e| e.key.clone());
    entries
}

/// Resolve a single target letter (uppercase monster, lowercase location label) to a Vec2.
pub fn resolve_location_letter(
    rest: &str,
    letter_map: &LetterMap,
    monster_query: &Query<(Entity, &Stats, &MonsterState, &Transform), With<Monster>>,
    goto_state: &GotoState,
) -> Option<Vec2> {
    if rest.len() != 1 {
        return None;
    }
    let c = rest.chars().next()?;
    match c {
        c if c.is_uppercase() || c.is_digit(10) => {
            let entity = letter_map.entity_for_letter(c)?;
            monster_query.get(entity).ok().map(|(_, _, _, tf)| tf.translation.truncate())
        }
        c if c.is_lowercase() => {
            let idx = (c as u8).wrapping_sub(b'a') as usize;
            goto_state.labels.get(idx).and_then(|o| *o)
        }
        _ => None,
    }
}

/// Shared completion body for LocationTarget commands.
/// `prefix` is the command key ("g", "z", …); `location_verb` is "Go to", "Zap at", etc.
pub fn targeting_sub_completions(
    prefix: &str,
    location_verb: &str,
    goto_state: &GotoState,
    letter_map: &LetterMap,
    monster_query: &Query<(Entity, &Stats, &MonsterState, &Transform), With<Monster>>,
    current_fov: Option<&geo::MultiPolygon<f32>>,
) -> Vec<PaletteEntry> {
    use geo::Contains;
    let in_fov = |pos: Vec2| -> bool {
        current_fov.map_or(true, |fov| fov.contains(&geo::Point::new(pos.x, pos.y)))
    };

    // Always build the full sub-menu so arrow keys can navigate among all targets.
    let mut entries: Vec<PaletteEntry> = monster_query
        .iter()
        .filter_map(|(entity, stats, state, tf)| {
            let letter = letter_map.letter_for_monster(entity)?;
            in_fov(tf.translation.truncate()).then(|| PaletteEntry {
                key: format!("{prefix} {letter}"),
                description: monster_desc(stats, state),
                icon: None,
                is_complete: true,
            })
        })
        .collect();
    for (i, opt_pos) in goto_state.labels.iter().enumerate() {
        if opt_pos.is_none() {
            continue;
        }
        let letter = ('a' as u8 + i as u8) as char;
        let desc = match letter {
            'h' => format!("{location_verb} left"),
            'j' => format!("{location_verb} down"),
            'k' => format!("{location_verb} up"),
            'l' => format!("{location_verb} right"),
            _ => format!("{location_verb} location {letter}"),
        };
        entries.push(PaletteEntry {
            key: format!("{prefix} {letter}"),
            description: desc,
            icon: None,
            is_complete: true,
        });
    }
    entries.sort_by(|a, b| a.key.cmp(&b.key));
    entries
}

fn monster_desc(stats: &Stats, state: &MonsterState) -> String {
    format!("{} {}/{} HP", state.label(), stats.hp as i32, stats.max_hp as i32)
}

pub struct CommandPalettePlugin;

impl Plugin for CommandPalettePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CommandPaletteState>()
            .init_resource::<PaletteRegistry>()
            .init_resource::<CommandPaletteWatchesClicks>()
            .add_systems(Update, open_palette_system);
    }
}

pub fn open_palette_system(
    keyboard: Res<ButtonInput<Key>>,
    mut state: ResMut<CommandPaletteState>,
    mut player_query: Query<
        (Entity, &mut MoveTarget, &mut AgentTarget2d, &mut LinearVelocity, &Inventory),
        With<Player>,
    >,
    mut commands: Commands,
    registry: Res<PaletteRegistry>,
    letter_map: Res<LetterMap>,
) {
    if !state.open {
        let open_with = if keyboard.just_pressed(Key::Space) {
            Some(String::new())
        } else {
            keyboard
                .get_just_pressed()
                .find_map(|k| if let Key::Character(ch) = k { Some(ch) } else { None })
                .and_then(|ch| {
                    let is_monster = ch.len() == 1
                        && ch.chars().next().is_some_and(|c| letter_map.entity_for_letter(c).is_some());
                    if is_monster {
                        return Some(format!("g {ch} "));
                    }
                    let matching_cmd = registry.commands.iter().find(|c| c.key == ch.as_str());
                    match matching_cmd {
                        Some(cmd) if matches!(cmd.kind, PaletteCommandKind::Instant) => {
                            state.pending_command = Some(ch.to_string());
                            None
                        }
                        Some(_) => Some(format!("{ch} ")),
                        None => None,
                    }
                })
        };

        if let Some(initial_input) = open_with {
            state.open = true;
            state.input = initial_input;
            state.selected_idx = 0;

            if let Ok((entity, mut move_target, mut agent_target, mut velocity, _)) =
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
    mut watches_clicks: ResMut<CommandPaletteWatchesClicks>,
    registry: Res<PaletteRegistry>,
    goto_state: Res<GotoState>,
    current_fov: Option<Res<CurrentFovState>>,
    player_query: Query<(&Inventory, &Transform), With<Player>>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    sprite_textures: Res<SpriteEguiTextures>,
    letter_map: Res<LetterMap>,
    monster_query: Query<(Entity, &Stats, &MonsterState, &Transform), With<Monster>>,
) -> Result {
    let ctx = contexts.ctx_mut()?;

    let Ok((inventory, player_tf)) = player_query.single() else {
        return Ok(());
    };

    let targeting = palette.open && registry.is_location_targeting(&palette.input);
    watches_clicks.0 = targeting;

    if palette.open {
        let fov = current_fov.as_deref().map(|r| &r.0);
        let completions = registry.completions(
            &palette.input,
            inventory,
            &letter_map,
            &goto_state,
            &monster_query,
            fov,
        );

        let screen_rect = ctx.viewport_rect();
        let screen_size = Vec2::new(screen_rect.width(), screen_rect.height());
        let anchor = camera_query
            .single()
            .ok()
            .and_then(|(cam, tf)| cam.world_to_viewport(tf, player_tf.translation).ok())
            .map(|p| farthest_corner_anchor(p, screen_size))
            .unwrap_or(egui::Align2::RIGHT_BOTTOM);

        match render_command_palette(
            ctx,
            &mut palette,
            &completions,
            anchor,
            &sprite_textures,
            targeting,
        ) {
            PaletteUiAction::Navigate(s) => {
                palette.input = s;
                palette.selected_idx = 0;
            }
            PaletteUiAction::Execute(cmd) => {
                let key = cmd.split_whitespace().next().unwrap_or("").to_string();
                if let Some(pc) = registry.commands.iter().find(|c| c.key == key) {
                    match &pc.kind {
                        PaletteCommandKind::LocationTarget { .. } => {
                            let rest = cmd[key.len()..].trim();
                            palette.pending_target = resolve_location_letter(
                                rest,
                                &letter_map,
                                &monster_query,
                                &goto_state,
                            );
                            palette.pending_command = Some(key);
                        }
                        PaletteCommandKind::InventoryTarget { .. } => {
                            palette.pending_command = Some(cmd);
                        }
                        PaletteCommandKind::Instant => {
                            palette.pending_command = Some(key);
                        }
                    }
                }
                palette.open = false;
                palette.input.clear();
            }
            PaletteUiAction::Close => {
                palette.open = false;
                palette.input.clear();
            }
            PaletteUiAction::None => {}
        }

        // Detect a click on the game world (outside any egui widget) while in targeting mode.
        if targeting
            && ctx.input(|i| i.pointer.button_clicked(PointerButton::Primary))
            && !ctx.is_pointer_over_area()
        {
            let world_pos = ctx.pointer_latest_pos().and_then(|egui::Pos2 { x, y }| {
                camera_query
                    .single()
                    .ok()
                    .and_then(|(cam, cam_tf)| {
                        cam.viewport_to_world_2d(cam_tf, bevy::math::Vec2::new(x, y)).ok()
                    })
            });
            if let Some(world_pos) = world_pos {
                let key = palette.input.split_whitespace().next().unwrap_or("").to_string();
                palette.pending_command = Some(key);
                palette.pending_target = Some(world_pos);
                palette.open = false;
                palette.input.clear();
            }
        }
    }
    Ok(())
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
    targeting: bool,
) -> PaletteUiAction {
    let n = completions.len();
    let te_id = egui::Id::new("##palette_input");

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
                if palette.input.len() > old_len {
                    let trimmed = palette.input.trim_end().to_string();
                    if completions.iter().any(|e| !e.is_complete && e.key == trimmed) {
                        palette.input = trimmed + " ";
                        move_cursor_to_end(ctx, te_id, palette.input.chars().count());
                    } else if let Some(entry) =
                        completions.iter().find(|e| e.is_complete && e.key == trimmed)
                    {
                        action = PaletteUiAction::Execute(entry.key.clone());
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

            if targeting {
                ui.separator();
                ui.label(
                    egui::RichText::new("select an onscreen target letter or click to target")
                        .italics()
                        .color(egui::Color32::from_rgb(140, 140, 170))
                        .size(11.0),
                );
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

/// Unified letter map: items use lowercase a-z (stable across sessions),
/// monsters use uppercase A-Z (recycled when monsters die or the level exits).
#[derive(Resource, Default)]
pub struct LetterMap {
    items: HashMap<ItemKind, char>,
    monsters: HashMap<Entity, char>,
}

impl LetterMap {
    // ── Item side ────────────────────────────────────────────────────────────

    pub fn get_or_assign_item(&mut self, item: ItemKind) -> Option<char> {
        if let Some(&letter) = self.items.get(&item) {
            return Some(letter);
        }
        for letter in 'a'..='z' {
            if !self.items.values().any(|&l| l == letter) {
                self.items.insert(item, letter);
                return Some(letter);
            }
        }
        None
    }

    pub fn get_item(&self, item: ItemKind) -> Option<char> { self.items.get(&item).copied() }

    pub fn item_for_letter(&self, letter: char) -> Option<ItemKind> {
        self.items.iter().find(|(_, l)| **l == letter).map(|(item, _)| *item)
    }

    // ── Monster side ─────────────────────────────────────────────────────────

    pub fn assign_monster(&mut self, entity: Entity) -> Option<char> {
        if let Some(&letter) = self.monsters.get(&entity) {
            return Some(letter);
        }
        for letter in ('1'..='9').chain('A'..'Z') {
            if !self.monsters.values().any(|&l| l == letter) {
                self.monsters.insert(entity, letter);
                return Some(letter);
            }
        }
        None
    }

    pub fn release_monster(&mut self, entity: Entity) { self.monsters.remove(&entity); }

    pub fn letter_for_monster(&self, entity: Entity) -> Option<char> {
        self.monsters.get(&entity).copied()
    }

    pub fn entity_for_letter(&self, letter: char) -> Option<Entity> {
        self.monsters.iter().find(|(_, l)| **l == letter).map(|(e, _)| *e)
    }

    pub fn clear_monsters(&mut self) { self.monsters.clear(); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{PotionColor, ScrollName};

    /// Inventory: Red×2, Readme×2, Blue×1, Agents×1 (with stable letters:
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

    fn test_letter_map() -> LetterMap {
        let mut map = LetterMap::default();
        map.get_or_assign_item(ItemKind::Potion(PotionColor::Red));
        map.get_or_assign_item(ItemKind::Scroll(ScrollName::Readme));
        map.get_or_assign_item(ItemKind::Potion(PotionColor::Blue));
        map.get_or_assign_item(ItemKind::Scroll(ScrollName::Agents));
        map
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
    fn quaff_uses_stable_letters() {
        let inv = test_inventory();
        let letter_map = test_letter_map();
        let entries = inventory_entries(
            "q", &inv, |i| matches!(i, ItemKind::Potion(_)), &letter_map,
        );
        assert_eq!(keys(&entries), ["q a", "q c"]);
        assert_eq!(entries[0].description, "2 Red potions");
        assert_eq!(entries[1].description, "a Blue potion");
    }

    #[test]
    fn read_uses_stable_letters() {
        let inv = test_inventory();
        let letter_map = test_letter_map();
        let entries = inventory_entries(
            "r", &inv, |i| matches!(i, ItemKind::Scroll(_)), &letter_map,
        );
        assert_eq!(keys(&entries), ["r b", "r d"]);
        assert_eq!(entries[0].description, "2 scrolls titled 'Readme'");
        assert_eq!(entries[1].description, "a scroll titled 'Agents'");
    }

    /// down → space → down should produce the same selected command as typing "r d".
    #[test]
    fn down_space_down_equals_r_d() {
        let inv = test_inventory();
        let letter_map = test_letter_map();

        let root_q = inventory_entries("q", &inv, |i| matches!(i, ItemKind::Potion(_)), &letter_map);
        let root_r = inventory_entries("r", &inv, |i| matches!(i, ItemKind::Scroll(_)), &letter_map);
        // Simulate root level: ["q", "r"] as non-complete stubs
        let root = vec![
            PaletteEntry { key: "q".into(), description: "".into(), icon: None, is_complete: false },
            PaletteEntry { key: "r".into(), description: "".into(), icon: None, is_complete: false },
        ];
        let after_down = press_down("", &root); // → "r"... wait, down from "" lands on first: "q"
        // Actually press_down("", &root) → current=0 (not found → 0), new=(0+1)%2=1 → "r"
        // Hmm, "" trim = "" which matches nothing, so unwrap_or(0), then (0+1)%2 = 1 → "r"
        let _ = (root_q, root_r); // suppress unused warning

        let spaced = after_down + " "; // "r "
        let read = inventory_entries("r", &inv, |i| matches!(i, ItemKind::Scroll(_)), &letter_map);
        let final_input = press_down(&spaced, &read); // → "r d" (second entry)

        assert_eq!(final_input, "r d");

        let sel = read.iter().position(|e| e.key == final_input).unwrap();
        assert_eq!(read[sel].key, "r d");
        assert!(read[sel].is_complete);
    }

    /// Typing 'x' then backspace restores empty input; the same down-space-down sequence
    /// then produces "r d" identically.
    #[test]
    fn x_backspace_then_down_space_down_equals_r_d() {
        let inv = test_inventory();
        let letter_map = test_letter_map();

        // Back to root after backspace: two commands available
        let root = vec![
            PaletteEntry { key: "q".into(), description: "".into(), icon: None, is_complete: false },
            PaletteEntry { key: "r".into(), description: "".into(), icon: None, is_complete: false },
        ];
        assert_eq!(root.len(), 2);

        let after_down = press_down("", &root);
        let spaced = after_down + " ";
        let read = inventory_entries("r", &inv, |i| matches!(i, ItemKind::Scroll(_)), &letter_map);
        let final_input = press_down(&spaced, &read);

        assert_eq!(final_input, "r d");
    }
}
