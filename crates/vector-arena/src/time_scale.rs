// This game's virtual-time-scale policy: bullet-time while any magic missile is
// in flight, full speed while the player is moving, paused otherwise. The
// generic vote arbitration (minimum across named votes -> Time<Virtual> relative
// speed) lives in `rogue_angles::time_scale`; this file only casts this game's
// votes and additionally speeds up the physics fixed-timestep while a missile is
// in flight, since magic missiles are fast enough that bounces need a higher
// substep rate than the time dilation alone would give them. That timestep
// tuning is specific to how fast this game's projectiles are, so it stays here
// rather than being a generic engine concept.
use std::time::Duration;

use bevy::{input::keyboard::Key, prelude::*};
use rogue_angles::time_scale::TimeScaleVotes;

use crate::{
    effects::projectile::MagicMissile,
    player::{MoveTarget, Player},
};

const TIME_SCALE_MISSILE: f32 = 0.5;

pub fn cast_time_scale_votes(
    mut votes: ResMut<TimeScaleVotes>,
    mut fixed_time: ResMut<Time<Fixed>>,
    missile_query: Query<&MagicMissile>,
    move_target: Single<&MoveTarget, With<Player>>,
    keyboard: Res<ButtonInput<Key>>,
) {
    let any_missile = missile_query.iter().next().is_some();
    let period_held = keyboard.pressed(Key::Character(".".into()));

    if any_missile {
        votes.set("missile_in_flight", TIME_SCALE_MISSILE);
        // Physics would normally slow down for the time dilation, but the magic missles are so fast
        // that we need it to speed up in order for the bounces to work right.
        fixed_time.set_timestep(Duration::from_secs_f64(1.0 / (256.0 / TIME_SCALE_MISSILE as f64)));
    } else {
        votes.clear("missile_in_flight");
        fixed_time.set_timestep(Duration::from_secs_f64(1.0 / 64.0));
    }

    if move_target.active || period_held {
        votes.set("moving", 1.0);
    } else {
        votes.clear("moving");
    }
}
