// Go-to-point-of-interest command: label explored points a-z by distance, navigate to them.
use bevy::prelude::*;
use bevy_egui::egui;
use bevy_landmass::prelude::*;

use crate::{
    command_palette::{
        CommandPaletteState, CommandPaletteWatchesClicks, PaletteCommand, PaletteCommandKind,
        PaletteRegistry,
    },
    dungeon::terrain::PointsOfInterest,
    fov::ExplorationState,
    player::{ExplorationGoal, MoveTarget, Player},
};

pub const GOTO_KEY: &str = "g";

#[derive(Resource, Default)]
pub struct GotoState {
    /// Index 0 = 'a', ..., 25 = 'z'. Indices 7/9/10/11 (h/j/k/l) are reserved for
    /// cardinal directions (left/down/up/right). None = no label assigned.
    pub labels: [Option<Vec2>; 26],
    pub computed: bool,
}

pub fn register_goto_command(mut registry: ResMut<PaletteRegistry>) {
    registry.commands.push(PaletteCommand {
        key: GOTO_KEY.to_string(),
        description: "Go to monster or location".to_string(),
        icon: None,
        kind: PaletteCommandKind::LocationTarget { target_verb: "Go to" },
    });
}

pub fn compute_goto_assignments(
    palette: Res<CommandPaletteState>,
    watches_clicks: Res<CommandPaletteWatchesClicks>,
    exploration_state: Res<ExplorationState>,
    poi: Res<PointsOfInterest>,
    player_query: Query<&Transform, With<Player>>,
    item_query: Query<&Transform, With<crate::item::Item>>,
    mut goto_state: ResMut<GotoState>,
) {
    if !palette.open || !watches_clicks.0 || goto_state.computed {
        return;
    }

    let player_transform = match player_query.single() {
        Ok(tf) => tf,
        Err(_) => return,
    };
    let player_pos = player_transform.translation.truncate();

    let is_explored = |p: Vec2| {
        use geo::Contains;
        !exploration_state.0.contains(&geo::Point::new(p.x, p.y))
    };

    // TODO: this is a bit of a hack!
    let idx = |c: char| c as usize - 'a' as usize;

    let cardinals = [
        (idx('h'), player_pos + Vec2::new(-70.0, 0.0)),
        (idx('j'), player_pos + Vec2::new(0.0, -70.0)),
        (idx('k'), player_pos + Vec2::new(0.0, 70.0)),
        (idx('l'), player_pos + Vec2::new(70.0, 0.0)),
        (idx('y'), player_pos + Vec2::new(-50.0, 50.0)),
        (idx('u'), player_pos + Vec2::new(50.0, 50.0)),
        (idx('b'), player_pos + Vec2::new(-50.0, -50.0)),
        (idx('n'), player_pos + Vec2::new(50.0, -50.0)),
    ];
    for (idx, pos) in cardinals {
        goto_state.labels[idx] = is_explored(pos).then_some(pos);
    }

    // Fill remaining slots with interesting points sorted by distance.
    let reserved: Vec<usize> = cardinals.map(|x| x.0).iter().cloned().collect();
    let map_points = poi.points.iter().copied().filter(|&p| is_explored(p));
    let item_points =
        item_query.iter().map(|tf| tf.translation.truncate()).filter(|&p| is_explored(p));
    let mut candidates: Vec<Vec2> = map_points.chain(item_points).collect();
    candidates.sort_by(|a, b| {
        player_pos
            .distance(*a)
            .partial_cmp(&player_pos.distance(*b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates.dedup_by(|a, b| a.distance(*b) < 1.0);

    let mut cand_iter = candidates.into_iter();
    for i in 0..26usize {
        if reserved.contains(&i) {
            continue;
        }
        goto_state.labels[i] = cand_iter.next();
    }

    goto_state.computed = true;
}

pub fn reset_goto_on_close(palette: Res<CommandPaletteState>, mut goto_state: ResMut<GotoState>) {
    if !palette.open && goto_state.computed {
        goto_state.labels = [None; 26];
        goto_state.computed = false;
    }
}

pub fn render_goto_markers(
    palette: Res<CommandPaletteState>,
    watches_clicks: Res<CommandPaletteWatchesClicks>,
    goto_state: Res<GotoState>,
    mut egui_context: bevy_egui::EguiContexts,
    camera_query: Query<(&Camera, &GlobalTransform)>,
) {
    if !palette.open || !watches_clicks.0 || goto_state.labels.iter().all(|l| l.is_none()) {
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

    for (i, opt_pos) in goto_state.labels.iter().enumerate() {
        let Some(pos) = opt_pos else { continue };
        let world_pos = pos.extend(0.0);
        let viewport_pos = match camera.world_to_viewport(camera_transform, world_pos) {
            Ok(vp) => vp,
            Err(_) => continue,
        };

        if viewport_pos.x < 0.0 || viewport_pos.y < 0.0 {
            continue;
        }

        let screen_pos = egui::Pos2::new(viewport_pos.x, viewport_pos.y);
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
    mut palette: ResMut<CommandPaletteState>,
    mut player_query: Query<
        (Entity, &Transform, &mut MoveTarget, &mut AgentTarget2d),
        With<Player>,
    >,
    time: Res<Time>,
    mut commands: Commands,
) {
    if palette.pending_command.as_deref() != Some(GOTO_KEY) {
        return;
    }
    palette.pending_command = None;
    let Some(destination) = palette.pending_target.take() else { return };

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
    use crate::command_palette::CommandPaletteState;

    fn make_app(destination: Option<Vec2>) -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);

        let mut palette_state = CommandPaletteState::default();
        if let Some(pos) = destination {
            palette_state.pending_command = Some(GOTO_KEY.to_string());
            palette_state.pending_target = Some(pos);
        }

        app.insert_resource(palette_state);
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

        (app, player)
    }

    #[test]
    fn execute_goto_navigates_to_destination() {
        let (mut app, player) = make_app(Some(Vec2::new(100.0, 200.0)));
        app.add_systems(Update, execute_goto_command);
        app.update();

        let move_target = app.world().entity(player).get::<MoveTarget>().unwrap();
        assert!(move_target.active, "move target should be active");
        assert_eq!(move_target.destination, Vec2::new(100.0, 200.0));
    }

    #[test]
    fn execute_goto_no_op_without_pending() {
        let (mut app, player) = make_app(None);
        app.add_systems(Update, execute_goto_command);
        app.update();

        let move_target = app.world().entity(player).get::<MoveTarget>().unwrap();
        assert!(!move_target.active);
    }
}
