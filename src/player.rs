use std::time::Duration;

// Player component, click-to-move targeting, and steering.
// `MoveTarget` drives both the custom lerped-velocity steering and the Landmass
// `AgentTarget2d` (so the navmesh path is kept current even though the player
// uses its own steering rather than the landmass-computed velocity).
use avian2d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::prelude::*;
use geo::{BooleanOps, Contains};

use bevy::input::keyboard::Key;

use crate::{
    GameState, Staircase,
    command_palette::{CommandPaletteState, PaletteCommand, PaletteCommandKind, PaletteRegistry},
    dungeon::terrain::DungeonState,
    fov::{ExplorationState, find_exploration_waypoint},
    ui::MessageLog,
};

// How far (in world units) to search for the nearest navmesh point when a
// destination lands off the mesh (e.g. too close to a wall).
const NAVMESH_SNAP_RADIUS: f32 = 80.0;

fn snap_to_navmesh(point: Vec2, archipelago_query: &Query<&Archipelago2d>) -> Vec2 {
    archipelago_query
        .iter()
        .next()
        .and_then(|a| a.sample_point(point, &NAVMESH_SNAP_RADIUS).ok())
        .map(|s| s.point())
        .unwrap_or(point)
}

pub const PLAYER_SPEED: f32 = 480.0;
pub const STOP_THRESHOLD: f32 = 8.0;
// When holding down the directional keys.
// Max speed feels too fast! But maybe we should implement acceleration.
pub const PLAYER_DIRECTIONAL_SPEED: f32 = 240.0;

#[derive(Component)]
pub struct Player;

/// When present on the player, indicates an ongoing auto-explore toward a point
/// that was in the never-explored area when clicked. The player walks to successive
/// frontier waypoints until the goal is reachable or turns out to be blocked.
#[derive(Component)]
pub struct ExplorationGoal(pub Vec2);

#[derive(Component, Default)]
pub struct MoveTarget {
    pub destination: Vec2,
    pub origin: Vec2,
    pub active: bool,
    pub time_set: Duration,
}

impl MoveTarget {
    fn set(&mut self, destination: Vec2, origin: Vec2, now: Duration) {
        self.destination = destination;
        self.origin = origin;
        self.active = true;
        self.time_set = now;
    }
}

fn lerp(start: f32, end: f32, t: f32) -> f32 { start + (end - start) * t }

pub fn set_target_on_click(
    window: Single<&Window>,
    camera_query: Single<(&Camera, &GlobalTransform)>,
    mouse_button_input: Res<ButtonInput<MouseButton>>,
    mut player_query: Query<
        (Entity, &Transform, &mut MoveTarget, &mut AgentTarget2d),
        With<Player>,
    >,
    archipelago_query: Query<&Archipelago2d>,
    time: Res<Time>,
    exploration_state: Res<ExplorationState>,
    dungeon_state: Res<DungeonState>,
    mut commands: Commands,
    palette_state: Res<CommandPaletteState>,
) {
    if !mouse_button_input.just_pressed(MouseButton::Left) {
        return;
    }
    if palette_state.open {
        return;
    }

    let cursor_position = match window.cursor_position() {
        Some(position) => position,
        None => return,
    };

    let (camera, camera_transform) = *camera_query;
    let goal_position = match camera.viewport_to_world_2d(camera_transform, cursor_position) {
        Ok(world_pos) => world_pos,
        Err(_) => return,
    };

    let snapped_goal = snap_to_navmesh(goal_position, &archipelago_query);

    for (entity, transform, mut move_target, mut agent_target) in player_query.iter_mut() {
        let current_position = transform.translation.truncate();
        let distance = current_position.distance(snapped_goal);

        if distance <= STOP_THRESHOLD {
            move_target.active = false;
            *agent_target = AgentTarget2d::None;
            commands.entity(entity).remove::<ExplorationGoal>();
            continue;
        }

        let geo_target = geo::Point::new(goal_position.x, goal_position.y);
        if exploration_state.0.contains(&geo_target) {
            // Click is in unexplored territory: find a reachable frontier point with LOS to target.
            let known_blockers = dungeon_state.solid_rock.difference(&exploration_state.0);
            if let Some(waypoint) = find_exploration_waypoint(
                goal_position,
                &exploration_state.0,
                &known_blockers,
                &dungeon_state.playable_area,
            ) {
                let waypoint = snap_to_navmesh(waypoint, &archipelago_query);
                move_target.set(waypoint, current_position, time.elapsed());
                *agent_target = AgentTarget2d::Point(waypoint);
                commands.entity(entity).insert(ExplorationGoal(goal_position));
            }
            // If no frontier waypoint exists, ignore the click.
        } else {
            // Normal click in explored territory: path directly (snapped to navmesh).
            move_target.set(snapped_goal, current_position, time.elapsed());
            *agent_target = AgentTarget2d::Point(snapped_goal);
            commands.entity(entity).remove::<ExplorationGoal>();
        }
    }
}

/// When the player has an `ExplorationGoal` and has just finished moving, advance toward
/// the goal by finding the next frontier waypoint or, once the goal is revealed, pathing there.
pub fn advance_exploration(
    mut player_query: Query<
        (Entity, &Transform, &mut MoveTarget, &mut AgentTarget2d, &ExplorationGoal),
        With<Player>,
    >,
    archipelago_query: Query<&Archipelago2d>,
    exploration_state: Res<ExplorationState>,
    dungeon_state: Res<DungeonState>,
    mut commands: Commands,
    time: Res<Time>,
) {
    let Ok((entity, transform, mut move_target, mut agent_target, exploration_goal)) =
        player_query.single_mut()
    else {
        return;
    };

    if move_target.active {
        return;
    }

    let goal = exploration_goal.0;
    let current = transform.translation.truncate();

    if !exploration_state.0.contains(&geo::Point::new(goal.x, goal.y)) {
        // Goal is now in explored territory — path directly to it (snapped to navmesh).
        let snapped = snap_to_navmesh(goal, &archipelago_query);
        move_target.set(snapped, current, time.elapsed());
        *agent_target = AgentTarget2d::Point(snapped);
        commands.entity(entity).remove::<ExplorationGoal>();
    } else {
        let known_blockers = dungeon_state.solid_rock.difference(&exploration_state.0);
        match find_exploration_waypoint(
            goal,
            &exploration_state.0,
            &known_blockers,
            &dungeon_state.playable_area,
        ) {
            Some(waypoint) if waypoint.distance(current) > STOP_THRESHOLD => {
                let waypoint = snap_to_navmesh(waypoint, &archipelago_query);
                move_target.set(waypoint, current, time.elapsed());
                *agent_target = AgentTarget2d::Point(waypoint);
            }
            // Waypoint is within stop threshold (already there) or None — give up.
            _ => {
                commands.entity(entity).remove::<ExplorationGoal>();
            }
        }
    }
}

pub fn move_player(
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
    time: Res<Time>,
) {
    for (transform, mut velocity, mut move_target, desired_velocity, mut agent_target) in
        query.iter_mut()
    {
        if !move_target.active
            || (velocity.length() < PLAYER_SPEED / 100.0
                && (time.elapsed() - move_target.time_set) > Duration::from_millis(100))
        {
            move_target.active = false;
            *velocity = LinearVelocity::ZERO;
            continue;
        }

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

const DESCEND_RANGE: f32 = 40.0;

pub fn register_player_commands(mut registry: ResMut<PaletteRegistry>) {
    registry.commands.push(PaletteCommand {
        key: "d".to_string(),
        description: "Descend the staircase".to_string(),
        icon: None,
        kind: PaletteCommandKind::Instant,
    });
    registry.commands.push(PaletteCommand {
        key: ".".to_string(),
        description: "Stop moving".to_string(),
        icon: None,
        kind: PaletteCommandKind::Instant,
    });
}

pub fn execute_stop_command(
    mut palette: ResMut<CommandPaletteState>,
    mut player_query: Query<
        (Entity, &mut MoveTarget, &mut AgentTarget2d, &mut LinearVelocity),
        With<Player>,
    >,
    mut commands: Commands,
) {
    if palette.pending_command.as_deref() != Some(".") {
        return;
    }
    palette.pending_command = None;
    let Ok((entity, mut move_target, mut agent_target, mut velocity)) = player_query.single_mut()
    else {
        return;
    };
    move_target.active = false;
    *agent_target = AgentTarget2d::None;
    velocity.0 = Vec2::ZERO;
    commands.entity(entity).remove::<ExplorationGoal>();
}

pub fn execute_descend_command(
    mut palette: ResMut<CommandPaletteState>,
    player_query: Query<&Transform, With<Player>>,
    staircase_query: Query<&Transform, With<Staircase>>,
    mut next_state: ResMut<NextState<GameState>>,
    mut message_log: ResMut<MessageLog>,
) {
    if palette.pending_command.as_deref() != Some("d") {
        return;
    }
    palette.pending_command = None;
    let Ok(player_tf) = player_query.single() else { return };
    let player_pos = player_tf.translation.truncate();
    if staircase_query.iter().any(|tf| tf.translation.truncate().distance(player_pos) <= DESCEND_RANGE) {
        next_state.set(GameState::Descend);
    } else {
        message_log.push("You are not near the staircase.".to_string());
    }
}

pub fn directional_move_system(
    keyboard: Res<ButtonInput<Key>>,
    palette: Res<CommandPaletteState>,
    mut player_query: Query<
        (Entity, &Transform, &mut LinearVelocity, &mut MoveTarget, &mut AgentTarget2d),
        With<Player>,
    >,
    mut commands: Commands,
    time: Res<Time>,
) {
    if palette.open {
        return;
    }
    let mut dir = Vec2::ZERO;
    if keyboard.pressed(Key::Character("h".into())) || keyboard.pressed(Key::ArrowLeft) {
        dir += Vec2::new(-1.0, 0.0);
    }
    if keyboard.pressed(Key::Character("j".into())) || keyboard.pressed(Key::ArrowDown) {
        dir += Vec2::new(0.0, -1.0);
    }
    if keyboard.pressed(Key::Character("k".into())) || keyboard.pressed(Key::ArrowUp) {
        dir += Vec2::new(0.0, 1.0);
    }
    if keyboard.pressed(Key::Character("l".into())) || keyboard.pressed(Key::ArrowRight) {
        dir += Vec2::new(1.0, 0.0);
    }
    let dir = dir.normalize_or_zero();
    if dir == Vec2::ZERO {
        return;
    }
    let Ok((entity, transform, mut velocity, mut move_target, mut agent_target)) =
        player_query.single_mut()
    else {
        return;
    };
    let pos = transform.translation.truncate();
    move_target.destination = pos + dir * 10000.0;
    move_target.origin = pos;
    move_target.active = true;
    move_target.time_set = time.elapsed();
    velocity.0 = dir * PLAYER_DIRECTIONAL_SPEED;
    *agent_target = AgentTarget2d::None;
    commands.entity(entity).remove::<ExplorationGoal>();
}
