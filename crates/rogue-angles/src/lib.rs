//! `rogue-angles` — an engine for 2D geometric free-movement roguelikes.
//!
//! The world is polygons and physics rather than a grid: BSP-generated dungeon
//! geometry, raycast field-of-view, navmesh steering, and destructible terrain.
//! On top of that sits the interaction model this engine is opinionated about —
//! a keystroke command palette addressed by letter-and-number abbreviations
//! (that piece arrives in a later phase; see `docs/ENGINE-SPLIT.md` at the
//! repository root for the full plan).
pub mod dungeon;
pub mod effects;
pub mod fov;
pub mod movement;
pub mod nav;
pub mod time_scale;
pub mod util;
pub mod visuals;

/// Collision radius shared by every agent (player, monsters). Used to erode
/// playable area for spawning and navmesh generation.
pub const AGENT_RADIUS: f32 = 10.0;

#[derive(bevy::prelude::Resource)]
pub struct WorldBounds {
    pub width: f32,
    pub height: f32,
}

/// Marks entities that belong to the current level and should be cleaned up
/// when the game restarts it or moves on (e.g. descending a staircase).
#[derive(bevy::prelude::Component)]
pub struct LevelEntity;

use avian2d::prelude::PhysicsLayer;

#[derive(PhysicsLayer, Clone, Copy, Debug, Default)]
pub enum GameLayer {
    #[default]
    Wall, // static terrain
    Dynamic, // player, monsters, doors, rubble
    Missile, // magic missiles
    Rope,    // rope segments
}
