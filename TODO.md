  * Ropes tend to pass through walls.
  * Game tooltips are constrained to be very narrow. Also, they display regardless of whether the thing they're attached to is visible.
  * Navigation to unseen areas still doesn't always work, especially when doorways are involved.
  * I think magic missiles sometimes pass through monsters.
  * Magic missles sometimes launch from the wrong spot, especially when monsters fire them
  * Monsters sometimes try to wander somewhere inaccessible, and hold still
  * Move the top-level constants/resources/enums out of [lib.rs](src/lib.rs) (`AGENT_RADIUS`, `WorldBounds`, `GameState`, `Staircase`, `DungeonDepth`, `GameLayer`) into properly-named modules.
  * Break `apply_missile_knockback` (~135 lines, [projectile.rs](src/effects/projectile.rs)) into helpers for knockback, and hit-flash. Drop-spawning should live in [monster.rs](src/monster.rs).
  * Investigate the `.expect()` in [dungeon/terrain.rs](src/dungeon/terrain.rs) that has failed before.
  * Investigate `validate()` failures in [nav.rs](src/nav.rs) when terrain is destroyed.
  * Make `ItemKind` data-driven instead of a hardcoded enum (see the TODO in [item.rs](src/item.rs)).
  * I've seen `buffer_with_style` in fov.rs crash (I was jammed against a door; no idea how to reproduce). Let's test when `buffer` crashes and see if we can prevent it.
  * Items should stay on their current letter (either Nethack-style or Angband-style)
  * "i" should look at all inventory (and give the option of using any items)
  * "d" should descend
  * "m" should allow targeting named locations (monsters, cardinal directions, corridors ends in the current room...)

  # Larger projects
  * Think about how to break this into a library + game definition
  * Using `rough_vello` to create a hand-drawn look might be cute