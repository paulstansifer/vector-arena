// Module exports, global constants, and shared types.
pub mod dungeon;
pub mod effects;
pub mod fov;
pub mod item;
pub mod monster;
pub mod nav;
pub mod player;
pub mod ui;

// TODO: move all these things out!
pub const AGENT_RADIUS: f32 = 10.0;

#[derive(bevy::prelude::Resource)]
pub struct WorldBounds {
    pub width: f32,
    pub height: f32,
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
