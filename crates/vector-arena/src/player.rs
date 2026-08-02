use std::time::Duration;

// Player component, click-to-move targeting, and steering.
// `MoveTarget` drives both the custom lerped-velocity steering and the Landmass
// `AgentTarget2d` (so the navmesh path is kept current even though the player
// uses its own steering rather than the landmass-computed velocity).
use avian2d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::prelude::*;
use geo::Contains;

use bevy::input::keyboard::Key;
use rand::Rng;

use rogue_angles::{
    dungeon::terrain::{DungeonState, TorporMultiplier},
    fov::{ExplorationState, find_exploration_waypoint},
    hud::MessageLog,
    nav::{STEERING_GAIN, STOP_THRESHOLD, snap_to_navmesh},
    palette::{CommandInvocation, CommandPaletteState, EntryOutcome, PaletteCommand, PaletteRegistry},
};

use crate::{GameState, Staircase, status_effect::StatusEffects};

pub const BOREDOM_MAX: f32 = 60.0;
const BOREDOM_WARN: f32 = 40.0;

#[derive(Resource, Default)]
pub struct Boredom {
    pub seconds: f32,
    warned: bool,
}

impl Boredom {
    pub fn reduce(&mut self, secs: f32) { self.seconds = (self.seconds - secs).max(0.0); }
}

pub fn tick_boredom(
    mut boredom: ResMut<Boredom>,
    time: Res<Time>,
    mut log: ResMut<MessageLog>,
    mut player_query: Query<&mut crate::monster::Stats, With<Player>>,
) {
    boredom.seconds += time.delta_secs();

    if boredom.seconds < BOREDOM_WARN {
        boredom.warned = false;
    }

    if boredom.seconds >= BOREDOM_WARN && !boredom.warned {
        boredom.warned = true;
        let suggestion = if rand::thread_rng().gen_bool(0.5) {
            "try an unknown item"
        } else {
            "blow something up"
        };
        log.push(format!("This is getting boring. Maybe you should {suggestion}."));
    }

    if boredom.seconds >= BOREDOM_MAX {
        boredom.seconds -= 15.0;
        log.push("You're so bored that it hurts! (-10 HP)");
        if let Ok(mut stats) = player_query.single_mut() {
            deal_damage_to_player(&mut stats, 10.0);
        }
    }
}

pub fn rotate_player_to_velocity(mut query: Query<(&LinearVelocity, &mut Rotation), With<Player>>) {
    for (velocity, mut rotation) in &mut query {
        if velocity.length_squared() > 1.0 {
            // Wizard sprite faces up (+Y); atan2 gives angle from +X, so offset by -PI/2.
            let angle = velocity.y.atan2(velocity.x) - std::f32::consts::FRAC_PI_2;
            *rotation = Rotation::radians(angle);
        }
    }
}

pub const PLAYER_SPEED: f32 = 320.0;
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
    pub directional: bool,
}

impl MoveTarget {
    fn set(&mut self, destination: Vec2, origin: Vec2, now: Duration) {
        self.destination = destination;
        self.origin = origin;
        self.active = true;
        self.directional = false;
        self.time_set = now;
    }
}

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
        (&LinearVelocity, &Transform, &mut MoveTarget, &mut AgentTarget2d, &mut AgentSettings),
        With<Player>,
    >,
    time: Res<Time>,
) {
    for (linear_vel, transform, mut move_target, mut agent_target, mut settings) in query.iter_mut()
    {
        let current_speed = linear_vel.length();

        if !move_target.active
            || (current_speed < PLAYER_SPEED / 100.0
                && (time.elapsed() - move_target.time_set) > Duration::from_millis(500))
        {
            move_target.active = false;
            settings.desired_speed = 0.0;
            *agent_target = AgentTarget2d::None;
            continue;
        }

        let current = transform.translation.truncate();
        let distance = (move_target.destination - current).length();

        if distance <= STOP_THRESHOLD {
            settings.desired_speed = 0.0;
            move_target.active = false;
            *agent_target = AgentTarget2d::None;
            continue;
        }

        settings.desired_speed = PLAYER_SPEED;
        settings.max_speed = PLAYER_SPEED * 1.2;
    }
}

const DESCEND_RANGE: f32 = 40.0;

pub fn register_player_commands(world: &mut World) {
    let stop_handler = world.register_system(execute_stop_command);
    let descend_handler = world.register_system(execute_descend_command);
    let mut registry = world.resource_mut::<PaletteRegistry>();
    registry.commands.push(PaletteCommand {
        key: "d".to_string(),
        description: "Descend the staircase".to_string(),
        icon: None,
        outcome: EntryOutcome::Run,
        handler: descend_handler,
    });
    registry.commands.push(PaletteCommand {
        key: ".".to_string(),
        description: "Stop moving".to_string(),
        icon: None,
        outcome: EntryOutcome::Run,
        handler: stop_handler,
    });
}

pub fn execute_stop_command(
    In(_invocation): In<CommandInvocation>,
    mut player_query: Query<
        (Entity, &mut MoveTarget, &mut AgentTarget2d, &mut LinearVelocity),
        With<Player>,
    >,
    mut commands: Commands,
) {
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
    In(_invocation): In<CommandInvocation>,
    player_query: Query<&Transform, With<Player>>,
    staircase_query: Query<&Transform, With<Staircase>>,
    mut next_state: ResMut<NextState<GameState>>,
    mut message_log: ResMut<MessageLog>,
) {
    let Ok(player_tf) = player_query.single() else { return };
    let player_pos = player_tf.translation.truncate();
    if staircase_query
        .iter()
        .any(|tf| tf.translation.truncate().distance(player_pos) <= DESCEND_RANGE)
    {
        next_state.set(GameState::Descend);
    } else {
        message_log.push("You are not near the staircase.".to_string());
    }
}

pub fn directional_move_system(
    keyboard: Res<ButtonInput<Key>>,
    palette: Res<CommandPaletteState>,
    mut player_query: Query<
        (
            Entity,
            Forces,
            &Transform,
            &mut MoveTarget,
            &mut AgentTarget2d,
            Option<&StatusEffects>,
            Option<&TorporMultiplier>,
        ),
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
    if keyboard.pressed(Key::Character("y".into())) {
        dir += Vec2::new(-1.0, 1.0);
    }
    if keyboard.pressed(Key::Character("u".into())) {
        dir += Vec2::new(1.0, 1.0);
    }
    if keyboard.pressed(Key::Character("b".into())) {
        dir += Vec2::new(-1.0, -1.0);
    }
    if keyboard.pressed(Key::Character("n".into())) {
        dir += Vec2::new(1.0, -1.0);
    }

    let dir = dir.normalize_or_zero();
    let Ok((entity, mut forces, transform, mut move_target, mut agent_target, effects, torpor)) =
        player_query.single_mut()
    else {
        return;
    };
    if dir == Vec2::ZERO {
        if move_target.directional {
            move_target.active = false;
            move_target.directional = false;
            *agent_target = AgentTarget2d::None;
        }
        return;
    }
    let pos = transform.translation.truncate();
    move_target.destination = pos + dir * 10000.0;
    move_target.origin = pos;
    move_target.active = true;
    move_target.directional = true;
    move_target.time_set = time.elapsed();
    let speed_mult = effects.map(|e| e.speed_multiplier()).unwrap_or(1.0)
        * torpor.map(|t| t.get()).unwrap_or(1.0);
    let desired_vel = dir * PLAYER_DIRECTIONAL_SPEED * speed_mult;
    let correction = desired_vel - forces.linear_velocity();
    forces.reset_accumulated_linear_acceleration();
    forces.apply_linear_acceleration(correction * STEERING_GAIN);
    *agent_target = AgentTarget2d::None;
    commands.entity(entity).remove::<ExplorationGoal>();
}

/// Applies `amount` damage to the player's stats, clamping HP at 0.
/// Returns the actual HP lost (may be less than `amount` if HP was already low).
pub fn deal_damage_to_player(stats: &mut crate::monster::Stats, amount: f32) -> f32 {
    let before = stats.hp.max(0.0);
    stats.hp = (stats.hp - amount).max(0.0);
    before - stats.hp
}

/// Detects the moment player HP hits zero and transitions to the GameOver screen.
/// Runs only in InLevel state so it fires at most once per death.
pub fn check_player_death(
    player_query: Query<&crate::monster::Stats, With<Player>>,
    mut next_state: ResMut<NextState<GameState>>,
    mut log: ResMut<MessageLog>,
) {
    let Ok(stats) = player_query.single() else { return };
    if stats.hp <= 0.0 {
        log.push("Alas! You have been slain!");
        next_state.set(GameState::GameOver);
    }
}
