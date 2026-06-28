  * Ropes tend to pass through walls.
  * Magic missles sometimes launch from the wrong spot, especially when monsters fire them
  * Move the top-level constants/resources/enums out of [lib.rs](src/lib.rs) (`AGENT_RADIUS`, `WorldBounds`, `GameState`, `Staircase`, `DungeonDepth`, `GameLayer`) into properly-named modules.
  * Make `ItemKind` data-driven instead of a hardcoded enum (see the TODO in [item.rs](src/item.rs)).
  * "i" should look at all inventory (and give the option of using any items)
  * Use less than half a megabyte worth of font for the scrolls.
  * The top bar should slide down when opened.
  * The test setup is causing a lot of unusedness warnings visible in the editor, but not `cargo check`.
  * "g h" is treated the same as pressing "h", instead of going to that spot.
  * Add a "vortex" dungeon feature (and a scroll that creates one).
    * Tried this. Meh? Stashed it.
  * Add more rooms:
    * Octagonal rooms
    * Rooms with a walled off (with a door) portion in the middle, always containing a monster and an item.
    * Maze of corridors, with an item at a dead end
  * Monsters should attack more
  * Different kinds of monsters!
  * Maybe the default monster attack shouldn't look like magic missile. Melee?
  * Try adding doors back.
  * Add a Scroll Of Binding (creates a rope holding a monster to a random point of terrain).
    * Maybe consider effects that apply to an LOS monster, falling back to you if there isn't one? That would incentivize trying things out near monsters...

  # Larger projects
  * Think about how to break this into a library + game definition
  * Using `rough_vello` to create a hand-drawn look might be cute