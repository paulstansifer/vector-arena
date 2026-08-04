// Go-to-point-of-interest command: label explored points a-z by distance, navigate to them.
// The label *storage* (LocationLabels, with hjkl/yubn pinned to directions) is an engine
// concept; this file only supplies the assignment *policy* (which points go where) and the
// "g" command itself, since the engine has no idea what "go to" means.
use bevy::prelude::*;
use bevy_egui::egui;
use bevy_landmass::prelude::*;

use rogue_angles::{
    dungeon::terrain::PointsOfInterest,
    fov::ExplorationState,
    hud::world_to_screen_pos,
    palette::{
        CommandInvocation, CommandPaletteState, CommandPaletteWatchesClicks, EntryOutcome,
        LocationDescriptions, LocationLabels, PaletteCommand, PaletteRegistry, Target,
        TargetFilter,
    },
};

use crate::{
    Staircase,
    player::{ExplorationGoal, MoveTarget, Player},
};

pub const GOTO_KEY: &str = "g";

pub fn register_goto_command(world: &mut World) {
    let handler = world.register_system(execute_goto_command);
    world.resource_mut::<PaletteRegistry>().commands.push(PaletteCommand {
        key: GOTO_KEY.to_string(),
        description: "Go to monster or location".to_string(),
        icon: None,
        outcome: EntryOutcome::PickTarget { verb: "Go to".to_string(), filter: TargetFilter::Any },
        handler,
    });
    // "g" is also the default action for typing a monster's letter with the
    // palette closed — see rogue_angles::palette::DefaultEntityAction.
    world.resource_mut::<rogue_angles::palette::DefaultEntityAction>().0 = Some(GOTO_KEY.to_string());
}

/// Keeps `LocationLabels` continuously up to date with the player's current
/// position and exploration state — the same "always live" model as
/// `EntityLabels`, rather than a once-per-palette-open snapshot. That matters
/// beyond the interactive UI: the engine's letter grammar (lowercase =
/// location) is meant to be usable from a programmatic driver — a headless
/// scripting harness, or an agent calling `execute_path_string` directly —
/// which never opens the palette UI at all, so gating this on
/// `CommandPaletteState`/`CommandPaletteWatchesClicks` would silently make
/// every location-letter target (e.g. "g h", "z s") unresolvable outside
/// interactive play.
pub fn compute_goto_assignments(
    exploration_state: Res<ExplorationState>,
    poi: Res<PointsOfInterest>,
    player_query: Query<&Transform, With<Player>>,
    item_query: Query<&Transform, With<crate::item::Item>>,
    staircase_query: Query<&Transform, With<Staircase>>,
    mut location_labels: ResMut<LocationLabels>,
    mut descriptions: ResMut<LocationDescriptions>,
) {
    let player_transform = match player_query.single() {
        Ok(tf) => tf,
        Err(_) => return,
    };
    let player_pos = player_transform.translation.truncate();

    let is_explored = |p: Vec2| exploration_state.is_explored(p);

    use rogue_angles::palette::{
        DIR_DOWN, DIR_DOWN_LEFT, DIR_DOWN_RIGHT, DIR_LEFT, DIR_RIGHT, DIR_UP, DIR_UP_LEFT,
        DIR_UP_RIGHT,
    };
    let idx = |c: char| c as usize - 'a' as usize;

    let cardinals = [
        (DIR_LEFT, player_pos + Vec2::new(-70.0, 0.0)),
        (DIR_DOWN, player_pos + Vec2::new(0.0, -70.0)),
        (DIR_UP, player_pos + Vec2::new(0.0, 70.0)),
        (DIR_RIGHT, player_pos + Vec2::new(70.0, 0.0)),
        (DIR_UP_LEFT, player_pos + Vec2::new(-50.0, 50.0)),
        (DIR_UP_RIGHT, player_pos + Vec2::new(50.0, 50.0)),
        (DIR_DOWN_LEFT, player_pos + Vec2::new(-50.0, -50.0)),
        (DIR_DOWN_RIGHT, player_pos + Vec2::new(50.0, -50.0)),
    ];
    for (idx, pos) in cardinals {
        location_labels.slots[idx] = is_explored(pos).then_some(pos);
    }

    // Reserve 's' for the staircase when it has been explored.
    if let Ok(tf) = staircase_query.single() {
        let pos = tf.translation.truncate();
        location_labels.slots[idx('s')] = is_explored(pos).then_some(pos);
        descriptions.0.insert('s', "staircase down".to_string());
    }

    // Fill remaining slots with interesting points sorted by distance.
    let mut reserved: Vec<usize> = cardinals.map(|x| x.0).iter().cloned().collect();
    reserved.push(idx('s'));
    let map_points = poi.points.iter().copied().filter(|&p| is_explored(p));
    let item_points =
        item_query.iter().map(|tf| tf.translation.truncate()).filter(|&p| is_explored(p));
    let mut candidates: Vec<Vec2> = map_points.chain(item_points).collect();
    // Only relative order matters, so compare squared distance and skip the sqrt — this runs
    // unconditionally every frame (not just while the palette is open).
    candidates.sort_by(|a, b| {
        player_pos
            .distance_squared(*a)
            .partial_cmp(&player_pos.distance_squared(*b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates.dedup_by(|a, b| a.distance_squared(*b) < 1.0);

    let mut cand_iter = candidates.into_iter();
    for i in 0..26usize {
        if reserved.contains(&i) {
            continue;
        }
        location_labels.slots[i] = cand_iter.next();
    }
}

pub fn render_goto_markers(
    palette: Res<CommandPaletteState>,
    watches_clicks: Res<CommandPaletteWatchesClicks>,
    location_labels: Res<LocationLabels>,
    mut egui_context: bevy_egui::EguiContexts,
    camera_query: Query<(&Camera, &GlobalTransform)>,
) {
    if !palette.open || !watches_clicks.0 || location_labels.slots.iter().all(|l| l.is_none()) {
        return;
    }

    let (camera, camera_transform) = match camera_query.single() {
        Ok((cam, tf)) => (cam, tf),
        Err(_) => return,
    };

    let ctx = match egui_context.ctx_mut() {
        Ok(c) => c,
        Err(_) => return,
    };

    let painter = ctx
        .layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("goto_markers")));

    for (i, opt_pos) in location_labels.slots.iter().enumerate() {
        let Some(pos) = opt_pos else { continue };
        let Some(screen_pos) = world_to_screen_pos(camera, camera_transform, pos.extend(0.0)) else {
            continue;
        };
        let radius = 10.0;
        let circle_color = egui::Color32::from_rgba_unmultiplied(100, 150, 255, 150);

        painter.circle_filled(screen_pos, radius, circle_color);

        let letter = (b'a' + i as u8) as char;
        painter.text(
            screen_pos,
            egui::Align2::CENTER_CENTER,
            letter.to_string(),
            egui::FontId::monospace(12.0),
            egui::Color32::WHITE,
        );
    }
}

pub fn execute_goto_command(
    In(invocation): In<CommandInvocation>,
    mut player_query: Query<
        (Entity, &Transform, &mut MoveTarget, &mut AgentTarget2d),
        With<Player>,
    >,
    all_transforms: Query<&Transform>,
    time: Res<Time>,
    mut commands: Commands,
) {
    let Some(target) = invocation.target else { return };
    let destination = match target {
        Target::Point(p) => p,
        Target::Entity(e) => match all_transforms.get(e) {
            Ok(tf) => tf.translation.truncate(),
            Err(_) => return,
        },
    };

    if let Ok((entity, transform, mut move_target, mut agent_target)) = player_query.single_mut() {
        let current_pos = transform.translation.truncate();
        move_target.destination = destination;
        move_target.origin = current_pos;
        move_target.active = true;
        move_target.time_set = time.elapsed();
        *agent_target = AgentTarget2d::Point(destination);
        commands.entity(entity).remove::<ExplorationGoal>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_app(destination: Option<Vec2>) -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<CommandPaletteState>();
        app.init_resource::<CommandPaletteWatchesClicks>();

        let player = app
            .world_mut()
            .spawn((
                Player,
                Transform::from_xyz(0.0, 0.0, 0.0),
                MoveTarget::default(),
                AgentTarget2d::None,
            ))
            .id();

        if let Some(pos) = destination {
            let handler = app.world_mut().register_system(execute_goto_command);
            app.world_mut()
                .run_system_with(handler, CommandInvocation {
                    path: vec![GOTO_KEY.to_string()],
                    target: Some(Target::Point(pos)),
                })
                .unwrap();
        }

        (app, player)
    }

    #[test]
    fn execute_goto_navigates_to_destination() {
        let (mut app, player) = make_app(Some(Vec2::new(100.0, 200.0)));
        app.update();

        let move_target = app.world().entity(player).get::<MoveTarget>().unwrap();
        assert!(move_target.active, "move target should be active");
        assert_eq!(move_target.destination, Vec2::new(100.0, 200.0));
    }

    #[test]
    fn execute_goto_no_op_without_pending() {
        let (mut app, player) = make_app(None);
        app.update();

        let move_target = app.world().entity(player).get::<MoveTarget>().unwrap();
        assert!(!move_target.active);
    }
}
