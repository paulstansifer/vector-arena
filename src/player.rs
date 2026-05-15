use bevy::prelude::*;
use bevy_rapier2d::prelude::*;

pub const PLAYER_RADIUS: f32 = 15.0;
pub const PLAYER_SPEED: f32 = 480.0;
pub const STOP_THRESHOLD: f32 = 8.0;

#[derive(Component)]
pub struct Player;

#[derive(Component, Default)]
pub struct MoveTarget {
    pub destination: Vec2,
    pub origin: Vec2,
    pub active: bool,
}

fn lerp(start: f32, end: f32, t: f32) -> f32 {
    start + (end - start) * t
}

pub fn set_target_on_click(
    window: Single<&Window>,
    camera_query: Single<(&Camera, &GlobalTransform)>,
    mouse_button_input: Res<ButtonInput<MouseButton>>,
    mut time: ResMut<Time<Virtual>>,
    mut player_query: Query<(&Transform, &mut MoveTarget), With<Player>>,
) {
    if !mouse_button_input.just_pressed(MouseButton::Left) {
        return;
    }

    time.set_relative_speed(0.25);

    let cursor_position = match window.cursor_position() {
        Some(position) => position,
        None => return,
    };

    let (camera, camera_transform) = *camera_query;
    let world_position = match camera.viewport_to_world_2d(camera_transform, cursor_position) {
        Ok(world_pos) => world_pos,
        Err(_) => return,
    };

    for (transform, mut move_target) in player_query.iter_mut() {
        let current_position = transform.translation.truncate();
        let distance = current_position.distance(world_position);

        if distance <= STOP_THRESHOLD {
            move_target.active = false;
            continue;
        }

        move_target.destination = world_position;
        move_target.origin = transform.translation.truncate();
        move_target.active = true;
    }
}

pub fn move_player(
    mut time: ResMut<Time<Virtual>>,
    mut query: Query<(&mut Transform, &mut Velocity, &mut MoveTarget), With<Player>>,
) {
    for (mut transform, mut velocity, mut move_target) in query.iter_mut() {
        if !move_target.active {
            time.set_relative_speed(0.0);
            velocity.linear = Vec2::ZERO;
            continue;
        }

        let current = transform.translation.truncate();
        let direction = move_target.destination - current;
        let distance = direction.length();

        if distance <= STOP_THRESHOLD || distance == 0.0 {
            transform.translation = move_target.destination.extend(transform.translation.z);
            velocity.linear = Vec2::ZERO;
            move_target.active = false;
            continue;
        }

        // speed up after starting to move:
        let away_from_origin = (current.distance(move_target.origin) / 60.0).clamp(0.0, 1.0);
        let speed_multiplier = lerp(0.25, 1.0, away_from_origin);
        let new_speed = f32::max(speed_multiplier, time.relative_speed());
        // slow down approaching the target:
        let speed_multiplier = lerp(0.25, 1.0, (distance / 60.0).clamp(0.0, 1.0));
        let new_speed = f32::min(speed_multiplier, new_speed);
        time.set_relative_speed(new_speed);

        let desired_speed = PLAYER_SPEED * time.relative_speed();
        velocity.linear = direction.normalize_or_zero() * desired_speed;

        let step = desired_speed * time.delta_secs();
        if step >= distance {
            transform.translation = move_target.destination.extend(transform.translation.z);
            velocity.linear = Vec2::ZERO;
            move_target.active = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::{App, Transform, Vec2, Vec3};
    use bevy::time::{Time, Virtual};
    use std::time::Duration;

    fn set_frame_delta(time: &mut Time<Virtual>, delta: Duration) {
        time.advance_by(delta);
    }

    #[test]
    fn player_eventually_reaches_target() {
        let mut app = App::new();
        app.insert_resource(Time::<Virtual>::default());
        app.add_systems(Update, move_player);

        let destination = Vec2::new(160.0, 120.0);

        app.world_mut().spawn((
            Transform::from_translation(Vec3::ZERO),
            Velocity::zero(),
            MoveTarget {
                destination,
                origin: Vec2::ZERO,
                active: true,
            },
            Player,
        ));

        for _ in 0..600 {
            let mut time = app.world_mut().resource_mut::<Time<Virtual>>();
            set_frame_delta(&mut *time, Duration::from_secs_f32(1.0 / 60.0));
            app.update();

            let world = app.world_mut();
            let mut query = world.query::<(&Transform, &MoveTarget)>();
            let (_, move_target) = query.iter(&*world).next().unwrap();
            if !move_target.active {
                break;
            }
        }

        let world = app.world_mut();
        let mut query = world.query::<(&Transform, &MoveTarget)>();
        let (transform, move_target) = query.iter(&*world).next().unwrap();

        assert!(
            transform.translation.truncate().distance(destination) <= STOP_THRESHOLD,
            "player final position {:?} should be within {} units of destination {:?}",
            transform.translation.truncate(),
            STOP_THRESHOLD,
            destination,
        );
        assert!(
            !move_target.active,
            "player should stop after reaching the destination"
        );
    }
}
