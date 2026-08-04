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

use bevy::{
    ecs::system::SystemId,
    input::{ButtonState, keyboard::{Key, KeyboardInput}},
    prelude::*,
};

use crate::fov::{CurrentFovState, ExplorationState};

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

// ── Shared label-map scan: first free letter from a range, reverse lookup ──

/// The first of `candidates` not already used as a value in `letters` — the allocation policy
/// shared by `EntityLabels` and `LabelPool<K>`, which otherwise differ only in their key type
/// and candidate letter range.
fn first_unused_letter<K>(letters: &HashMap<K, char>, candidates: impl Iterator<Item = char>) -> Option<char> {
    candidates.into_iter().find(|c| !letters.values().any(|l| l == c))
}

/// The key mapped to `letter`, if any — the reverse-lookup half of the same shared shape.
fn key_for_letter<K: Copy>(letters: &HashMap<K, char>, letter: char) -> Option<K> {
    letters.iter().find(|&(_, &l)| l == letter).map(|(&k, _)| k)
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
        if let Some(letter) = first_unused_letter(&self.letters, ('1'..='9').chain('A'..='Z')) {
            self.letters.insert(entity, letter);
        }
    }

    pub fn letter_for(&self, entity: Entity) -> Option<char> { self.letters.get(&entity).copied() }

    pub fn entity_for_letter(&self, letter: char) -> Option<Entity> {
        key_for_letter(&self.letters, letter)
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
        let letter = first_unused_letter(&self.letters, 'a'..='z')?;
        self.letters.insert(key, letter);
        Some(letter)
    }

    pub fn get(&self, key: K) -> Option<char> { self.letters.get(&key).copied() }

    pub fn key_for_letter(&self, letter: char) -> Option<K> { key_for_letter(&self.letters, letter) }
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
    /// A `PickTarget` node's target-letter completion, as built by `build_target_entries`:
    /// `root` is the command path that led to the `PickTarget` (i.e. `key` minus its trailing
    /// letter), and `target` is the already-resolved target — carried as data here rather than
    /// left for the dispatcher to re-parse out of `key`'s display text.
    RunTarget { root: PalettePath, target: Target },
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
    crate::fov::is_currently_visible(world.get_resource::<CurrentFovState>(), pos)
}

/// Whether `pos` has ever been seen, per `ExplorationState::is_explored` — the same criterion
/// `compute_goto_assignments`-style policy code uses to decide which points get a location
/// letter in the first place. `LocationLabels` entries are static points, not mobile like
/// `Targetable` entities, so once explored they stay selectable even when no longer in the
/// player's *current* FOV; requiring both would defeat the point of a remembered waypoint.
fn is_explored(world: &World, pos: Vec2) -> bool {
    world.get_resource::<ExplorationState>().is_none_or(|exp| exp.is_explored(pos))
}

fn build_target_entries(
    world: &mut World,
    stem: &[String],
    verb: &str,
    filter: TargetFilter,
) -> Vec<PaletteEntry> {
    let mut entries = Vec::new();
    let prefix = stem.join(" ");

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
                outcome: EntryOutcome::RunTarget {
                    root: stem.to_vec(),
                    target: Target::Entity(entity),
                },
            });
        }
    }

    if filter != TargetFilter::EntitiesOnly {
        let loc_labels = world.resource::<LocationLabels>();
        let slots = loc_labels.slots;
        for (i, opt_pos) in slots.iter().enumerate() {
            let Some(pos) = opt_pos else { continue };
            if !is_explored(world, *pos) {
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
                outcome: EntryOutcome::RunTarget { root: stem.to_vec(), target: Target::Point(*pos) },
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
        EntryOutcome::Run | EntryOutcome::RunTarget { .. } => None,
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
            build_target_entries(world, committed, &verb, filter)
        }
        Some(EntryOutcome::Run) | Some(EntryOutcome::RunTarget { .. }) | None => Vec::new(),
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

/// Given a chosen entry (from typing, arrow+Enter, or a clicked row), either
/// navigate deeper or execute — shared by every input source that picks an
/// entry by its already-known `PaletteEntry`.
pub fn select_entry(world: &mut World, entry: &PaletteEntry) {
    match &entry.outcome {
        EntryOutcome::Run => run_command(world, committed_path(&entry.key), None),
        EntryOutcome::RunTarget { root, target } => {
            run_command(world, root.clone(), Some(*target));
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

/// Directly execute a full space-separated command path (e.g. `"w a A"`), bypassing the
/// keyboard/click UI entirely — for programmatic driving such as a headless scripting harness.
/// If the path's stem resolves to a `PickTarget` node, the last token is looked up against the
/// *actual* entries `compute_entries` would show interactively (the same ones a player would
/// pick from), so this can never resolve a target differently than the UI does. Otherwise the
/// whole path must already resolve to `Run`. Returns `false` if the path doesn't resolve to a
/// runnable command (in which case nothing happens).
pub fn execute_path_string(world: &mut World, input: &str) -> bool {
    let path = committed_path(input);
    if path.is_empty() {
        return false;
    }
    if path.len() >= 2
        && let Some(EntryOutcome::PickTarget { .. }) = resolve_outcome(world, &path[..path.len() - 1])
    {
        let stem = &path[..path.len() - 1];
        let full_key = path.join(" ");
        let Some(EntryOutcome::RunTarget { root, target }) =
            compute_entries(world, stem).into_iter().find(|e| e.key == full_key).map(|e| e.outcome)
        else {
            return false;
        };
        run_command(world, root, Some(target));
        return true;
    }
    if matches!(resolve_outcome(world, &path), Some(EntryOutcome::Run)) {
        run_command(world, path, None);
        return true;
    }
    false
}

/// Escape: close the palette and discard whatever was typed.
pub fn close_on_escape(keyboard: Res<ButtonInput<KeyCode>>, mut state: ResMut<CommandPaletteState>) {
    if state.open && keyboard.just_pressed(KeyCode::Escape) {
        state.open = false;
        state.input.clear();
        state.selected_idx = 0;
    }
}

/// One relevant keystroke, in the arrival order `KeyboardInput` events carry — unlike
/// `ButtonInput<T>`'s `just_pressed`, which is backed by a `HashSet` with no ordering guarantee.
/// That distinction is the whole reason this type (and reading `EventReader<KeyboardInput>`
/// instead of `ButtonInput`) exists: when two character keys are pressed within the same Bevy
/// frame — a real occurrence under a frame hitch or just fast typing — `ButtonInput` has already
/// destroyed which one came first by the time a system reads it. Concretely, typing "g" then "h"
/// in the same frame used to sometimes have `find_map` over `get_just_pressed()` see "h" before
/// "g": "h" isn't a registered root command, so it was silently discarded, and "g" alone survived
/// to open the palette a frame late with nothing left to complete it — reproducing the reported
/// "'g h' behaves like just tapping 'h'" bug. Reading the ordered event stream instead means "g"
/// is always processed (opening the palette) before "h" is processed (completing it).
#[derive(Clone, Copy)]
enum PaletteKey {
    Space,
    Char(char),
    Up,
    Down,
    Enter,
    Backspace,
}

/// Typing, backspace, arrow navigation, and Enter — the palette's entire keyboard state machine,
/// including opening it. (A single system, not the previous open/type split, because a same-frame
/// sequence like "g" then "h" needs the "g" event's `state.open` transition to be visible to the
/// "h" event processed right after it — two separate systems can't do that without also relying on
/// `ButtonInput`'s unordered `just_pressed`, which is the bug this replaced.)
///
/// Every event is resolved against freshly recomputed entries (`compute_entries`), not the
/// once-per-frame `CurrentPaletteEntries` snapshot `update_palette_entries` produces for
/// rendering — that snapshot is stale for a second keystroke landing in the same frame.
pub fn handle_palette_keyboard(mut key_events: MessageReader<KeyboardInput>, mut commands: Commands) {
    let ordered: Vec<PaletteKey> = key_events
        .read()
        .filter(|e| e.state == ButtonState::Pressed && !e.repeat)
        .filter_map(|e| match &e.logical_key {
            Key::Space => Some(PaletteKey::Space),
            Key::Character(s) => s.chars().next().map(PaletteKey::Char),
            Key::ArrowUp => Some(PaletteKey::Up),
            Key::ArrowDown => Some(PaletteKey::Down),
            Key::Enter => Some(PaletteKey::Enter),
            Key::Backspace => Some(PaletteKey::Backspace),
            _ => None,
        })
        .collect();
    if ordered.is_empty() {
        return;
    }
    commands.queue(move |world: &mut World| {
        for key in ordered {
            handle_one_palette_key(world, key);
        }
    });
}

/// Opens the palette on Space (empty), on a `Targetable` entity's bare letter (runs
/// `DefaultEntityAction`, if the game set one), or on a registered root command's key (instant
/// commands like "." fire immediately; everything else opens with that key typed). While open:
/// typing a character that exactly names one of the current entries auto-selects it immediately
/// — matching the "type the whole thing, no Enter needed" feel the single-character-token grammar
/// is built around. Enter does the same for whatever's currently typed or reached via arrow nav.
fn handle_one_palette_key(world: &mut World, key: PaletteKey) {
    if !world.resource::<CommandPaletteState>().open {
        match key {
            PaletteKey::Space => {
                let mut state = world.resource_mut::<CommandPaletteState>();
                state.open = true;
                state.input.clear();
                state.selected_idx = 0;
            }
            PaletteKey::Char(ch) => open_palette_for_char(world, ch),
            PaletteKey::Up | PaletteKey::Down | PaletteKey::Enter | PaletteKey::Backspace => {}
        }
        return;
    }

    match key {
        PaletteKey::Space => {}
        PaletteKey::Up | PaletteKey::Down => {
            let committed = committed_path(&world.resource::<CommandPaletteState>().input);
            let entries = compute_entries(world, &committed);
            let n = entries.len();
            if n == 0 {
                return;
            }
            let current_key = world.resource::<CommandPaletteState>().input.trim().to_string();
            let current = entries.iter().position(|e| e.key == current_key).unwrap_or(0);
            let next =
                if matches!(key, PaletteKey::Down) { (current + 1) % n } else { (current + n - 1) % n };
            let mut state = world.resource_mut::<CommandPaletteState>();
            state.input = entries[next].key.clone();
            state.selected_idx = next;
        }
        PaletteKey::Backspace => {
            let mut state = world.resource_mut::<CommandPaletteState>();
            state.input.pop();
            if state.input.ends_with(' ') {
                state.input.pop();
            }
        }
        PaletteKey::Enter => {
            let committed = committed_path(&world.resource::<CommandPaletteState>().input);
            let entries = compute_entries(world, &committed);
            let current_key = world.resource::<CommandPaletteState>().input.trim().to_string();
            if let Some(entry) = entries.into_iter().find(|e| e.key == current_key) {
                select_entry(world, &entry);
            }
        }
        PaletteKey::Char(ch) => {
            let input = world.resource::<CommandPaletteState>().input.clone();
            let committed = committed_path(&input);
            let candidate_key = if committed.is_empty() {
                ch.to_string()
            } else {
                format!("{} {ch}", committed.join(" "))
            };
            let entries = compute_entries(world, &committed);
            if let Some(entry) = entries.into_iter().find(|e| e.key == candidate_key) {
                select_entry(world, &entry);
            }
        }
    }
}

fn open_palette_for_char(world: &mut World, ch: char) {
    let entity = world.resource::<EntityLabels>().entity_for_letter(ch);
    if let Some(entity) = entity {
        let default_key = world.resource::<DefaultEntityAction>().0.clone();
        if let Some(default_key) = default_key {
            run_command(world, vec![default_key], Some(Target::Entity(entity)));
            return;
        }
    }

    let matched = world
        .resource::<PaletteRegistry>()
        .commands
        .iter()
        .find(|c| c.key == ch.to_string())
        .map(|c| (c.key.clone(), matches!(c.outcome, EntryOutcome::Run)));
    let Some((key, is_run)) = matched else { return };

    if is_run {
        run_command(world, vec![key], None);
    } else {
        let mut state = world.resource_mut::<CommandPaletteState>();
        state.open = true;
        state.input = format!("{ch} ");
        state.selected_idx = 0;
    }
}

#[cfg(test)]
mod tests {
    use bevy::input::keyboard::NativeKeyCode;

    use super::*;

    /// Records the `target` the fake command's handler was invoked with. `None` at the outer
    /// level means the handler never ran at all.
    #[derive(Resource, Default)]
    struct Recorded(Option<Option<Target>>);

    fn record_invocation(In(invocation): In<CommandInvocation>, mut recorded: ResMut<Recorded>) {
        recorded.0 = Some(invocation.target);
    }

    /// A "key `ch` was pressed" `KeyboardInput` event. `key_code`/`text`/`window` are irrelevant
    /// to `handle_palette_keyboard` (it only reads `logical_key` and `state`), so they're filled
    /// with placeholders.
    fn char_key_event(ch: char) -> KeyboardInput {
        KeyboardInput {
            key_code: KeyCode::Unidentified(NativeKeyCode::Unidentified),
            logical_key: Key::Character(ch.to_string().into()),
            state: ButtonState::Pressed,
            text: None,
            repeat: false,
            window: Entity::PLACEHOLDER,
        }
    }

    fn send_key(app: &mut App, event: KeyboardInput) {
        app.world_mut().resource_mut::<Messages<KeyboardInput>>().write(event);
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
        app.init_resource::<LocationDescriptions>();
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

    /// Regression test: selecting a real target-entry (as `build_target_entries` produces,
    /// `outcome: EntryOutcome::RunTarget`) must dispatch its carried `Target` to the handler.
    /// This is exactly the path `handle_palette_keyboard` takes for typed-letter and
    /// arrow+Enter selection, and `palette_system` takes for a clicked row.
    #[test]
    fn select_entry_on_target_entry_resolves_and_dispatches_target() {
        let loc = Vec2::new(42.0, 7.0);
        let mut app = make_pick_target_app(loc);

        // Use the real entry `compute_entries` (== `build_target_entries`) produces, the same
        // one the interactive UI would show and select from — not a hand-built stand-in.
        let entries = compute_entries(app.world_mut(), &["g".to_string()]);
        let entry = entries.into_iter().find(|e| e.key == "g h").expect("g h should be a candidate");
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

    /// Regression test: `LocationLabels` entries (static, remembered points) must stay
    /// selectable once explored even after the player looks away — unlike `Targetable`
    /// entities, which do need to be currently in FOV since they can move. Before the fix,
    /// `build_target_entries` gated *both* on `CurrentFovState`, so a location dropped out of
    /// the palette's candidate list the moment it left the player's current view, even though
    /// `LocationLabels` itself (assigned by exploration, not current visibility) still had it.
    #[test]
    fn build_target_entries_includes_explored_but_out_of_current_fov_location() {
        use crate::util::safegeo::SafeMultiPolygon;

        let loc = Vec2::new(42.0, 7.0);
        let mut app = make_pick_target_app(loc);
        // Nothing is currently visible...
        app.insert_resource(CurrentFovState(SafeMultiPolygon::empty()));
        // ...but nothing is unexplored either (i.e. everything has been seen at some point).
        app.insert_resource(ExplorationState(SafeMultiPolygon::empty()));

        let entries = compute_entries(app.world_mut(), &["g".to_string()]);
        assert!(
            entries.iter().any(|e| e.key == "g h"),
            "an explored-but-not-currently-visible location should still be a selectable entry, got {:?}",
            entries.iter().map(|e| &e.key).collect::<Vec<_>>()
        );
    }

    /// Sets up a world identical to `make_pick_target_app`, but wired for the interactive
    /// keyboard path (`handle_palette_keyboard` reading `KeyboardInput` events) instead of the
    /// programmatic `select_entry`/`execute_path_string` paths the earlier tests exercise.
    fn make_keyboard_app(loc: Vec2) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<KeyboardInput>();
        app.init_resource::<Recorded>();
        app.init_resource::<CommandPaletteState>();
        app.init_resource::<LocationLabels>();
        app.init_resource::<EntityLabels>();
        app.init_resource::<PaletteRegistry>();
        app.init_resource::<DefaultEntityAction>();
        app.init_resource::<CurrentPaletteEntries>();
        app.init_resource::<LocationDescriptions>();

        let handler = app.world_mut().register_system(record_invocation);
        app.world_mut().resource_mut::<LocationLabels>().slots[(b'h' - b'a') as usize] = Some(loc);
        app.world_mut().resource_mut::<PaletteRegistry>().commands.push(PaletteCommand {
            key: "g".to_string(),
            description: "Go to".to_string(),
            icon: None,
            outcome: EntryOutcome::PickTarget { verb: "Go to".to_string(), filter: TargetFilter::Any },
            handler,
        });

        app.add_systems(Update, (handle_palette_keyboard, update_palette_entries).chain());
        app
    }

    /// Regression test for the interactive keyboard path specifically (not `select_entry` or
    /// `execute_path_string` directly, both already covered above): opening the palette on a
    /// command letter that also happens to be an assigned, selectable location letter (e.g.
    /// "g" the command colliding with location letter "g") must only seed the input — it must
    /// not *also* immediately self-select and run that location as the target.
    #[test]
    fn opening_palette_does_not_self_select_a_colliding_letter() {
        let loc = Vec2::new(5.0, 5.0);
        // Location letter "g" collides with the command's own key "g".
        let mut app = make_keyboard_app(loc);
        app.world_mut().resource_mut::<LocationLabels>().slots[(b'g' - b'a') as usize] = Some(loc);

        send_key(&mut app, char_key_event('g'));
        app.update();

        let state = app.world().resource::<CommandPaletteState>();
        assert_eq!(state.input, "g ", "the opening keystroke should only seed the input");
        assert!(state.open, "palette should still be open, awaiting the target letter");

        let recorded = app.world().resource::<Recorded>();
        assert_eq!(
            recorded.0, None,
            "the command handler must not have run yet from the single opening keystroke"
        );
    }

    /// Full two-keystroke sequence across two frames, matching an actual player typing "g"
    /// then "h" with a pause in between: presses "g" in frame 1, then "h" in frame 2, and checks
    /// the second keystroke actually navigates.
    #[test]
    fn typing_g_then_h_across_two_frames_navigates_to_location_h() {
        let loc = Vec2::new(9.0, 3.0);
        let mut app = make_keyboard_app(loc);

        send_key(&mut app, char_key_event('g'));
        app.update();

        let state = app.world().resource::<CommandPaletteState>();
        assert_eq!(state.input, "g ", "frame 1 should have opened and seeded the input");
        assert!(state.open);

        send_key(&mut app, char_key_event('h'));
        app.update();

        let recorded = app.world().resource::<Recorded>();
        assert_eq!(
            recorded.0,
            Some(Some(Target::Point(loc))),
            "typing 'h' after 'g' should navigate to location h, not act like a bare 'h' tap"
        );
        let state = app.world().resource::<CommandPaletteState>();
        assert!(!state.open, "palette should have closed after the successful dispatch");
    }

    /// The actual regression this fix targets: "g" and "h" landing as two separate
    /// `KeyboardInput` events *in the same frame* (a real occurrence — a frame hitch or fast
    /// typing can coalesce multiple keydowns into one frame) must still be processed in arrival
    /// order, opening on "g" and then completing with "h", rather than losing one of them. Before
    /// the fix, both keys were read via `ButtonInput::get_just_pressed()`, backed by an unordered
    /// `HashSet` — if "h" happened to be visited before "g", it matched no root command, was
    /// silently discarded, and "g" alone survived to open the palette a frame late with the "h"
    /// keystroke already lost. This is exactly the "'g h' behaves like just tapping 'h'" bug
    /// report; reading `EventReader<KeyboardInput>` (arrival-ordered, unlike `ButtonInput`) and
    /// processing every event from a single frame sequentially against live state fixes it.
    #[test]
    fn typing_g_then_h_in_the_same_frame_navigates_to_location_h() {
        let loc = Vec2::new(11.0, -4.0);
        let mut app = make_keyboard_app(loc);

        send_key(&mut app, char_key_event('g'));
        send_key(&mut app, char_key_event('h'));
        app.update();

        let recorded = app.world().resource::<Recorded>();
        assert_eq!(
            recorded.0,
            Some(Some(Target::Point(loc))),
            "'g' then 'h' in the same frame should navigate to location h, not act like a bare 'h' tap"
        );
        let state = app.world().resource::<CommandPaletteState>();
        assert!(!state.open, "palette should have closed after the successful dispatch");
    }
}
