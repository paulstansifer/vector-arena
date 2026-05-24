# Vector Arena

A 2D dungeon-exploration game built with [Bevy](https://bevyengine.org/) (0.18.1) and [Avian2D](https://github.com/Jondolf/avian) physics. BSP-generated dungeons, raycasted FOV, physics-based combat, and destructible terrain.

---

## Directory Structure

```
src/
├── main.rs                   # App setup, system registration, world startup (terrain, doors, FOV)
├── lib.rs                    # Module exports, GameLayer/GameState enums, WorldBounds, AGENT_RADIUS, etc.
├── player.rs                 # Player component, MoveTarget steering, click-to-move, exploration goals
├── monster.rs                # Monster, Stats, wander/seek AI, MonsterDrop, tooltip refresh
├── nav.rs                    # Landmass→Avian2D velocity bridge + navmesh triangulation
├── fov.rs                    # FOV raycasting, exploration tracking, mesh overlays
├── item.rs                   # Items, inventory, pickup animation, use dialog dispatch
├── populate_level.rs         # Spawns player, monsters, items, and the down staircase into rooms
├── time_scale.rs             # Global virtual-time scaling (bullet-time / pause / normal)
├── ui.rs                     # egui HUD: message log, stat bars, inventory, menu, item dialogs
├── dungeon/
│   ├── mod.rs                # Re-exports
│   ├── bsp.rs                # Binary Space Partitioning algorithm
│   ├── level_generation.rs   # Dungeon geometry from BSP (rooms, corridors, doors)
│   └── terrain.rs            # Geometry → Bevy mesh / Avian collider / Landmass navmesh
└── effects/
    ├── mod.rs                # Re-exports
    ├── projectile.rs         # Magic missiles, trails, knockback
    ├── rope.rs               # Drag-to-draw rope with segment physics
    └── crumble_terrain.rs    # Right-click excavation and rubble spawning
```

---

## Key Dependencies

| Crate                  | Purpose                                                  |
| ---------------------- | -------------------------------------------------------- |
| `bevy` 0.18.1          | Game engine (ECS, rendering, input, windowing)           |
| `avian2d` 0.5          | 2D rigid-body physics                                    |
| `bevy_landmass` 0.11.1 | Navigation mesh pathfinding for monsters                 |
| `geo` 0.32             | Computational geometry — polygon booleans, triangulation |
| `rand` 0.8             | RNG for dungeon generation                               |
| `bevy_egui` 0.39.1     | UI (imported, not yet wired up)                          |

---

## Architecture Overview

### Startup sequence (`main.rs::spawn_game_world`)

1. Generate dungeon: `BSP → level_generation → TerrainGeometry`
2. Create physics/render/navmesh resources from terrain geometry
3. Spawn terrain entity, doors (with revolute-joint hinges), FOV overlay meshes, navmesh island
4. Hand off to `populate_level::populate` to spawn the player, monsters, items, and the down staircase
5. `setup` (separate Startup system) inserts the camera and clear color

### Per-frame update loop

```
Input
  left-click  → MoveTarget on player, AgentTarget2d for pathfinding
  right-click → excavate terrain circle, spawn rubble
  M           → fire player magic missile
  R           → draw rope

Movement
  player   → custom steering from MoveTarget (lerped acceleration)
  monsters → Landmass desired velocity → Avian2D LinearVelocity (nav.rs)

FOV        → fov.rs raycasts from player, updates ExplorationState mesh

Missiles   → advance, spawn trails, apply manual knockback to Dynamic bodies
Items      → proximity check, animate pickup into inventory

Physics    → Avian2D step

Time scale → 0.0× idle | 1.0× moving | 0.05× (bullet-time) while missiles exist
```

---

## Time Scale Summary

| Situation               | Virtual time scale  |
| ----------------------- | ------------------- |
| Player idle (no target) | 0.0× (paused)       |
| Player moving           | 1.0×                |
| Any missile in flight   | 0.05× (bullet-time) |

Item pickup animations and the physics fixed-timestep are both adjusted to remain smooth regardless of scale.

---

## Adding New Things

**New item type** — add a variant to `ItemKind` in [src/item.rs](src/item.rs), then spawn it in [src/populate_level.rs](src/populate_level.rs) with a mesh/material.

**New effect** — add a file under `src/effects/`, register its plugin/systems in `main.rs`, add it to the `effects/mod.rs` re-exports.

**New monster behavior** — extend [src/monster.rs](src/monster.rs) with components and systems; register the systems in `main.rs`.

**Terrain changes at runtime** — mutate `DungeonState` (the `solid_rock`/`playable_area` `MultiPolygon`s); `sync_dungeon_to_entities()` will automatically rebuild mesh, collider, and navmesh on the next frame. See `crumble_terrain.rs` for the pattern.
