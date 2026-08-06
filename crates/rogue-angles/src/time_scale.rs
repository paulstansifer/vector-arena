// Generic virtual-time arbitration. Any game system can register a reason for
// time to flow at some speed by inserting a named vote each frame; the engine
// takes the minimum across whatever votes are currently present and applies it
// to `Time<Virtual>`. A frame with no votes at all means nothing is asking for
// time to advance, so the default is 0.0 (paused) rather than 1.0 — a game
// that always wants time flowing can simply always cast a `1.0` vote of its
// own; the engine doesn't assume that's the common case.
use bevy::prelude::*;
use std::collections::HashMap;

#[derive(Resource, Default)]
pub struct TimeScaleVotes(HashMap<&'static str, f32>);

impl TimeScaleVotes {
    /// Cast (or overwrite) this frame's vote under `name`. Remove a vote a
    /// system no longer wants to contribute with `clear`.
    pub fn set(&mut self, name: &'static str, relative_speed: f32) {
        self.0.insert(name, relative_speed);
    }

    pub fn clear(&mut self, name: &'static str) { self.0.remove(name); }
}

pub fn arbitrate_time_scale(votes: Res<TimeScaleVotes>, mut time: ResMut<Time<Virtual>>) {
    let speed = votes.0.values().copied().fold(f32::INFINITY, f32::min);
    time.set_relative_speed(if speed.is_finite() { speed.max(0.0) } else { 0.0 });
}
