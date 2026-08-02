# Splitting Vector Arena into `rogue-angles` + a demo game

Status: **in progress.** Phase 0 done; phases 1–8 pending.

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
   auto-explore, `time_scale`, `indicator`, message log, headless harness. Break
   up the `lib.rs` globals block. Near-zero inbound coupling; mostly mechanical.
2. **Palette rework.** `EntryOutcome` tree, `SystemId` handlers, `IconId`. The
   biggest API-design step; do it while both halves are still one crate so the
   refactor is compiler-guided.
3. **Labels and goto** to the engine; `LabelPool<K>` generic.
4. **Level generation.** `RoomKind` / `LevelBuilder` / `LevelPlan`; convert the
   eight variants in place into stock implementors; `PopulateLevel` observer.
5. **Frameworks.** Status effects generic over `K`; `IdentityTable<A, E>`;
   `MovementModifiers` inversion.
6. **Presentation split.** `IconId` registry over the SVG pipeline; engine HUD
   chrome separated from the demo's stat and inventory panels; the
   `ui.rs ↔ command_palette.rs` cycle untangled.
7. **Draw the crate boundary.** Everything unmoved is game code; make
   `rogue-angles` a real dependency and let the compiler find the leaks.
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
