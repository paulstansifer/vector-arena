  * Ropes tend to pass through walls.
  * Magic missles sometimes launch from the wrong spot, especially when monsters fire them
  * Monsters sometimes try to wander somewhere inaccessible, and hold still
  * Move the top-level constants/resources/enums out of [lib.rs](src/lib.rs) (`AGENT_RADIUS`, `WorldBounds`, `GameState`, `Staircase`, `DungeonDepth`, `GameLayer`) into properly-named modules.
  * Investigate the `.expect()` in [dungeon/terrain.rs](src/dungeon/terrain.rs) that has failed before.
  * Investigate `validate()` failures in [nav.rs](src/nav.rs) when terrain is destroyed.
  * Make `ItemKind` data-driven instead of a hardcoded enum (see the TODO in [item.rs](src/item.rs)).
  * I've seen `buffer_with_style` in fov.rs crash (I was jammed against a door; no idea how to reproduce). Let's test when `buffer` crashes and see if we can prevent it.
  * Items should stay on their current letter (either Nethack-style or Angband-style)
  * "i" should look at all inventory (and give the option of using any items)
  * Use less than half a megabyte worth of font for the scrolls.
  * The allocated playable area overlaps the top and bottom bars.
  * The top bar should slide down when opened.
  * Place monsters and down staircases in random accessible locations, just like items.
  * The test setup is causing a lot of unusedness warnings visible in the editor, but not `cargo check`.
  

  # Larger projects
  * Think about how to break this into a library + game definition
  * Using `rough_vello` to create a hand-drawn look might be cute