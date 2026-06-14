// Module exports, global constants, and shared types.
pub mod command_palette;
pub mod dungeon;
pub mod effects;
pub mod fov;
pub mod goto;
pub mod indicator;
pub mod item;
pub mod monster;
pub mod nav;
pub mod objects;
pub mod player;
pub mod populate_level;
pub mod sprite;
pub mod status_effect;
pub mod time_scale;
pub mod ui;

// TODO: move all these things out!
pub const AGENT_RADIUS: f32 = 10.0;

pub const WORLD_WIDTH: f32 = 1280.0;
pub const WORLD_HEIGHT: f32 = 720.0;

#[derive(bevy::prelude::Resource)]
pub struct WorldBounds {
    pub width: f32,
    pub height: f32,
}

#[derive(bevy::prelude::States, Default, Clone, PartialEq, Eq, Hash, Debug)]
pub enum GameState {
    #[default]
    Restart,
    Descend,
    InLevel,
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

use avian2d::prelude::PhysicsLayer;

#[derive(PhysicsLayer, Clone, Copy, Debug, Default)]
pub enum GameLayer {
    #[default]
    Wall, // static terrain
    Dynamic, // player, monsters, doors, rubble
    Missile, // magic missiles
    Rope,    // rope segments
}
