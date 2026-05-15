use avian2d::prelude::*;
use bevy::prelude::*;

use crate::player::Player;

pub const MONSTER_RADIUS: f32 = 10.0;
pub const MONSTER_SPEED: f32 = 80.0;
pub const MONSTER_STOP_DIST: f32 = 50.0;

#[derive(Component)]
pub struct Monster;

// pub fn move_monsters(
//     player_query: Query<&Transform, (With<Player>, Without<Monster>)>,
//     mut monster_query: Query<(&Transform, &mut LinearVelocity), With<Monster>>,
// ) {
//     let player_transform = player_query.single().unwrap();

//     let player_position = player_transform.translation.truncate();

//     for (transform, mut velocity) in monster_query.iter_mut() {
//         let current = transform.translation.truncate();
//         let delta = player_position - current;
//         let distance = delta.length();

//         if distance <= MONSTER_STOP_DIST {
//             *velocity = LinearVelocity::ZERO;
//             continue;
//         }

//         let direction = delta.normalize_or_zero();
//         velocity.0 = direction * MONSTER_SPEED;
//     }
// }
