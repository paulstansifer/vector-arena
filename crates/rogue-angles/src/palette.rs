// The command palette: a keystroke-completion tree addressed by
// letter-and-number abbreviations. This is deliberately baked into the
// engine's interaction model (see docs/ENGINE-SPLIT.md) — every game built on
// rogue-angles gets this UI grammar — but the engine never sees what any
// individual command *means*. Games register a tree of `EntryOutcome`s; every
// terminal path invokes a `SystemId<In<CommandInvocation>>` handler the game
// supplied, with no re-parsing or polled mailbox.
//
// Rendering (egui, sprites, …) is not this crate's concern — nothing here
// draws anything. A game reads `CommandPaletteState` + `CurrentPaletteEntries`
// to render its own UI, and drives the state machine via the keyboard system
// below plus `submit_click_target` / `select_entry` for click-driven input.
use std::collections::HashMap;

use bevy::{ecs::system::SystemId, input::keyboard::Key, prelude::*};
use geo::Contains;

use crate::fov::CurrentFovState;

// ── Icons ────────────────────────────────────────────────────────────────────

/// Opaque handle to an icon a game registers; the engine never interprets it,
/// just carries it through to whatever renders the palette.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct IconId(pub u32);

// ── Targets ──────────────────────────────────────────────────────────────────

/// Marks an entity addressable by an auto-assigned uppercase-letter/digit
/// label in the built-in target picker. Letters are assigned on insert and
/// released on despawn or component removal — a game never manages them by
/// hand.
#[derive(Component)]
pub struct Targetable;

/// Optional one-line description shown next to an entity's label in the
/// target picker (e.g. "Sleeping goblin 12/20 HP"). Falls back to a generic
/// "target `<letter>`" when absent.
#[derive(Component, Default)]
pub struct TargetDescription(pub String);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Entity(Entity),
    Point(Vec2),
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum TargetFilter {
    #[default]
    Any,
    EntitiesOnly,
    LocationsOnly,
}

// ── Entity labels: uppercase + digits ───────────────────────────────────────

#[derive(Resource, Default)]
pub struct EntityLabels {
    letters: HashMap<Entity, char>,
}

impl EntityLabels {
    fn assign(&mut self, entity: Entity) {
        if self.letters.contains_key(&entity) {
            return;
        }
        for letter in ('1'..='9').chain('A'..='Z') {
            if !self.letters.values().any(|&l| l == letter) {
                self.letters.insert(entity, letter);
                return;
            }
        }
    }

    pub fn letter_for(&self, entity: Entity) -> Option<char> { self.letters.get(&entity).copied() }

    pub fn entity_for_letter(&self, letter: char) -> Option<Entity> {
        self.letters.iter().find(|&(_, &l)| l == letter).map(|(&e, _)| e)
    }

    /// Release every assigned letter immediately, e.g. at a level transition.
    /// `release_entity_labels` also does this per-entity on despawn, so this
    /// is a belt-and-suspenders call for "zero stale letters, even for a
    /// frame" rather than something every game must call.
    pub fn clear(&mut self) { self.letters.clear(); }
}

pub fn assign_entity_labels(
    mut labels: ResMut<EntityLabels>,
    query: Query<Entity, Added<Targetable>>,
) {
    for entity in &query {
        labels.assign(entity);
    }
}

pub fn release_entity_labels(
    mut labels: ResMut<EntityLabels>,
    mut removed: RemovedComponents<Targetable>,
) {
    for entity in removed.read() {
        labels.letters.remove(&entity);
    }
}

// ── Label pool: stable lowercase a-z labels for a game-defined key ─────────

/// Stable lowercase-letter labels for a game-defined key type (e.g. an item
/// kind). Generic over `K` so the engine never sees what the game's keys are;
/// a game instantiates `LabelPool<ItemKind>` and registers it itself.
#[derive(Resource)]
pub struct LabelPool<K: Copy + Eq + std::hash::Hash + Send + Sync + 'static> {
    letters: HashMap<K, char>,
}

impl<K: Copy + Eq + std::hash::Hash + Send + Sync + 'static> Default for LabelPool<K> {
    fn default() -> Self { Self { letters: HashMap::new() } }
}

impl<K: Copy + Eq + std::hash::Hash + Send + Sync + 'static> LabelPool<K> {
    pub fn get_or_assign(&mut self, key: K) -> Option<char> {
        if let Some(&letter) = self.letters.get(&key) {
            return Some(letter);
        }
        for letter in 'a'..='z' {
            if !self.letters.values().any(|&l| l == letter) {
                self.letters.insert(key, letter);
                return Some(letter);
            }
        }
        None
    }

    pub fn get(&self, key: K) -> Option<char> { self.letters.get(&key).copied() }

    pub fn key_for_letter(&self, letter: char) -> Option<K> {
        self.letters.iter().find(|&(_, &l)| l == letter).map(|(&k, _)| k)
    }
}

// ── Location labels: lowercase a-z waypoints, 8 reserved direction slots ───

pub const DIR_LEFT: usize = (b'h' - b'a') as usize;
pub const DIR_DOWN: usize = (b'j' - b'a') as usize;
pub const DIR_UP: usize = (b'k' - b'a') as usize;
pub const DIR_RIGHT: usize = (b'l' - b'a') as usize;
pub const DIR_UP_LEFT: usize = (b'y' - b'a') as usize;
pub const DIR_UP_RIGHT: usize = (b'u' - b'a') as usize;
pub const DIR_DOWN_LEFT: usize = (b'b' - b'a') as usize;
pub const DIR_DOWN_RIGHT: usize = (b'n' - b'a') as usize;

fn direction_word(slot: usize) -> Option<&'static str> {
    match slot {
        s if s == DIR_LEFT => Some("left"),
        s if s == DIR_DOWN => Some("down"),
        s if s == DIR_UP => Some("up"),
        s if s == DIR_RIGHT => Some("right"),
        s if s == DIR_UP_LEFT => Some("up-left"),
        s if s == DIR_UP_RIGHT => Some("up-right"),
        s if s == DIR_DOWN_LEFT => Some("down-left"),
        s if s == DIR_DOWN_RIGHT => Some("down-right"),
        _ => None,
    }
}

/// 26 lowercase-letter slots mapped to world points. A game assigns these
/// however it likes (frontier waypoints, points of interest, …); the eight
/// `DIR_*` slots are a convention for pinning cardinal/diagonal directions
/// relative to the player, used only to synthesize a description when the
/// game hasn't supplied one via `LocationDescriptions`.
#[derive(Resource, Default)]
pub struct LocationLabels {
    pub slots: [Option<Vec2>; 26],
}

impl LocationLabels {
    pub fn get(&self, letter: char) -> Option<Vec2> {
        if !letter.is_ascii_lowercase() {
            return None;
        }
        self.slots[(letter as u8 - b'a') as usize]
    }
}

/// Per-letter description override (e.g. "staircase down"); falls back to a
/// direction word for the eight reserved slots, or "location `<letter>`".
#[derive(Resource, Default)]
pub struct LocationDescriptions(pub HashMap<char, String>);

// ── Command tree ─────────────────────────────────────────────────────────────

pub type PalettePath = Vec<String>;

pub struct CommandInvocation {
    pub path: PalettePath,
    pub target: Option<Target>,
}

#[derive(Clone)]
pub enum EntryOutcome {
    /// Ask this one-shot system for the next level of entries, given the full
    /// path committed so far (including the key that led here).
    Submenu(SystemId<In<PalettePath>, Vec<PaletteEntry>>),
    /// Switch to the built-in target picker.
    PickTarget { verb: String, filter: TargetFilter },
    /// This path is complete; selecting it runs the owning command's handler.
    Run,
}

#[derive(Clone)]
pub struct PaletteEntry {
    /// The full path so far, space-joined, e.g. "w a".
    pub key: String,
    pub description: String,
    pub icon: Option<IconId>,
    pub outcome: EntryOutcome,
}

pub struct PaletteCommand {
    pub key: String,
    pub description: String,
    pub icon: Option<IconId>,
    pub outcome: EntryOutcome,
    pub handler: SystemId<In<CommandInvocation>>,
}

#[derive(Resource, Default)]
pub struct PaletteRegistry {
    pub commands: Vec<PaletteCommand>,
}

/// The key of the command that should run when a targetable entity's bare
/// letter is typed with the palette closed (e.g. "g" for "go to" in a game
/// where that's the natural default). `None` means bare entity letters do
/// nothing when the palette is closed. Set once by the game at startup.
#[derive(Resource, Default)]
pub struct DefaultEntityAction(pub Option<String>);

// ── Palette state ────────────────────────────────────────────────────────────

#[derive(Resource, Default)]
pub struct CommandPaletteState {
    pub open: bool,
    pub input: String,
    pub selected_idx: usize,
}

/// True while the palette is in target-picking mode and world clicks should
/// be interpreted as a target rather than, say, click-to-move.
#[derive(Resource, Default)]
pub struct CommandPaletteWatchesClicks(pub bool);

/// The entries selectable at the *next* step, recomputed every frame the
/// palette is open by `update_palette_entries`. Read by rendering and by the
/// keyboard/click input systems — never write to this directly.
#[derive(Resource, Default)]
pub struct CurrentPaletteEntries(pub Vec<PaletteEntry>);

fn committed_path(input: &str) -> PalettePath { input.split_whitespace().map(String::from).collect() }

fn is_in_fov(world: &World, pos: Vec2) -> bool {
    world
        .get_resource::<CurrentFovState>()
        .is_none_or(|fov| fov.0.contains(&geo::Point::new(pos.x, pos.y)))
}

fn build_target_entries(
    world: &mut World,
    prefix: &str,
    verb: &str,
    filter: TargetFilter,
) -> Vec<PaletteEntry> {
    let mut entries = Vec::new();

    if filter != TargetFilter::LocationsOnly {
        let mut query = world
            .query_filtered::<(Entity, &Transform, Option<&TargetDescription>), With<Targetable>>();
        let candidates: Vec<(Entity, Vec2, Option<String>)> = query
            .iter(world)
            .map(|(e, tf, desc)| (e, tf.translation.truncate(), desc.map(|d| d.0.clone())))
            .collect();
        let labels = world.resource::<EntityLabels>();
        for (entity, pos, desc) in candidates {
            if !is_in_fov(world, pos) {
                continue;
            }
            let Some(letter) = labels.letter_for(entity) else { continue };
            entries.push(PaletteEntry {
                key: format!("{prefix} {letter}"),
                description: desc.unwrap_or_else(|| format!("target {letter}")),
                icon: None,
                outcome: EntryOutcome::Run,
            });
        }
    }

    if filter != TargetFilter::EntitiesOnly {
        let loc_labels = world.resource::<LocationLabels>();
        let slots = loc_labels.slots;
        for (i, opt_pos) in slots.iter().enumerate() {
            let Some(pos) = opt_pos else { continue };
            if !is_in_fov(world, *pos) {
                continue;
            }
            let letter = (b'a' + i as u8) as char;
            let descs = world.resource::<LocationDescriptions>();
            let description = descs.0.get(&letter).cloned().unwrap_or_else(|| {
                direction_word(i)
                    .map(|w| format!("{verb} {w}"))
                    .unwrap_or_else(|| format!("{verb} location {letter}"))
            });
            entries.push(PaletteEntry {
                key: format!("{prefix} {letter}"),
                description,
                icon: None,
                outcome: EntryOutcome::Run,
            });
        }
    }

    entries.sort_by(|a, b| a.key.cmp(&b.key));
    entries
}

/// Resolve the `EntryOutcome` in force *after* `path` (non-empty) has been
/// fully selected, calling game-registered `Submenu` systems as needed.
fn resolve_outcome(world: &mut World, path: &[String]) -> Option<EntryOutcome> {
    if path.len() == 1 {
        return world.resource::<PaletteRegistry>().commands.iter().find_map(|c| {
            (c.key == path[0]).then(|| c.outcome.clone())
        });
    }
    let parent_outcome = resolve_outcome(world, &path[..path.len() - 1])?;
    match parent_outcome {
        EntryOutcome::Submenu(system_id) => {
            let entries: Vec<PaletteEntry> =
                world.run_system_with(system_id, path[..path.len() - 1].to_vec()).ok()?;
            let full_key = path.join(" ");
            entries.into_iter().find(|e| e.key == full_key).map(|e| e.outcome)
        }
        // A PickTarget's next token is always a resolved target; that path is terminal.
        EntryOutcome::PickTarget { .. } => Some(EntryOutcome::Run),
        // Typed past a terminal node — no further entries.
        EntryOutcome::Run => None,
    }
}

/// The entries selectable at the step *after* `committed` (which may be
/// empty, meaning "no command chosen yet" — the registry's root entries).
pub fn compute_entries(world: &mut World, committed: &[String]) -> Vec<PaletteEntry> {
    if committed.is_empty() {
        return world
            .resource::<PaletteRegistry>()
            .commands
            .iter()
            .map(|c| PaletteEntry {
                key: c.key.clone(),
                description: c.description.clone(),
                icon: c.icon,
                outcome: c.outcome.clone(),
            })
            .collect();
    }
    match resolve_outcome(world, committed) {
        Some(EntryOutcome::Submenu(system_id)) => {
            world.run_system_with(system_id, committed.to_vec()).unwrap_or_default()
        }
        Some(EntryOutcome::PickTarget { verb, filter }) => {
            build_target_entries(world, &committed.join(" "), &verb, filter)
        }
        Some(EntryOutcome::Run) | None => Vec::new(),
    }
}

/// Exclusive system: recomputes `CurrentPaletteEntries` from
/// `CommandPaletteState.input`. Must run before anything else that reads
/// `CurrentPaletteEntries` this frame (rendering, keyboard/click input).
pub fn update_palette_entries(world: &mut World) {
    if !world.resource::<CommandPaletteState>().open {
        world.resource_mut::<CurrentPaletteEntries>().0.clear();
        return;
    }
    let input = world.resource::<CommandPaletteState>().input.clone();
    let committed = committed_path(&input);
    let entries = compute_entries(world, &committed);
    world.resource_mut::<CurrentPaletteEntries>().0 = entries;
}

/// Keep `CommandPaletteWatchesClicks` in sync with whether the currently
/// committed path is mid-`PickTarget`, so other systems (click-to-move, world
/// click handling) know whether a world click should be treated as a target.
pub fn update_watches_clicks(world: &mut World) {
    let open = world.resource::<CommandPaletteState>().open;
    let watching = open
        && matches!(
            {
                let input = world.resource::<CommandPaletteState>().input.clone();
                let committed = committed_path(&input);
                if committed.is_empty() { None } else { resolve_outcome(world, &committed) }
            },
            Some(EntryOutcome::PickTarget { .. })
        );
    world.resource_mut::<CommandPaletteWatchesClicks>().0 = watching;
}

/// Runs the command owning `path`'s root, with `target` if one was resolved,
/// then closes the palette. `path` is the full path up to and including the
/// terminal entry (for a typed/clicked completion) — or the stem path before
/// a target for a raw world click, which is why `target` is separate rather
/// than encoded into `path`.
fn run_command(world: &mut World, path: PalettePath, target: Option<Target>) {
    let Some(root_key) = path.first().cloned() else { return };
    let handler =
        world.resource::<PaletteRegistry>().commands.iter().find(|c| c.key == root_key).map(|c| c.handler);
    if let Some(handler) = handler {
        let _ = world.run_system_with(handler, CommandInvocation { path, target });
    }
    let mut state = world.resource_mut::<CommandPaletteState>();
    state.open = false;
    state.input.clear();
    state.selected_idx = 0;
}

enum TargetSplit {
    /// `path`'s last token isn't completing a `PickTarget` node — nothing to pull out.
    NotTarget,
    /// It is, and the letter resolved: the stem path (without the letter) plus its target.
    Resolved(PalettePath, Target),
    /// It is, but the letter didn't resolve to anything (stale/invalid label).
    Unresolved,
}

/// If `path`'s last token is a target letter completing a `PickTarget` node (i.e. the path
/// minus its last token resolves to `PickTarget`), resolves that letter via
/// `resolve_letter_target`. `build_target_entries` only bakes a target letter into
/// `PaletteEntry.key`/`description` text, not into `PaletteEntry.outcome` (which is just
/// `Run`), so every caller that dispatches a selected/typed/clicked entry needs this
/// re-resolution step — shared here so `select_entry` and `execute_path_string` can't drift
/// out of sync on it again.
fn split_and_resolve_target(world: &mut World, path: &[String]) -> TargetSplit {
    if path.len() < 2 {
        return TargetSplit::NotTarget;
    }
    let Some(EntryOutcome::PickTarget { .. }) = resolve_outcome(world, &path[..path.len() - 1])
    else {
        return TargetSplit::NotTarget;
    };
    let letter_str = &path[path.len() - 1];
    let Some(letter) = letter_str.chars().next().filter(|_| letter_str.chars().count() == 1) else {
        return TargetSplit::Unresolved;
    };
    match resolve_letter_target(world, letter) {
        Some(target) => TargetSplit::Resolved(path[..path.len() - 1].to_vec(), target),
        None => TargetSplit::Unresolved,
    }
}

/// Given a chosen entry (from typing, arrow+Enter, or a clicked row), either
/// navigate deeper or execute — shared by every input source that picks an
/// entry by its already-known `PaletteEntry`.
pub fn select_entry(world: &mut World, entry: &PaletteEntry) {
    match &entry.outcome {
        EntryOutcome::Run => {
            let path = committed_path(&entry.key);
            match split_and_resolve_target(world, &path) {
                TargetSplit::Resolved(stem, target) => run_command(world, stem, Some(target)),
                TargetSplit::NotTarget => run_command(world, path, None),
                // The entry came from CurrentPaletteEntries, already built against a valid
                // target, so this shouldn't happen — but if the label went stale this frame,
                // do nothing rather than run the command with no target (the original bug).
                TargetSplit::Unresolved => {}
            }
        }
        EntryOutcome::Submenu(_) | EntryOutcome::PickTarget { .. } => {
            let mut state = world.resource_mut::<CommandPaletteState>();
            state.input = format!("{} ", entry.key);
            state.selected_idx = 0;
        }
    }
}

/// Called by the game once it has determined (via its own camera + UI-capture
/// logic) that a genuine world click landed while `CommandPaletteWatchesClicks`
/// was true. Resolves the current `PickTarget` node directly against `target`
/// — no label lookup needed, since a click always names its target precisely.
pub fn submit_click_target(world: &mut World, target: Target) {
    let input = world.resource::<CommandPaletteState>().input.clone();
    let path = committed_path(&input);
    if path.is_empty() {
        return;
    }
    run_command(world, path, Some(target));
}

/// Resolve a single target letter per the engine's baked-in grammar:
/// uppercase-or-digit names a `Targetable` entity, lowercase names an
/// assigned location. This is the same resolver used implicitly by typed/
/// clicked target entries, exposed directly for programmatic driving (e.g. a
/// headless scripting harness) that wants to name a target by letter without
/// going through simulated keystrokes.
pub fn resolve_letter_target(world: &World, letter: char) -> Option<Target> {
    if letter.is_uppercase() || letter.is_ascii_digit() {
        world.get_resource::<EntityLabels>()?.entity_for_letter(letter).map(Target::Entity)
    } else if letter.is_lowercase() {
        world.get_resource::<LocationLabels>()?.get(letter).map(Target::Point)
    } else {
        None
    }
}

/// Directly execute a full space-separated command path (e.g. `"w a A"`),
/// bypassing the keyboard/click UI entirely — for programmatic driving such
/// as a headless scripting harness. If the path's last token is a target
/// letter for a `PickTarget` node, it's resolved via `resolve_letter_target`
/// and passed as the invocation's target; otherwise the whole path must
/// already resolve to `Run`. Returns `false` if the path doesn't resolve to a
/// runnable command (in which case nothing happens).
pub fn execute_path_string(world: &mut World, input: &str) -> bool {
    let path = committed_path(input);
    if path.is_empty() {
        return false;
    }
    match split_and_resolve_target(world, &path) {
        TargetSplit::Resolved(stem, target) => {
            run_command(world, stem, Some(target));
            true
        }
        TargetSplit::Unresolved => false,
        TargetSplit::NotTarget => {
            if matches!(resolve_outcome(world, &path), Some(EntryOutcome::Run)) {
                run_command(world, path, None);
                true
            } else {
                false
            }
        }
    }
}

/// Escape: close the palette and discard whatever was typed.
pub fn close_on_escape(keyboard: Res<ButtonInput<KeyCode>>, mut state: ResMut<CommandPaletteState>) {
    if state.open && keyboard.just_pressed(KeyCode::Escape) {
        state.open = false;
        state.input.clear();
        state.selected_idx = 0;
    }
}

/// Opens the palette on Space (empty), on a registered root command's key
/// (instant commands like "." fire immediately; everything else opens with
/// that key typed), or on a `Targetable` entity's bare letter (runs
/// `DefaultEntityAction`, if the game set one).
pub fn open_palette_on_keypress(
    keyboard: Res<ButtonInput<Key>>,
    mut state: ResMut<CommandPaletteState>,
    registry: Res<PaletteRegistry>,
    labels: Res<EntityLabels>,
    default_action: Res<DefaultEntityAction>,
    mut commands: Commands,
) {
    if state.open {
        return;
    }
    if keyboard.just_pressed(Key::Space) {
        state.open = true;
        state.input.clear();
        state.selected_idx = 0;
        return;
    }
    let Some(ch) = keyboard.get_just_pressed().find_map(|k| {
        if let Key::Character(s) = k { s.chars().next() } else { None }
    }) else {
        return;
    };
    if let Some(entity) = labels.entity_for_letter(ch)
        && let Some(default_key) = default_action.0.clone()
    {
        commands.queue(move |world: &mut World| {
            run_command(world, vec![default_key], Some(Target::Entity(entity)));
        });
        return;
    }
    if let Some(cmd) = registry.commands.iter().find(|c| c.key == ch.to_string()) {
        if matches!(cmd.outcome, EntryOutcome::Run) {
            let key = cmd.key.clone();
            commands.queue(move |world: &mut World| {
                run_command(world, vec![key], None);
            });
        } else {
            state.open = true;
            state.input = format!("{ch} ");
            state.selected_idx = 0;
        }
    }
}

/// Typing, backspace, arrow navigation, and Enter — the palette's keyboard
/// state machine. Typing a character that exactly names one of
/// `CurrentPaletteEntries` (computed from the input as of frame start, i.e.
/// before this keystroke) auto-selects it immediately — matching the "type
/// the whole thing, no Enter needed" feel the single-character-token grammar
/// is built around. Enter does the same for whatever's currently typed/typed
/// via arrow navigation.
pub fn handle_palette_keyboard(
    keyboard: Res<ButtonInput<Key>>,
    keyboard_codes: Res<ButtonInput<KeyCode>>,
    entries: Res<CurrentPaletteEntries>,
    mut commands: Commands,
) {
    let up = keyboard_codes.just_pressed(KeyCode::ArrowUp);
    let down = keyboard_codes.just_pressed(KeyCode::ArrowDown);
    let enter = keyboard_codes.just_pressed(KeyCode::Enter);
    let backspace = keyboard_codes.just_pressed(KeyCode::Backspace);
    let typed_char = keyboard
        .get_just_pressed()
        .find_map(|k| if let Key::Character(s) = k { s.chars().next() } else { None });

    if !up && !down && !enter && !backspace && typed_char.is_none() {
        return;
    }

    let entries = entries.0.clone();
    commands.queue(move |world: &mut World| {
        if !world.resource::<CommandPaletteState>().open {
            return;
        }
        let n = entries.len();

        if up || down {
            if n == 0 {
                return;
            }
            let current_key = world.resource::<CommandPaletteState>().input.trim().to_string();
            let current = entries.iter().position(|e| e.key == current_key).unwrap_or(0);
            let next = if down { (current + 1) % n } else { (current + n - 1) % n };
            let mut state = world.resource_mut::<CommandPaletteState>();
            state.input = entries[next].key.clone();
            state.selected_idx = next;
            return;
        }

        if backspace {
            let mut state = world.resource_mut::<CommandPaletteState>();
            state.input.pop();
            if state.input.ends_with(' ') {
                state.input.pop();
            }
            return;
        }

        if enter {
            let current_key = world.resource::<CommandPaletteState>().input.trim().to_string();
            if let Some(entry) = entries.iter().find(|e| e.key == current_key).cloned() {
                select_entry(world, &entry);
            }
            return;
        }

        if let Some(ch) = typed_char {
            let committed = committed_path(&world.resource::<CommandPaletteState>().input);
            let candidate_key = if committed.is_empty() {
                ch.to_string()
            } else {
                format!("{} {ch}", committed.join(" "))
            };
            if let Some(entry) = entries.iter().find(|e| e.key == candidate_key).cloned() {
                select_entry(world, &entry);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records the `target` the fake command's handler was invoked with. `None` at the outer
    /// level means the handler never ran at all.
    #[derive(Resource, Default)]
    struct Recorded(Option<Option<Target>>);

    fn record_invocation(In(invocation): In<CommandInvocation>, mut recorded: ResMut<Recorded>) {
        recorded.0 = Some(invocation.target);
    }

    /// Sets up a world with a "g" command whose outcome is `PickTarget`, and location `h`
    /// assigned to `loc`, mirroring what a real game's goto-style command looks like once
    /// `LocationLabels` has an assignment.
    fn make_pick_target_app(loc: Vec2) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<Recorded>();
        app.init_resource::<CommandPaletteState>();
        app.init_resource::<LocationLabels>();
        app.init_resource::<EntityLabels>();
        app.init_resource::<PaletteRegistry>();

        let handler = app.world_mut().register_system(record_invocation);
        app.world_mut().resource_mut::<LocationLabels>().slots[(b'h' - b'a') as usize] = Some(loc);
        app.world_mut().resource_mut::<PaletteRegistry>().commands.push(PaletteCommand {
            key: "g".to_string(),
            description: "Go to".to_string(),
            icon: None,
            outcome: EntryOutcome::PickTarget { verb: "Go to".to_string(), filter: TargetFilter::Any },
            handler,
        });
        app
    }

    /// Regression test: selecting a target-entry built the way `build_target_entries` builds
    /// them (`outcome: EntryOutcome::Run`, with the target letter only in `key`'s text, not in
    /// `outcome`) must still dispatch the resolved `Target` to the handler — not silently drop
    /// it. This is exactly the path `handle_palette_keyboard` takes for typed-letter and
    /// arrow+Enter selection, and `palette_system` takes for a clicked row.
    #[test]
    fn select_entry_on_target_entry_resolves_and_dispatches_target() {
        let loc = Vec2::new(42.0, 7.0);
        let mut app = make_pick_target_app(loc);

        let entry = PaletteEntry {
            key: "g h".to_string(),
            description: "Go to location h".to_string(),
            icon: None,
            outcome: EntryOutcome::Run,
        };
        select_entry(app.world_mut(), &entry);

        let recorded = app.world().resource::<Recorded>();
        assert_eq!(
            recorded.0,
            Some(Some(Target::Point(loc))),
            "handler should have been invoked with the resolved target, not None"
        );

        let state = app.world().resource::<CommandPaletteState>();
        assert!(!state.open, "palette should close after a successful dispatch");
    }

    /// `execute_path_string` (used by the headless scripting harness) exercises the same
    /// target-letter resolution as `select_entry` — verify they agree.
    #[test]
    fn execute_path_string_resolves_location_letter() {
        let loc = Vec2::new(42.0, 7.0);
        let mut app = make_pick_target_app(loc);

        let ran = execute_path_string(app.world_mut(), "g h");
        assert!(ran, "execute_path_string should report success for a resolvable target letter");

        let recorded = app.world().resource::<Recorded>();
        assert_eq!(recorded.0, Some(Some(Target::Point(loc))));
    }

    /// A letter with no assigned location (e.g. never explored) must not silently dispatch
    /// with no target — `execute_path_string` should report failure and leave the handler
    /// un-run, matching `select_entry`'s "do nothing" behavior for the same case.
    #[test]
    fn execute_path_string_fails_for_unassigned_letter() {
        let mut app = make_pick_target_app(Vec2::ZERO);
        // "z" was never assigned a location.
        let ran = execute_path_string(app.world_mut(), "g z");
        assert!(!ran);
        assert_eq!(app.world().resource::<Recorded>().0, None, "handler should not have run");
    }
}
