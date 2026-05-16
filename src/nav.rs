use bevy::prelude::*;
use bevy_landmass::prelude::*;
use crate::player;

/// Apply landmass's desired velocity as actual movement on agents.
pub fn apply_agent_velocity(
    mut agents: Query<(
        &AgentDesiredVelocity2d,
        &mut Transform,
    ), Without<player::Player>>,
    time: Res<Time>,
) {
    for (desired_velocity, mut transform) in agents.iter_mut() {
        let vel = desired_velocity.velocity();
        transform.translation += vel.extend(0.0) * time.delta_secs();
    }
}
