// Player component, click-to-move targeting, and steering. Mirrors vector-arena's
// player.rs pattern (MoveTarget drives both the lerped-velocity steering and the Landmass
// AgentTarget2d) but without directional-key movement or a command palette — this game's
// only inputs are "click to move" and "right-click to fire" (dig_shot.rs).
use std::time::Duration;

use avian2d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::prelude::*;
use geo::Contains;

use rogue_angles::{
    dungeon::terrain::DungeonState,
    fov::{ExplorationState, find_exploration_waypoint},
    nav::{STOP_THRESHOLD, snap_to_navmesh},
};

use rogue_angles::hud::MessageLog;

use crate::GameState;

pub const PLAYER_SPEED: f32 = 280.0;
pub const PLAYER_MAX_HP: f32 = 40.0;

/// Monsters never take damage in this game (dig shots only knock back and slow them), so
/// `Stats` — unlike `vector-arena`'s — belongs only to the player.
#[derive(Component, Default, Clone, Copy)]
pub struct Stats {
    pub hp: f32,
    pub max_hp: f32,
}

#[derive(Component)]
pub struct Player;

/// Fire-rate limiter for dig shots; see `dig_shot::fire_dig_shot`.
#[derive(Component)]
pub struct FireCooldown(pub f32);

/// When present, indicates an ongoing auto-explore toward a point that was in the
/// never-explored area when clicked. The player walks to successive frontier waypoints
/// until the goal is reachable or turns out to be blocked.
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
) {
    if !mouse_button_input.just_pressed(MouseButton::Left) {
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
        } else {
            move_target.set(snapped_goal, current_position, time.elapsed());
            *agent_target = AgentTarget2d::Point(snapped_goal);
            commands.entity(entity).remove::<ExplorationGoal>();
        }
    }
}

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
        let snapped = snap_to_navmesh(goal, &archipelago_query);
        move_target.set(snapped, current, time.elapsed());
        *agent_target = AgentTarget2d::Point(snapped);
        commands.entity(entity).remove::<ExplorationGoal>();
        return;
    }

    let known_blockers = dungeon_state.solid_rock.difference(&exploration_state.0);
    if let Some(waypoint) = find_exploration_waypoint(
        goal,
        &exploration_state.0,
        &known_blockers,
        &dungeon_state.playable_area,
    ) {
        let waypoint = snap_to_navmesh(waypoint, &archipelago_query);
        move_target.set(waypoint, current, time.elapsed());
        *agent_target = AgentTarget2d::Point(waypoint);
    } else {
        commands.entity(entity).remove::<ExplorationGoal>();
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

pub fn rotate_player_to_velocity(mut query: Query<(&LinearVelocity, &mut Rotation), With<Player>>) {
    for (velocity, mut rotation) in &mut query {
        if velocity.length_squared() > 1.0 {
            let angle = velocity.y.atan2(velocity.x) - std::f32::consts::FRAC_PI_2;
            *rotation = Rotation::radians(angle);
        }
    }
}

pub fn deal_damage_to_player(stats: &mut Stats, amount: f32) -> f32 {
    let before = stats.hp.max(0.0);
    stats.hp = (stats.hp - amount).max(0.0);
    before - stats.hp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deal_damage_clamps_at_zero_and_returns_actual_damage() {
        let mut stats = Stats { hp: 5.0, max_hp: 40.0 };
        let dealt = deal_damage_to_player(&mut stats, 12.0);
        assert_eq!(stats.hp, 0.0);
        assert_eq!(dealt, 5.0, "should report only the damage actually absorbed, not the full hit");
    }

    #[test]
    fn deal_damage_below_max_hp_is_unclamped() {
        let mut stats = Stats { hp: 40.0, max_hp: 40.0 };
        let dealt = deal_damage_to_player(&mut stats, 12.0);
        assert_eq!(stats.hp, 28.0);
        assert_eq!(dealt, 12.0);
    }
}

/// Detects the moment player HP hits zero and transitions to the GameOver screen.
/// Runs only in InLevel state so it fires at most once per death.
pub fn check_player_death(
    player_query: Query<&Stats, With<Player>>,
    mut next_state: ResMut<NextState<GameState>>,
    mut log: ResMut<MessageLog>,
) {
    let Ok(stats) = player_query.single() else { return };
    if stats.hp <= 0.0 {
        log.push("A monster's lunge finishes you off!");
        next_state.set(GameState::GameOver);
    }
}

/// Advances the fire-cooldown timer every frame; see `dig_shot::fire_dig_shot`.
pub fn tick_fire_cooldown(time: Res<Time>, mut query: Query<&mut FireCooldown>) {
    for mut cd in &mut query {
        cd.0 = (cd.0 - time.delta_secs()).max(0.0);
    }
}
