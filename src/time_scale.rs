// Global virtual-time scaling. Slows the world to bullet-time while any magic missile
// is in flight, pauses entirely when the player has no current move target, and bumps
// the fixed-timestep rate to keep physics (and bouncing missiles in particular) stable
// under the time dilation.
use std::time::Duration;

use bevy::{input::keyboard::Key, prelude::*};

use crate::{
    effects::projectile::MagicMissile,
    player::{MoveTarget, Player},
};

const TIME_SCALE_MISSILE: f32 = 0.5;

pub fn manage_time_scale(
    mut time: ResMut<Time<Virtual>>,
    mut fixed_time: ResMut<Time<Fixed>>,
    missile_query: Query<&MagicMissile>,
    move_target: Single<&MoveTarget, With<Player>>,
    keyboard: Res<ButtonInput<Key>>,
) {
    let any_missile = missile_query.iter().next().is_some();
    let period_held = keyboard.pressed(Key::Character(".".into()));

    if any_missile {
        time.set_relative_speed(TIME_SCALE_MISSILE);
        // Physics would normally slow down for the time dilation, but the magic missles are so fast
        // that we need it to speed up in order for the bounces to work right.
        fixed_time.set_timestep(Duration::from_secs_f64(1.0 / (256.0 / TIME_SCALE_MISSILE as f64)));
    } else {
        fixed_time.set_timestep(Duration::from_secs_f64(1.0 / 64.0));
        if move_target.active || period_held {
            time.set_relative_speed(1.0);
        } else {
            time.set_relative_speed(0.0);
        }
    }
}
