// Monster marker component. Targeting and velocity are handled externally:
// `main.rs` attaches `AgentTarget2d::Entity(player)` and `nav.rs` converts the
// Landmass desired-velocity into an Avian2D `LinearVelocity` each frame.
use bevy::prelude::*;

pub const MONSTER_SPEED: f32 = 80.0;
pub const MONSTER_MAX_HP: f32 = 20.0;
// pub const MONSTER_STOP_DIST: f32 = 50.0;

#[derive(Component)]
pub struct Monster;

#[derive(Component, Clone, Copy)]
pub struct MonsterStats {
    pub hp: f32,
    pub max_hp: f32,
}

pub fn refresh_monster_tooltips(
    mut query: Query<(&MonsterStats, &mut crate::ui::WorldTooltip), With<Monster>>,
) {
    for (stats, mut tooltip) in query.iter_mut() {
        tooltip.0 = format!("HP: {}/{}", stats.hp as i32, stats.max_hp as i32);
    }
}

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
