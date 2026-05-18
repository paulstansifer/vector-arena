use avian2d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::prelude::*;

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
    mut player_query: Query<(&Transform, &mut MoveTarget, &mut AgentTarget2d), With<Player>>,
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

    for (transform, mut move_target, mut agent_target) in player_query.iter_mut() {
        let current_position = transform.translation.truncate();
        let distance = current_position.distance(world_position);

        if distance <= STOP_THRESHOLD {
            move_target.active = false;
            *agent_target = AgentTarget2d::None;
            continue;
        }

        move_target.destination = world_position;
        move_target.origin = transform.translation.truncate();
        move_target.active = true;
        *agent_target = AgentTarget2d::Point(world_position);
    }
}

pub fn move_player(
    mut time: ResMut<Time<Virtual>>,
    mut query: Query<
        (
            &Transform,
            &mut LinearVelocity,
            &mut MoveTarget,
            Option<&AgentDesiredVelocity2d>,
            &mut AgentTarget2d,
        ),
        With<Player>,
    >,
) {
    for (transform, mut velocity, mut move_target, desired_velocity, mut agent_target) in
        query.iter_mut()
    {
        if !move_target.active {
            time.set_relative_speed(0.0);
            *velocity = LinearVelocity::ZERO;
            continue;
        }
        time.set_relative_speed(1.0);

        let current = transform.translation.truncate();
        let distance = (move_target.destination - current).length();

        if distance <= STOP_THRESHOLD {
            *velocity = LinearVelocity::ZERO;
            move_target.active = false;
            *agent_target = AgentTarget2d::None;
            continue;
        }

        let direction = if let Some(dv) = desired_velocity {
            dv.velocity()
        } else {
            move_target.destination - current
        };

        // speed up after starting to move:
        let away_from_origin = (current.distance(move_target.origin) / 60.0).clamp(0.0, 1.0);
        let adj_speed = lerp(0.25, 1.0, away_from_origin) * PLAYER_SPEED;
        let new_speed = f32::max(adj_speed, velocity.length());
        // slow down approaching the target:
        let adj_speed = lerp(0.25, 1.0, (distance / 60.0).clamp(0.0, 1.0)) * PLAYER_SPEED;
        let new_speed = f32::min(adj_speed, new_speed);

        velocity.0 = direction.normalize_or_zero() * new_speed;
    }
}

