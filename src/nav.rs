// Bridge between Landmass pathfinding and Avian2D physics for monsters.
// Reads `AgentDesiredVelocity2d` computed by Landmass each frame and writes it
// to `LinearVelocity`, capped at MONSTER_SPEED. The player is excluded here
// because it uses custom steering instead.
use crate::player;
use avian2d::prelude::LinearVelocity;
use bevy::prelude::*;
use bevy_landmass::prelude::*;

const AGENT_ACCELERATION: f32 = 50.0;

/// Apply landmass's desired velocity as actual movement on agents.
pub fn apply_agent_velocity(
    mut agents: Query<(&AgentDesiredVelocity2d, &mut LinearVelocity), Without<player::Player>>,
) {
    for (desired_velocity, mut velocity) in agents.iter_mut() {
        let vel_diff: Vec2 = desired_velocity.velocity() - velocity.0;
        velocity.0 += vel_diff.clamp_length_max(AGENT_ACCELERATION);
    }
}
