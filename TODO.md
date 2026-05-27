  * Ropes to solid rock don't work.
  * Game tooltips are constrained to be very narrow.
  * I think magic missiles sometimes pass through monsters.
  * Magic missles sometimes launch from the wrong spot, especially when monsters fire them
  * Monsters sometimes try to wander somewhere inaccessible, and hold still
  * Move the top-level constants/resources/enums out of [lib.rs](src/lib.rs) (`AGENT_RADIUS`, `WorldBounds`, `GameState`, `Staircase`, `DungeonDepth`, `GameLayer`) into properly-named modules.
  * Break `apply_missile_knockback` (~135 lines, [projectile.rs](src/effects/projectile.rs)) into helpers for knockback, hit-flash, and drop-spawning.
  * Investigate the `.expect()` in [dungeon/terrain.rs](src/dungeon/terrain.rs) that has reportedly failed before.
  * Investigate `validate()` failures noted in [nav.rs](src/nav.rs) when terrain is destroyed.
  * Make `ItemKind` data-driven instead of a hardcoded enum (see the TODO in [item.rs](src/item.rs)).
  * I've seen `buffer_with_style` in fov.rs crash (I was jammed against a door; no idea how to reproduce). Let's test when `buffer` crashes and see if we can prevent it.
  
  # Larger projects
  * Think about how to break this into a library + game definition