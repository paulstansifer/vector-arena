// Module exports and global run-state. This is `rogue-angles`'s second consumer: a
// deliberately different game from `vector-arena` (no inventory, no ranged monster
// attacks, no command palette) meant to pressure-test that the engine boundary is real —
// see docs/ENGINE-SPLIT.md at the repo root for what each module buys this game and what
// it deliberately doesn't touch.
pub mod dig_shot;
pub mod game;
pub mod monster;
pub mod player;
pub mod populate_level;
pub mod status_effect;
pub mod ui;

pub const WORLD_WIDTH: f32 = 1280.0;
pub const WORLD_HEIGHT: f32 = 720.0;
/// Reach this depth's amulet and the game is won.
pub const FINAL_DEPTH: u32 = 5;

#[derive(bevy::prelude::States, Default, Clone, PartialEq, Eq, Hash, Debug)]
pub enum GameState {
    #[default]
    Restart,
    Descend,
    InLevel,
    GameOver,
    Won,
}

/// The down staircase on depths 1..FINAL_DEPTH. Touching it descends.
#[derive(bevy::prelude::Component)]
pub struct Staircase;

/// Replaces the staircase on `FINAL_DEPTH`. Touching it wins the game.
#[derive(bevy::prelude::Component)]
pub struct AmuletOfWinning;

#[derive(bevy::prelude::Resource)]
pub struct DungeonDepth(pub u32);

impl Default for DungeonDepth {
    fn default() -> Self { Self(1) }
}
