  * Ropes tend to pass through walls.
  * Magic missles sometimes launch from the wrong spot, especially when monsters fire them
  * Move the top-level constants/resources/enums out of [lib.rs](src/lib.rs) (`AGENT_RADIUS`, `WorldBounds`, `GameState`, `Staircase`, `DungeonDepth`, `GameLayer`) into properly-named modules.
  * Investigate the `.expect()` in [dungeon/terrain.rs](src/dungeon/terrain.rs) that has failed before.
  * Investigate `validate()` failures in [nav.rs](src/nav.rs) when terrain is destroyed.
  * Make `ItemKind` data-driven instead of a hardcoded enum (see the TODO in [item.rs](src/item.rs)).
  * I've seen `buffer_with_style` in fov.rs crash (I was jammed against a door; no idea how to reproduce). Let's test when `buffer` crashes and see if we can prevent it.
  * "i" should look at all inventory (and give the option of using any items)
  * Use less than half a megabyte worth of font for the scrolls.
  * The top bar should slide down when opened.
  * The test setup is causing a lot of unusedness warnings visible in the editor, but not `cargo check`.
  * "g h" is treated the same as pressing "h", instead of going to that spot.
  * Hunger clock: a boredem meter fills up as time goes by. Fighting, explosions and (especially) trying unidentified items refills it.
  * Add a "vortex" dungeon feature (and a scroll that creates one).
  * Add more rooms:
    * Octagonal rooms
    * Rooms with a walled off portion in the middle, always containing a monster and an item.
  * Monsters should attack more
  * Different kinds of monsters!
  * Maybe the default monster attack shouldn't look like magic missile. Melee?
  * Confusion should have a chance of causing missiles to fire in the wrong direction.

  * Stauses should be carried over between levels.
  * Fix the failing integration test

  # Larger projects
  * Think about how to break this into a library + game definition
  * Using `rough_vello` to create a hand-drawn look might be cute