use bevy::prelude::*;

use crate::player::Player;

pub const MONSTER_RADIUS: f32 = 10.0;
pub const MONSTER_SPEED: f32 = 80.0;
pub const MONSTER_STOP_DIST: f32 = 50.0;

#[derive(Component)]
pub struct Monster;

pub fn move_monsters(
    time: ResMut<Time<Virtual>>,
    player_query: Query<&Transform, (With<Player>, Without<Monster>)>,
    mut monster_query: Query<&mut Transform, With<Monster>>,
) {
    let player_transform = match player_query.single() {
        Ok(transform) => transform,
        Err(_) => return,
    };

    let player_position = player_transform.translation.truncate();
    let step = MONSTER_SPEED * time.delta_secs();

    for mut transform in monster_query.iter_mut() {
        let current = transform.translation.truncate();
        let delta = player_position - current;
        let distance = delta.length();

        if distance <= MONSTER_STOP_DIST {
            continue;
        }

        let direction = delta / distance;
        let target_distance = (distance - MONSTER_STOP_DIST).max(0.0);
        let move_amount = step.min(target_distance);
        transform.translation += (direction * move_amount).extend(0.0);
    }
}
