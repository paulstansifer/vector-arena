pub mod bsp;
pub mod fov;
pub mod monster;
pub mod nav;
pub mod player;
pub mod terrain;

pub const AGENT_RADIUS: f32 = 10.0;

#[derive(bevy::prelude::Resource)]
pub struct WorldBounds {
    pub width: f32,
    pub height: f32,
}

#[derive(bevy::prelude::Resource)]
pub struct WorldObstacles(pub geo::MultiPolygon<f32>);
