// Module exports, global constants, and shared types.
//
// The geometry/FOV/nav/terrain/palette substrate lives in the `rogue-angles`
// engine crate (see docs/ENGINE-SPLIT.md at the repo root); this crate is the
// game built on it. `AGENT_RADIUS`, `WorldBounds`, `LevelEntity`, and
// `GameLayer` moved to `rogue_angles` because engine code (nav, terrain, FOV,
// crumble_terrain) needs them. The globals below stay here because nothing
// engine-side currently needs them — they're this game's own run-state and
// depth-descent vocabulary.
pub mod command_palette;
pub mod effects;
pub mod fov;
pub mod game;
pub mod goto;
pub mod item;
pub mod monster;
pub mod player;
pub mod populate_level;
pub mod sprite;
pub mod status_effect;
pub mod time_scale;
pub mod ui;
pub mod visuals;

pub const WORLD_WIDTH: f32 = 1280.0;
pub const WORLD_HEIGHT: f32 = 720.0;

#[derive(bevy::prelude::States, Default, Clone, PartialEq, Eq, Hash, Debug)]
pub enum GameState {
    #[default]
    Restart,
    Descend,
    InLevel,
    GameOver,
}

#[derive(bevy::prelude::Component)]
pub struct Staircase;

#[derive(bevy::prelude::Component)]
pub struct StaircaseFogCopy;

#[derive(bevy::prelude::Resource)]
pub struct DungeonDepth(pub u32);

impl Default for DungeonDepth {
    fn default() -> Self { Self(1) }
}
