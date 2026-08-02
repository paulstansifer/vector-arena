# Splitting Vector Arena into `rogue-angles` + a demo game

Status: **in progress.** Phases 0–6 done (2 absorbed phase 3 — see below);
phases 7–8 pending.

## Phase 6 notes (presentation split)

Two moves, both landed as one commit.

**SVG-to-texture rendering pipeline** (`rogue_angles::sprite`): this was the
one architecturally significant call in this phase, surfaced to the user
before implementation rather than decided unilaterally — `rogue-angles` had
zero UI-toolkit dependency until now (the command palette deliberately emits
plain data, letting the game decide how to render it), and moving
`sprite.rs`'s `bevy_svg`/`resvg`-based pipeline into the engine means every
game built on it inherits an opinionated egui/SVG icon system whether it
wants one or not. The user chose to move it in, so `rogue-angles` now
depends on `bevy_egui`/`bevy_svg`/`resvg`/`egui_extras`, and owns
`SvgSprite`/`SpriteEguiData`/`SpriteCache`, SVG parameterization
(color/text substitution), tessellation to a `bevy_svg` mesh, and
rasterization to an egui texture — plus the embedded default font
(`LiberationSans-Regular.ttf`, moved from `vector-arena/fonts/` to
`rogue-angles/fonts/`) needed to render text-parameterized SVGs reliably.

What the engine still doesn't own: any actual sprite art. A game registers
its own SVG bytes into the new `SvgSource` resource (by the same
`svg_path` string `SvgSprite` already carried, typically via
`include_bytes!` in a `Startup` system), rather than the engine shipping
sprites of its own or hardcoding a path→bytes match statement the way
`vector-arena`'s old `get_embedded_svg` did. `vector-arena`'s `sprite.rs`
shrank to just its own vocabulary: `sprite_spec`/`potion_hex`/
`scroll_letter`/`wand_hex` (which `ItemKind` maps to which asset/param),
`SpriteEguiTextures` (an `ItemKind`-keyed cache the HUD/palette read,
separate from the engine's own `(path, param)`-keyed one), and
`register_svg_assets`.

The world-mesh half of the pipeline (`SvgSpritePlugin`: `bevy_svg`'s own
plugin + `SpriteCache`/`SvgSource` + `insert_svg_components`) and the
egui-texture half (`register_egui_sprites`, a plain system) are separate on
purpose: the world-mesh half needs no egui context and is safe to add in
headless mode (matching how `GamePlugin` already split egui-dependent vs.
egui-independent systems for the headless runner), while the egui half only
makes sense wherever a game actually wires up `EguiPrimaryContextPass`.
Syncing the engine's per-entity `SpriteEguiData` into the game's
`ItemKind`-keyed `SpriteEguiTextures` is a small lazy game-side system
(`sync_item_egui_textures`) that checks a `HashMap` entry rather than an
`Added<SpriteEguiData>` query filter chained in the same frame — the
engine's insert goes through `Commands`, which isn't guaranteed visible to
`Added<T>` in a later system of the same pass without an explicit sync
point, so the idempotent lazy check sidesteps that ordering hazard entirely
rather than risking a one-frame-late icon.

**Generic HUD chrome** (`rogue_angles::hud`): `MessageLog` (already
`String`-only, flagged since phase 1 as "should move but nothing forced
it"), `WorldTooltip` + `show_world_entity_tooltip` (proximity-based world
tooltips, already only touching `Transform` and the engine's own
`fov::CurrentFovState`), and `draw_stat_bar` (a labeled progress-bar
primitive, now parameterized by `width`/`height` instead of hardcoding this
game's specific bar dimensions) all moved with no behavior change. HP/MP/
boredom bars, item icons, the depth/descend/menu bar, and the game-over
screen stay entirely in `vector-arena`'s `ui.rs`, as planned — those are
this game's vocabulary, not chrome.

**Not done, and not needed**: the plan flagged "untangle the `ui.rs` ↔
`command_palette.rs` cycle" as a phase-6 task. Investigated first rather
than done reflexively — there is no cycle. `command_palette.rs` (already a
thin egui shell over the engine's `palette` module since phase 2+3) doesn't
import from `ui.rs` at all; `ui.rs` only references
`command_palette::palette_system`/`handle_world_click_for_palette` for
system registration, which isn't a type-level dependency. This was
apparently resolved as a side effect of the phase 2+3 palette rework, not
something left over to fix here.

Verification: `cargo test --workspace` 139/139 green throughout both
sub-changes (test counts shifted between crates as `MessageLog`'s tests
moved with it — same total). Two headless smoke tests (`wait 2s; snap ...`),
one after each sub-change, visually confirm sprites, doors, and the goto
command all still work correctly with the pipeline now living in the engine.

One environment note from this phase, unrelated to the code: mid-phase the
session's disk allowance was exhausted by accumulated `target/` build
artifacts (28G), which surfaced as a rustc internal compiler error
("No space left on device") that could easily be misread as a real
compilation bug. Deleting `target/` and rebuilding from scratch resolved it
immediately with no code changes — worth remembering if a future phase hits
the same wall, especially since this phase's dependency additions
(`bevy_egui`/`bevy_svg`/`resvg` now built twice into the dependency graph
briefly during the transition) made the build artifacts unusually large.

## Phase 5 notes (status-effect and item-identification frameworks)

Two independent genericizations, landed together since both are small and
touch disjoint files.

**Status effects** (`rogue_angles::status_effects`): the engine now owns
`StatusEffects<K>`/`ActiveStatusEffect<K>` (a timed collection that ramps
strength down over `RAMP_DOWN_SECS` before expiring) and the generic
`tick_status_effects::<K>` system, registered per-game via
`app.add_status_effects::<K>()`. A game's `K` only needs to implement
`StatusKind`, telling the engine which of its variants affect movement speed
or vision (`speed_factor`/`vision_factor`, both optional) and supplying a
`tick` hook for anything that needs per-frame mutation (this game's Confused
wander-direction random walk). Everything else about a game's effects —
missile damage multipliers, blindness, displacement — the engine has no
opinion on; `StatusEffects<K>` exposes generic `strength_of`/`multiplier_of`
helpers so the game builds its own aggregates without the engine knowing
what they mean. `vector-arena`'s `status_effect.rs` is now ~40 lines shorter
and contains only the `StatusEffect` enum, its `StatusKind` impl, and the
free functions (`blind_strength`, `missile_multiplier`, `confusion_strength`,
`displacing_strength`, `confused_strength_and_dir`) that read those generic
helpers back out — free functions rather than inherent methods on
`StatusEffects<StatusEffect>` because Rust's orphan rules don't allow a
downstream crate to add inherent impls to a foreign generic type, even at a
concrete instantiation. `sync_movement_modifiers` (composing the status
effect aggregate with the separate, terrain-owned `TorporMultiplier` into
`MovementModifiers`) stays a game-side system, unchanged in shape — the
engine's generic system only ticks durations, not the eventual composition
with a second, game-specific modifier source.

**Item identification** (`rogue_angles::identity`): `IdentityTable<A, E>` is
a per-run shuffled `A → E` bijection plus which `A`s are identified, with
`randomize`/`effect_of`/`is_identified`/`identify`/`forget_some` (the last
absorbing `derange_indices`, also moved to the engine). `vector-arena`'s
`ItemIdentities` now holds three `IdentityTable`s (potion, scroll, wand)
instead of three raw `HashMap`s plus one `ItemKind`-keyed identified set;
its public API (`is_identified`/`identify`/`forget`, all keyed by the game's
umbrella `ItemKind`) is unchanged, so every call site outside `item.rs`
needed no changes at all. `Inventory` stays entirely game-side, as planned.

One deviation, in the same spirit as phase 4's: `forget_some` samples its
scrambled subset from an internal `HashSet`, whereas the original sampled
from a fixed-order `Vec` built by filtering `ALL_POTION_COLORS`/
`ALL_SCROLL_NAMES` in their declared order. The *set* of eligible
appearances and the probability distribution over which get chosen are
identical; only the RNG call sequence for a given seed can differ. No test
or gameplay behavior depends on that exact sequence (the existing
`forget_scrambles_three_known_potions` test asserts properties — exactly
three scrambled, each with a genuinely different effect, bijection intact —
not specific outcomes for a specific seed), so this wasn't worth the extra
complexity of threading a stable order through the engine's table.

Verification: `cargo test --workspace` 139/139 green throughout, zero
warnings. Headless smoke test (`wait 2s; snap ...; cmd g h`) confirms the
game still starts, generates a level, and resolves a goto command with the
new status-effect and identity plumbing wired in.

## Phase 4 notes (level generation: `RoomKind` + `LevelBuilder` + `LevelPlan`)

`crates/vector-arena/src/dungeon/level_generation.rs` (1758 lines) is gone —
its framework half moved to `crates/rogue-angles/src/dungeon/level_generation.rs`
and its eight room variants became stock `RoomKind` implementors under
`crates/rogue-angles/src/dungeon/rooms/{normal,oval,colonnade,slow_zone,octagon,
vault,rubble,ring}.rs`. The 6×-duplicated connection-carving block is now one
method, `LevelBuilder::carve_connection`. `TerrainGeometry`'s 9-field/8-tuple
mismatch is gone too — `LevelPlan` has named fields plus tagged
`markers`/`regions` (`HashMap<&'static str, Vec<_>>`) so a custom `RoomKind`
can introduce new gameplay output (a game-specific vocabulary, e.g. a new
kind of trap floor) without an engine change. `random_room_variant`'s
`gen_range(0..6)` + `unreachable!()` became a weighted draw over
`RoomRegistry`, chosen per-partition in `allocate_roles` and carried on
`PartitionRole::Room { kind: Arc<dyn RoomKind> }`.

`rooms` is a **sibling** of `level_generation`, not a submodule nested inside
it — deliberately, so `LevelBuilder`'s four fields stay genuinely private
(Rust's descendant-module visibility rule would have let a nested `rooms`
reach into them, defeating the "stock rooms use only the public API" rule).
Stock rooms only ever call `LevelBuilder`'s `add_*`/`carve_connection`/
`carve_entry`/`carve_full_wall_opening` methods, never touch its fields; a
`floor()`/`doors()`/`markers()`/`regions()` read accessor set exists
alongside the mutators, primarily so each room's own unit tests (ported
inline into `rooms/octagon.rs` and `rooms/vault.rs`, calling `RoomKind::carve`
directly on a fresh `LevelBuilder` rather than going through the full BSP
pipeline) can assert on the result — exactly the ergonomics Style A was
chosen for.

Two intentional, documented deviations from bit-for-bit behavior (both
covered by the "exact per-seed reproduction not required" relaxation the
user confirmed when settling the trait shape):

- **`SlowZoneRoom` (formerly Torpor) now uses the same `carve_connection` as
  every other room**, including its double-door branch. The original Torpor
  arm was a near-copy of the shared block that silently dropped the
  double-door case — normalizing it removes an inconsistency rather than
  porting a likely-unintentional gap.
- **`OvalRoom` keeps its `weight() == 0.0`** (never drawn by
  `RoomRegistry::stock()`), preserving the pre-existing exclusion
  (`random_room_variant`'s comment cites an unresolved performance issue).
  It's still fully implemented and registered, so a game can give it nonzero
  weight once that's investigated — not this phase's problem to fix.

**Deferred, not done:** the plan's `PopulateLevel` triggered-event observer.
`spawn_game_world` still calls `populate_level::populate(...)` directly with
plain data pulled from `LevelPlan` (`&terrain_geometry.rooms`,
`.playable_area`, `.markers["vault_center"]`) rather than
`commands.trigger(PopulateLevel { plan })` dispatching to a game-registered
observer. The event-based indirection is about letting a third-party game
hook population without the engine calling into game code by name — a real
goal, but a separate concern from proving the `RoomKind` extension point,
which is this phase's core deliverable and the piece every other phase-4
design decision was validated against. Left for a follow-up pass rather than
this phase's scope creeping further.

Verification: `cargo test --workspace` is **139/139 green** — the pre-phase
138 (117 rogue-angles + 13 vector-arena lib + 8 across
`fov_performance`/`game_tests`/`integration_tests`/`rope_tests`) plus one new
test (`rooms::rubble::tests::stays_open_and_yields_movable_pieces`) that
closes a gap the port would otherwise have left: nothing lost, and the one
addition is a straight port of the pre-existing rubble-room behavioral spec.
The five variant-specific tests
(`test_octagon_bevels_corners_with_no_nearby_connection`,
`test_octagon_skips_bevel_near_connection`,
`test_chamber_carves_vault_with_single_door_and_records_center`,
`test_rubble_room_stays_open_and_yields_movable_pieces`, and
`test_ring_room_has_solid_center_and_playable_walkway`) now call
`RoomKind::carve` directly rather than the full `render()` pipeline.
`cargo run --bin headless -- 'wait 1s; snap ...; cmd g h'` produced a level
with rooms, corridors, a glass wall, and a monster/items placed correctly,
and the goto command round-tripped through a freshly generated level (visual
check, not a pixel diff, per the plan).

## Phase 2 notes (merged with phase 3, and one scripting-capability fix found along the way)

**Phases 2 and 3 were done together, not separately as originally planned.**
The reason surfaced during implementation, not before: `EntryOutcome::PickTarget`
— the palette's built-in target picker — is structurally meaningless without
engine-owned labels to enumerate. Building the palette tree in phase 2 against
the still-game-side `LetterMap`/`GotoState`, then rebuilding it against engine
labels in phase 3, would have been the same work twice. So phase 2 shipped the
full `rogue_angles::palette` module: `EntryOutcome` (`Submenu`/`PickTarget`/`Run`),
`PaletteCommand`/`PaletteEntry`/`CommandInvocation`/`Target`, `EntityLabels`
(uppercase/digits, auto-assigned via a `Targetable` marker + `Added`/
`RemovedComponents`, no more manual `release_monster` calls), `LabelPool<K>`
(generic lowercase pool — the demo instantiates `LabelPool<ItemKind>`, the
engine never sees `ItemKind`), and `LocationLabels`/`LocationDescriptions`
(the waypoint storage, `DIR_LEFT`/`DIR_RIGHT`/… constants for the eight pinned
direction slots). Execution is no longer a polled `pending_command` mailbox —
the engine resolves a path and calls the owning command's
`SystemId<In<CommandInvocation>>` handler directly, so `item.rs:774`'s old
"put the command back if it isn't mine" workaround is gone along with the
string-parsing it existed to route around.

The demo's eight commands map onto the tree as: `q`/`r`/`e` are
`Submenu(list matching inventory) → Run`; `w` is
`Submenu(list wands) → PickTarget → Run` (waving a wand needs a target, so its
submenu entries lead to `PickTarget` instead of `Run` directly); `g`/`z` are
`PickTarget` at the root; `d`/`.` are `Run` at the root. Rendering (egui) stays
entirely in the game — `rogue_angles::palette` has no UI-framework dependency —
via `IconId`, an opaque handle the demo maps back to `ItemKind` through
`ALL_ITEM_KINDS`'s index.

**A real capability gap, found by testing the headless `cmd` path rather than
assumed away:** `goto.rs`'s `compute_goto_assignments` originally only ran
while `CommandPaletteState.open && CommandPaletteWatchesClicks.0` — i.e. only
during interactive palette use. Driving the palette programmatically (the
headless runner's `cmd` script command, now `execute_path_string`, added as a
direct `path → resolve → run` entry point alongside the keyboard/click UI)
never opens that UI, so `LocationLabels` would silently stay empty and every
lowercase-letter target (`"g h"`, `"z s"`, …) would fail to resolve outside
interactive play — uppercase/digit entity targets worked fine since
`EntityLabels` was already always-live. Fixed by making location-label
assignment run unconditionally every frame, same model as `EntityLabels`,
rather than a once-per-palette-session snapshot. Confirmed via the headless
runner: `cmd g h` resolves and moves the player; `cmd g s` correctly fails
before the staircase is explored, matching pre-existing game rules, not a bug.

## Phase 1 notes (what actually moved, and three couplings resolved on the way)

`safegeo`, `bsp` (plus `PADDING`/`CORRIDOR_WIDTH`, which `level_generation.rs`
now imports back from `rogue_angles::dungeon::bsp` until it moves itself in
phase 4), `terrain`, `fov` (minus the game-specific staircase fog-of-war copy,
which stayed behind as `crates/vector-arena/src/fov.rs`), the non-egui half of
`indicator.rs` (`HitFlash`; the egui-drawn `StateIndicator` stayed in the
game), `nav`, `crumble_terrain`, and `time_scale` are now in `rogue-angles`.
`AGENT_RADIUS`, `WorldBounds`, `LevelEntity`, and `GameLayer` moved out of
`lib.rs`'s globals block; `GameState`, `Staircase`, `DungeonDepth`, and the
`WORLD_WIDTH`/`WORLD_HEIGHT` constants stayed, since nothing engine-side
actually needed them yet — moving them would have been unforced scope creep.

Three modules that looked like clean leaves weren't quite, once their
`use crate::` lists were checked against the actual game types they touched:

- **`nav.rs`** read `StatusEffects::speed_multiplier()` directly. Fixed with a
  new narrow `rogue_angles::movement::MovementModifiers` component
  (`speed_multiplier`, `vision_multiplier`) that the game's
  `status_effect::sync_movement_modifiers` system writes each frame from its
  own `StatusEffects` + `TorporMultiplier`; `nav::apply_nav_velocity` and
  `fov::update_fov` (which had the same problem via `blind_strength()`) read
  it instead. `fov::update_fov` also swapped its `With<Player>` query for a
  new `rogue_angles::movement::Viewer` marker.
- **`crumble_terrain.rs`** had an observer listening for the game's own
  `item::WandCrumblingEvent` directly. Replaced with an engine-owned
  `CrumbleTerrainRequest { target: Vec2 }` event; the game's wand-crumbling
  code triggers that instead, and the now-redundant `WandCrumblingEvent`
  wrapper was deleted.
- **`time_scale.rs`** hardcoded its whole policy by importing
  `MagicMissile`/`Player`/`MoveTarget`. Replaced with the `TimeScaleVotes`
  resource sketched below (engine takes the minimum of named votes, defaulting
  to 0.0/paused when no votes are present — the game must opt into "keep
  flowing," not the other way around); the game's `time_scale.rs` now just
  casts the same three votes it always computed (idle/moving/missile-in-flight)
  and separately tunes the physics fixed-timestep for fast projectiles, which
  stayed game-side since it's about this game's specific projectile speed, not
  a generic engine concept.

The headless snapshot runner's script parser and command dispatch (`cmd`,
`click left`, `level blank`) turned out to be a driver for *this* game
(`CommandPaletteState`, `GamePlugin`, `Player`), not generic infrastructure —
deferred rather than force-extracted now; likely pairs naturally with the
phase 6 presentation split.

Verification: `cargo test --workspace` — 138/138 passing (86 engine + 44 game
lib + 8 across the integration suites; the 86/44 split against the original
130 is an exact sanity check that nothing was lost or duplicated in the move).
Headless snapshot before/after visually identical; move/descend commands
exercise the `Viewer`/`MovementModifiers` path end-to-end without error.

Source paths below are relative to `crates/vector-arena/` unless stated
otherwise, and refer to the pre-split layout where the code still lives.

## Why

`vector-arena` grew as one ~12k-line crate that mixes two things: a reusable
substrate for *2D geometric free-movement roguelikes* (polygon terrain, BSP
level generation, raycast FOV, navmesh steering, destructible rock, a keystroke
command palette) and one specific game (potions/scrolls/wands, magic missiles,
one monster type).

The goal is a **`rogue-angles`** crate that a third party can build a
*different* game on — different combat, different inventory, new room types,
retuned level generation — while inheriting the geometry, FOV, nav, terrain, and
interaction model.

Two things are deliberately **baked in**, not pluggable:

- The **command palette**. A keystroke-completion tree is *the* interaction model.
- The **letter-and-number abbreviation scheme**. Uppercase and digits address
  entities, lowercase addresses locations and inventory slots, and a full action
  is always a space-separated string path (`"z A"`, `"w a A"`, `"g s"`). This
  string form stays canonical: it is what the headless runner scripts, and what
  any future scripting surface would drive.

Layout is a Cargo workspace in this repo; extension is by Rust traits and Bevy
plugins, compile-time, with no scripting runtime.

## The central design problem: extending without hiding ECS

The engine must call into game logic whose types it cannot name. Four
mechanisms cover every case, and none of them wrap or hide Bevy from the game
author.

### 1. `SystemId` callbacks — when the engine needs arbitrary world access

Where the engine would otherwise hardcode a game type, it holds a **one-shot
system** the game registered. The author writes an ordinary Bevy system with
whatever `Query`/`Res` parameters it likes and hands over the `SystemId`. ECS
stays fully visible; the engine just doesn't know what's inside. This is the
answer for palette submenus, command handlers, and level population.

### 2. Object-safe traits — for pure-data work with no world access

Level generation is geometry in, geometry out. `dyn RoomKind` in a registry
resource: no ECS involved, trivially unit-testable.

### 3. Generic systems over a game-defined kind — for frameworks with game vocabulary

Status effects and item identification are *shapes* the engine can own while the
game supplies the vocabulary. `StatusEffects<K>` where the game defines `K`, and
`app.add_status_effects::<K>()` instantiates the engine's ticking system for it.
Idiomatic Bevy, no dynamic dispatch, and the engine never learns what "torpor"
means.

### 4. Marker components + narrow engine-owned data

The engine owns only what its own systems must read: `Viewer` (FOV origin),
`NavAgent`, `Occluder`, `Targetable`, `MovementModifiers`. The game owns HP,
factions, inventory, damage. The engine never branches on
`Has<Player>` / `Has<Monster>`, which `apply_damage_on_hit`
(`src/effects/projectile.rs:476`) does today.

## Workspace layout

```
Cargo.toml                     # virtual workspace root (profiles live here)
docs/ENGINE-SPLIT.md           # this document
index.html, Trunk.toml         # wasm entry point, points at the demo crate
crates/
  rogue-angles/                # the engine
  vector-arena/                # the demo game: bin `game`, bin `headless`
  bevy_verlet/                 # vendored fork, excluded from the workspace
```

## What lands where

### `rogue-angles`

| Area | Source today |
|---|---|
| Sanitised geometry (`SafePolygon` etc.) | `src/util/safegeo.rs` — moves verbatim, zero crate deps |
| BSP partitioning | `src/dungeon/bsp.rs` |
| Level-gen framework + **stock room library** | `src/dungeon/level_generation.rs` |
| Terrain mesh/collider/navmesh sync, `DungeonState` | `src/dungeon/terrain.rs` |
| Destructible terrain + rubble slicing | `src/effects/crumble_terrain.rs` |
| Navmesh build, steering bridge, cost regions | `src/nav.rs` |
| FOV raycasting, exploration state, fog meshes, **auto-explore** | `src/fov.rs`, `src/player.rs:185` |
| Command palette, label map, goto/waypoints | `src/command_palette.rs`, `src/goto.rs` |
| Status-effect framework (generic over kind) | `src/status_effect.rs` |
| Item-identification framework (generic over appearance/effect) | `src/item.rs:324-448` |
| Message log, hit-flash, SVG icon registry | `src/ui.rs` (partial), `src/visuals/indicator.rs`, `src/sprite.rs` |
| Time-scale arbitration | `src/time_scale.rs` |
| Headless scripted runner + PNG snapshot harness | `src/bin/headless/runner.rs` |
| Run states, `LevelEntity`, `DungeonDepth`, `WorldBounds`, physics-layer scaffold | the `// TODO: move all these things out!` block in `src/lib.rs` |
| Verlet ropes constrained by terrain (optional plugin) | `src/effects/rope.rs` |

### `vector-arena` (demo)

Content only: item kinds and use-dispatch, `Inventory`, `Stats`, the
`MonsterState` AI, the player, level population, magic missiles, unstable
sigils, scroll effects, the concrete status-effect kinds (Confusion, Torpor, …),
the eight palette commands, the five sprites, and the game-specific HUD panels.

## API sketches

### Palette — generalise the two-level hack into an n-level tree

`PaletteCommandKind` bakes in `ItemKind` today and supports exactly two levels,
via a `requires_target` bool plus a `target_los_only` fallback the code itself
labels "legacy" (`src/command_palette.rs:481-496`). Replace with:

```rust
pub struct PaletteCommand {
    pub key: String,
    pub description: String,
    pub icon: Option<IconId>,          // opaque handle, not ItemKind
    pub root: EntryOutcome,
}

pub enum EntryOutcome {
    /// Engine calls this one-shot system for the next level of entries.
    Submenu(SystemId<In<PalettePath>, Vec<PaletteEntry>>),
    /// Engine runs its own target picker: entity letters, location letters,
    /// or a world click. Filter decides which entities are offerable.
    PickTarget { verb: String, filter: TargetFilter },
    /// Terminal: engine invokes the command's handler.
    Run,
}
```

Execution stops being a polled mailbox. Each command registers a
`SystemId<In<CommandInvocation>>` handler and the engine invokes exactly the
right one — no re-parsing, no re-queueing. (Today `src/item.rs:774` *puts the
command back* when it isn't an item command, so handlers cooperate by
convention.) `resolve_location_letter` (`src/command_palette.rs:231`) becomes
the engine's resolver, grammar unchanged.

The demo's `w` becomes
`Submenu(wands_in_inventory) → PickTarget { verb: "Wave at" } → Run`, where
`wands_in_inventory` is a game-side Bevy system. `q`/`r`/`e` are one-level
submenus; `g`/`z` are `PickTarget` at the root; `d`/`.` are `Run`.

### Labels — generic over the game's key type

`LetterMap` (`src/command_palette.rs:794`) splits into:

- `EntityLabels` — uppercase and digits, auto-assigned to entities carrying the
  engine's `Targetable` marker, auto-released on despawn via
  `RemovedComponents` (removing the manual `release_monster` call inside
  `apply_damage_on_hit`).
- `LabelPool<K: Hash + Eq>` — lowercase a–z, stable, generic over a game key.
  The demo instantiates `LabelPool<ItemKind>`; the engine never sees `ItemKind`.

Waypoint labels (`GotoState.labels`, with `hjkl`/`yubn`/`s` pinned) stay engine.

### Level generation — trait + builder, kill the 6× copy-paste

`src/dungeon/level_generation.rs:474-793` copy-pastes ~30 lines of
connection-carving in six of eight room arms; `random_room_variant` is a
`gen_range(0..6)` index match with an `unreachable!()`; and every new feature
added a `Vec` field to `TerrainGeometry` (now 9 fields, returned as an 8-tuple).

```rust
pub trait RoomKind: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn weight(&self, ctx: &LevelContext) -> f32;   // depth-aware; 0.0 = never
    fn carve(&self, req: &RoomRequest, rng: &mut dyn RngCore, out: &mut LevelBuilder);
}
```

`LevelBuilder` exposes `add_floor`, `add_glass`, `add_door`,
`add_cost_region(poly, cost)`, `add_marker(tag, pos)`, `add_region(tag, poly)`,
and crucially `carve_connection(conn)` — the helper that absorbs the
duplication. Selection is a weighted draw over a
`RoomRegistry(Vec<Arc<dyn RoomKind>>)` resource, so adding a room is
`app.add_room_kind(MyRoom)`, and retuning the generator is adjusting weights or
replacing the registry wholesale.

`TerrainGeometry`'s per-feature `Vec`s collapse into tagged output:

```rust
pub struct LevelPlan {
    pub solid_rock: SafeMultiPolygon,
    pub playable_area: SafeMultiPolygon,
    pub glass_walls: SafeMultiPolygon,
    pub rooms: Vec<Rect<f32>>,
    pub corridor_ends: Vec<Vec2>,
    pub doors: Vec<DoorGeometry>,
    pub markers: HashMap<&'static str, Vec<Vec2>>,        // "vault_center", …
    pub regions: HashMap<&'static str, Vec<SafePolygon>>, // "slow", "rubble", …
}
```

The game reads its own tags, so a new room feature needs no engine change.

**Stock rooms.** All eight variants ship in `rogue_angles::rooms::stock`, opt-in
via `RoomRegistry::stock()`. Two renames drop game vocabulary: `Torpor` →
`SlowZoneRoom` (tags a `"slow"` cost region, which the demo maps to its Torpor
status effect) and `Chamber` → `VaultRoom` (walled inner chamber with a door,
emitting a `"vault_center"` marker).

> **Enforcement rule:** stock rooms may use only the *public* `LevelBuilder`
> API — no `pub(crate)` shortcuts. Keeping them in the engine means they no
> longer pressure-test the trait by being ported out, so this rule plus the
> phase-8 example's custom room is what keeps the API honest.

### Population — an observer, not a hardcoded call

`spawn_game_world` currently ends by calling `populate_level::populate`
(`src/game.rs:165-334`). Instead the engine finishes terrain and triggers
`PopulateLevel { plan }`; the demo's observer spawns player, monsters, items,
sigils, and the staircase. The engine only needs the player to carry `Viewer`
and `NavAgent`.

### Status effects — engine framework, game vocabulary

```rust
pub trait StatusKind: Copy + Eq + Send + Sync + 'static {
    fn modifiers(self, strength: f32) -> Modifiers;   // engine-meaningful only
}
```

`Modifiers` carries what engine systems consume — movement speed, vision radius.
`app.add_status_effects::<K>()` registers the generic tick system, which writes
a `MovementModifiers` component that `nav.rs` reads. Game-only aggregates (the
missile-damage multiplier) stay game-side via `StatusEffects::<K>::strength_of`.
This also inverts today's `nav.rs → status_effect.rs` dependency.

Fix during the move: `src/status_effect.rs:77` has a standing TODO — effects
stack additively instead of refreshing an existing match.

### Item identification — engine framework, game types

`ItemIdentities` (`src/item.rs:324-448`) becomes
`IdentityTable<A: Hash + Eq + Copy, E: Copy>`: per-run shuffled
appearance→effect bijection, `identify`, and `forget` using the true derangement
from `derange_indices` (`src/item.rs:419`). Being generic, it does not constrain
the game's inventory model — the demo instantiates three
(`PotionColor→PotionEffect`, `ScrollName→ScrollEffect`, `WandGem→WandEffect`)
and keeps `Inventory` entirely game-side.

### Combat — engine owns nothing

The engine contributes queries and hooks only: `los_clear(a, b)` (already the
one-liner duplicated in four places), shape-cast helpers, `Occluder` /
`OpaqueVertices`, physics-layer registration, and despawn notification. HP,
damage, factions, death, knockback, and dodge are game-side. `Stats` (today in
`src/monster.rs:33`, used by the player too) moves to a game module.

### Time scale — votes, not a fixed policy

Today: 0.0× idle, 1.0× moving, 0.5× missile-in-flight, hardcoded at
`src/time_scale.rs:16`. Replace with a `TimeScaleVotes` resource that systems
push named requests into; the engine takes the minimum. The demo registers the
same three votes and behaves identically.

## Phasing

Each phase compiles and keeps `cargo test` green.

0. **Workspace scaffold.** Virtual root, empty `rogue-angles`, `vector-arena`
   moved under `crates/`, Trunk/CI/asset paths fixed. **Done.**
1. **Move the clean leaves.** `safegeo`, `bsp`, `terrain`, `nav`, `fov` plus
   auto-explore, `time_scale`, `indicator`. Also landed here (pulled forward
   because the "clean" leaves turned out not to be): the `MovementModifiers`
   inversion (nav/fov no longer read the game's `StatusEffects` directly) and
   the `TimeScaleVotes` generalization. Message log and the headless snapshot
   harness were deliberately deferred, not moved — see the phase-1 notes
   above. **Done.**
2. **Palette rework, merged with phase 3 (labels + goto).** `EntryOutcome`
   tree, `SystemId` handlers, `IconId`, `EntityLabels`, `LabelPool<K>`,
   `LocationLabels`/`LocationDescriptions`. See the phase-2 notes above for
   why the merge happened and the scripting-capability fix that came out of
   testing it. **Done.**
3. *(Absorbed into phase 2.)*
4. **Level generation.** `RoomKind` / `LevelBuilder` / `LevelPlan`; convert the
   eight variants in place into stock implementors; `PopulateLevel` observer.
5. **Frameworks.** Status effects generic over `K`; `IdentityTable<A, E>`.
   (`MovementModifiers` itself already landed in phase 1.)
6. **Presentation split.** `IconId` registry over the SVG pipeline; engine HUD
   chrome separated from the demo's stat and inventory panels; the
   `ui.rs ↔ command_palette.rs` cycle untangled; the headless snapshot/tick
   harness extraction deferred from phase 1 belongs here too.
7. **Draw the crate boundary.** Everything unmoved is game code; the
   `rogue-angles` path dependency has been live since phase 1 — this is the
   final sweep for any remaining leaks.
8. **Acceptance test: a second game.** A ~500-line example under
   `crates/rogue-angles/examples/` with deliberately different mechanics — melee
   combat instead of projectiles, no inventory at all, one custom `RoomKind`.
   Written *before* the split is declared done; it doubles as the tutorial.

## Verification

- `cargo test` at each phase — `integration_tests.rs`, `rope_tests.rs`,
  `fov_performance.rs`, `game_tests.rs`, and the palette unit tests
  (`src/command_palette.rs:852`) stay green. The palette tests need rewriting in
  phase 2; that is part of the phase.
- **Determinism check.** `DungeonSeedOverride` plus seed 42 makes levels
  reproducible. Before phase 4, snapshot the generated geometry for several
  seeds; after the `RoomKind` refactor the stock registry must reproduce them
  bit-for-bit. This is the strongest available regression test for the riskiest
  phase.
- `cargo run --bin headless -- 'wait 1s; snap /tmp/a.png'` before and after each
  phase, visually diffed. Phase 2 additionally scripts `cmd q`, `cmd 'w a A'`,
  and `cmd 'g s'` end-to-end.
- `cargo run` and play a level after phases 2, 4, and 6.
- Phase 8's example must build and run against `rogue-angles` alone.

## Docs

`DEVELOPING.md` (= `AGENTS.md`) is stale — it lists `scrolls.rs` at top level,
says `bevy_egui` is "not yet wired up", and points registration at `main.rs`
instead of `game.rs`. Split it into an engine guide (the extension points above,
with the phase-8 example as the worked tutorial) and a shorter demo-game guide;
correct it as part of phase 7.
