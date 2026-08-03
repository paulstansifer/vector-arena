# Vector Arena

A traditional dungeon-crawl roguelike ... except that the world is 2D vector objects with physics instead of a grid. It is built with [Bevy](https://bevyengine.org/) (0.18.1) and [Avian2D](https://github.com/Jondolf/avian) physics. BSP-generated dungeons, raycasted FOV, physics-based combat, and destructible terrain.

The project is split into a reusable engine and the game itself, so this file
covers both. If you only care about one side, jump to
[The engine](#the-engine-rogue-angles) or
[The demo game](#the-demo-game-vector-arena).

## Workspace

This repo is a Cargo workspace:

| Crate | Path | Role |
| ----- | ---- | ---- |
| `vector-arena` | `crates/vector-arena` | The game — bins `game` and `headless`. Bare `cargo run`/`cargo test` target this. |
| `gauntlet` | `crates/gauntlet` | A second, deliberately different game on the same engine (lunge-attacking monsters, a terrain-crumbling/knockback shot, no inventory) — the phase-8 acceptance test that the engine boundary is real. `cargo run -p gauntlet` / `cargo test -p gauntlet`. |
| `rogue-angles` | `crates/rogue-angles` | The engine: geometry, level generation, FOV, navmesh steering, the command palette, and generic status-effect/identification/HUD frameworks. |
| `bevy_verlet`  | `crates/bevy_verlet`  | Vendored third-party fork, excluded from the workspace. |

The engine/game split's design rationale and phase-by-phase history are in
[docs/ENGINE-SPLIT.md](docs/ENGINE-SPLIT.md) — read that first if you're
deciding whether something new belongs in the engine or the game. Build
profiles live in the root `Cargo.toml`, since Cargo ignores `[profile.*]` in
non-root members.

---

## The engine (`rogue-angles`)

`rogue-angles` is for *2D geometric free-movement roguelikes*: polygon
terrain instead of a grid, BSP level generation, raycast FOV, navmesh
steering, destructible terrain, and a keystroke command palette. It has no
idea what a potion, a monster stat, or a magic missile is — those are the
game's vocabulary. Two things are deliberately baked in rather than
pluggable: the command palette's letter-and-number abbreviation grammar, and
its keystroke-completion interaction model.

Paths below are relative to `crates/rogue-angles/src/`.

```
  *  lib.rs                # AGENT_RADIUS, WorldBounds, LevelEntity, GameLayer
  *  fov.rs                # FOV raycasting, exploration tracking, mesh overlays
  *  nav.rs                # Landmass→Avian2D velocity bridge + navmesh triangulation
  *  movement.rs           # Viewer marker, MovementModifiers (narrow engine-owned data)
  *  palette.rs            # Command palette: EntryOutcome tree, labels, IconId
  *  sprite.rs             # SVG-to-texture pipeline (bevy_svg/resvg); games register their own art
  *  hud.rs                # MessageLog, world tooltips, a labeled-bar primitive
  *  status_effects.rs     # StatusEffects<K>/StatusKind — generic timed-effect framework
  *  identity.rs           # IdentityTable<A, E> — per-run shuffled appearance→effect bijection
  *  time_scale.rs         # TimeScaleVotes — engine takes the minimum of named votes
  *  dungeon/
     *  bsp.rs              # Binary Space Partitioning algorithm
     *  level_generation.rs # Framework: role allocation, corridor carving, RoomKind/LevelBuilder/LevelPlan
     *  rooms/              # Stock RoomKind implementors (normal, oval, octagon, vault, rubble, ring, ...)
     *  terrain.rs          # Geometry → Bevy mesh / Avian collider / Landmass navmesh
  *  effects/
     *  crumble_terrain.rs  # Rock -> rubble in a region
  *  visuals/
     *  indicator.rs        # Hit-flash (non-egui half; egui-drawn state indicator stays game-side)
  *  util/
     *  safegeo.rs          # Sanitised geometry wrappers (SafePolygon, SafeMultiPolygon)
```

### How a game extends the engine

Four mechanisms cover every extension point; see `docs/ENGINE-SPLIT.md` for
the full rationale, and `crates/vector-arena` for a worked example of each:

1. **`SystemId` callbacks** — the engine holds a one-shot system the game
   registered, for anything needing arbitrary world access (palette submenus
   and command handlers: `palette::PaletteCommand`).
2. **Object-safe traits** — for pure-data work with no world access
   (`dungeon::level_generation::RoomKind`).
3. **Generic systems over a game-defined kind** — for frameworks with game
   vocabulary (`status_effects::StatusKind`, `identity::IdentityTable<A, E>`).
4. **Marker components + narrow engine-owned data** — the engine only reads
   what its own systems need (`movement::Viewer`, `movement::MovementModifiers`,
   `palette::Targetable`).

---

## The demo game (`vector-arena`)

Paths below are relative to `crates/vector-arena/src/`.

```
  *  main.rs                  # Binary entry point: window setup, camera, clear color
  *  lib.rs                   # Module exports, GameState enum, WORLD_WIDTH/HEIGHT, etc.
  *  game.rs                  # GamePlugin: all gameplay systems, startup logic, dungeon seeding
  *  player.rs                # Player component, MoveTarget steering, click-to-move, exploration goals
  *  monster.rs               # Monster, Stats, wander/seek AI, MonsterDrop, tooltip refresh
  *  item.rs                  # ItemKind, Inventory, ItemIdentities, pickup, use-command dispatch
  *  populate_level.rs        # Spawns level contents (player, monsters, items, staircase)
  *  time_scale.rs            # Casts this game's votes (idle/moving/missile-in-flight) into the engine's TimeScaleVotes
  *  ui.rs                    # egui HUD: stat bars, inventory icons, menu, game-over screen
  *  command_palette.rs       # Thin egui rendering shell over the engine's palette module
  *  sprite.rs                # This game's sprite vocabulary (ItemKind → SVG asset/param)
  *  status_effect.rs         # StatusEffect enum + StatusKind impl (confusion, blindness, ...)
  *  goto.rs                  # The "go to" command: location-label assignment policy
  *  bin/
     *  headless/             # Scripted headless runner (native only; see Testing section below)
        *  main.rs            # Entry point; compiles to an empty program on wasm32
        *  runner.rs          # The actual runner implementation
  *  effects/                 # Specific independent game systems
     *  projectile.rs         # Magic missiles, trails, knockback
     *  rope.rs               # Rope physics
     *  scroll.rs             # Scroll effects (teleport, summon monster, magic mapping, ...)
     *  unstable_sigils.rs    # Okay, fine, these are just explosive barrels
     *  hit_particles.rs      # Hit-impact particle burst
  *  visuals/                 # Visual-only systems
     *  indicator.rs          # egui-drawn state indicator (pairs with the engine's hit-flash)
     *  torpor_particles.rs   # Ambient particles indicating this game's slow zones
```

## Key Dependencies

| Crate                  | Purpose                                                  | Lives in |
| ---------------------- | -------------------------------------------------------- | -------- |
| `bevy` 0.18.1          | Game engine (ECS, rendering, input, windowing)           | both     |
| `avian2d` 0.5          | 2D rigid-body physics                                    | both     |
| `bevy_landmass` 0.11.1 | Navigation mesh pathfinding for monsters                 | both     |
| `geo` 0.32             | Computational geometry — polygon booleans, triangulation | both     |
| `rand` 0.8             | RNG for dungeon generation                               | both     |
| `bevy_egui` 0.39.1     | Command palette / HUD rendering                          | both     |
| `bevy_svg`, `resvg`    | SVG-to-mesh/texture sprite pipeline                      | `rogue-angles` |
| `pyri_tooltip`, `bevy_enoki` | egui tooltips, GPU particle effects                | `vector-arena` |

## Architecture Overview

### Startup sequence (`game.rs::spawn_game_world`)

1. Generate dungeon: `RoomRegistry::stock() → LevelPlan::new_seeded` (engine)
2. Create physics/render/navmesh resources from the level plan
3. Spawn terrain entity, doors (with revolute-joint hinges), FOV overlay meshes, navmesh island
4. Hand off to `populate_level::populate` to spawn the player, monsters, items, and the down staircase
5. `setup` (separate Startup system, in `main.rs`) inserts the camera and clear color

### Per-frame update loop

```
Input
  left-click  → MoveTarget on player, AgentTarget2d for pathfinding
  right-click → excavate terrain circle, spawn rubble
  z           → fire player magic missile (command palette)
  command palette → quaff/read/wave items, go to a point, descend, ...

Movement
  player   → custom steering from MoveTarget (lerped acceleration)
  monsters → Landmass desired velocity → Avian2D LinearVelocity (rogue_angles::nav)

FOV        → rogue_angles::fov raycasts from player, updates ExplorationState mesh

Missiles   → advance, spawn trails, apply manual knockback to Dynamic bodies
Items      → proximity check, animate pickup into inventory

Physics    → Avian2D step
```

## Time scale

The engine's `TimeScaleVotes` resource takes the minimum of whatever named
votes are currently cast; this game casts:

| Situation               | Virtual time scale |
| ----------------------- | ------------------ |
| Player idle (no target) | 0.0× (paused)      |
| Player moving           | 1.0×               |
| Any missile in flight   | 0.5× (bullet-time) |

Item pickup animations and the physics fixed-timestep are both adjusted to remain smooth regardless of scale.

## Adding New Things

**New item type** — add a variant to `ItemKind` in [src/item.rs](crates/vector-arena/src/item.rs), then spawn it in [src/populate_level.rs](crates/vector-arena/src/populate_level.rs) with a mesh/material.

**New effect** — add a file under `src/effects/`, add it to `effects/mod.rs`, and wire its systems/observers into `GamePlugin::build` in [src/game.rs](crates/vector-arena/src/game.rs) (most effects register directly there; a few purely-visual ones like `hit_particles`/`torpor_particles` are separate `Plugin`s added in `main.rs` instead, since they're optional in headless mode).

**New monster behavior** — extend [src/monster.rs](crates/vector-arena/src/monster.rs) with components and systems; register the systems in `GamePlugin::build`.

**New room type** — implement `rogue_angles::dungeon::level_generation::RoomKind` and register it on a `RoomRegistry` (see `dungeon::rooms::stock()` in the engine for eight worked examples).

**Terrain changes at runtime** — mutate `DungeonState` (the `solid_rock`/`playable_area` `MultiPolygon`s, from `rogue_angles::dungeon::terrain`); `sync_dungeon_to_entities()` will automatically rebuild mesh, collider, and navmesh on the next frame. See `rogue_angles::effects::crumble_terrain` for the pattern.

---

## Testing

### System dependencies (Linux)

Bevy needs a few system libraries, and `tests/game_tests.rs` builds a real
render app, so it needs a GPU adapter. On a headless box (CI, containers,
Claude Code web sessions) install Mesa's `llvmpipe` software rasterizer or the
test fails with `Unable to find a GPU!`:

```
apt-get update
apt-get install -y libwayland-dev libxkbcommon-dev libasound2-dev libudev-dev \
    libx11-dev libxrandr-dev libxi-dev libxcursor-dev libxinerama-dev \
    libgl1-mesa-dev mesa-vulkan-drivers libvulkan1
```

Verify with `vulkaninfo --summary` — a `deviceName` of `llvmpipe` is enough for
both `cargo test` and the headless snapshot runner. ALSA prints
`Unknown PCM default` warnings on such machines; they are harmless.

### Integration tests

```
cargo test --workspace          # run all tests, both crates
cargo test start_complete_game  # full-game smoke test only
```

Integration tests live in `crates/vector-arena/tests/`. Shared helpers live under `tests/test_lib/` (a subdirectory, not a top-level `.rs` file, so Cargo does not compile them as standalone test binaries). Each test file imports only the helper file it needs via `#[path]`, so unused helpers from other files don't produce dead-code warnings:

| Helper file           | Imported by                       | Provides                             |
| --------------------- | --------------------------------- | ------------------------------------ |
| `test_lib/physics.rs` | `integration_tests`, `rope_tests` | `physics_app`, `tick`, `loc`         |
| `test_lib/game.rs`    | `game_tests`                      | `headless_game_app`, `tick`          |
| `test_lib/preview.rs` | `fov_performance`                 | `PreviewWindow`, `Frame`, `Layer`, … |

- **`physics_app(gravity, ropes)`** — minimal app with Avian2D physics, no window.
- **`headless_game_app(seed)`** — full game app with no window/Winit; drives the game via `app.update()` directly. Used by the startup smoke test in `tests/game_tests.rs`.
- **`tick(app)`** — advance one 60 Hz frame (advances `Time<Virtual>` then calls `app.update()`).
- **`loc(app, entity)`** — read an entity's world-space `Transform` translation.

`GamePlugin { headless: true }` is used in test contexts to skip egui-dependent plugins while still initialising all gameplay systems.

`rogue-angles` also has its own unit tests (`cargo test -p rogue-angles`), covering geometry, BSP partitioning, the stock `RoomKind`s, palette resolution, and the generic status-effect/identity frameworks — these don't need a GPU or the system libraries above.

### Headless scripted runner

For interactive manual testing of a change — for example to take a screenshot or script a sequence of player actions — use the `headless` binary instead of the test harness:

```
cargo run --bin headless -- 'wait 1s; snap /tmp/before.png'
cargo run --bin headless -- 'wait 0.5s; click left 200 100; wait 2s; snap /tmp/after.png'
```

Progress and errors are written to `/tmp/va-headless.log` (Bevy's INFO logs are suppressed).

**Supported commands** (semicolon-separated in a single argument):

| Command              | Effect                                                          |
| -------------------- | ---------------------------------------------------------------- |
| `wait <N>s`          | Advance N seconds at 60 fps                                      |
| `snap <path>`        | Save a PNG screenshot to `<path>`                                |
| `cmd <path>`         | Drive the command palette directly (e.g. `cmd g h` to go to point `h`) |
| `click left <x> <y>` | Set the player's move target to world coordinates (x, y)         |
| `level blank`        | Replace the dungeon with an open 800×500 room                    |

`cmd` calls `rogue_angles::palette::execute_path_string` directly — the same
string form the interactive palette produces from keystrokes — so it can
drive any registered command, not just top-level ones.

The binary always starts with seed 42 (deterministic dungeon) and waits 120 frames for the game to reach `InLevel` before executing commands.
