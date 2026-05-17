use crate::player;
use avian2d::prelude::LinearVelocity;
use bevy::prelude::*;
use bevy_landmass::prelude::*;

/// Apply landmass's desired velocity as actual movement on agents.
pub fn apply_agent_velocity(
    mut agents: Query<(&AgentDesiredVelocity2d, &mut LinearVelocity), Without<player::Player>>,
) {
    for (desired_velocity, mut velocity) in agents.iter_mut() {
        velocity.0 = desired_velocity.velocity();
    }
}
