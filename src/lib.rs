pub mod bsp;
pub mod fov;
pub mod monster;
pub mod nav;
pub mod player;
pub mod projectile;
pub mod terrain;

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
    Wall,    // static terrain
    Dynamic, // player, monsters, doors, rubble
    Missile, // magic missiles
}
