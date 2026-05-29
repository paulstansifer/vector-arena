// Go-to-point-of-interest command: label explored points a-z by distance, navigate to them.
use bevy::prelude::*;
use bevy_egui::egui;
use bevy_landmass::prelude::*;
use geo::Contains;

use crate::{
    command_palette::{CommandPaletteState, LetterMap, PaletteEntry, PendingClickTarget},
    dungeon::terrain::PointsOfInterest,
    fov::ExplorationState,
    monster::{Monster, MonsterState, Stats},
    player::{ExplorationGoal, MoveTarget, Player},
};

#[derive(Resource, Default)]
pub struct GotoState {
    pub labels: Vec<Vec2>, // labels[0] = 'a', up to 26 entries
    pub computed: bool,
}

pub fn compute_goto_assignments(
    palette: Res<CommandPaletteState>,
    exploration_state: Res<ExplorationState>,
    poi: Res<PointsOfInterest>,
    player_query: Query<&Transform, With<Player>>,
    item_query: Query<&Transform, With<crate::item::Item>>,
    mut goto_state: ResMut<GotoState>,
) {
    let active = palette.input.starts_with('g') || palette.input.starts_with('m');
    if !palette.open || !active || goto_state.computed {
        return;
    }

    let player_transform = match player_query.single() {
        Ok(tf) => tf,
        Err(_) => return,
    };
    let player_pos = player_transform.translation.truncate();

    let is_explored = |p: Vec2| !exploration_state.0.contains(&geo::Point::new(p.x, p.y));

    // Room centers and corridor ends from level generation.
    let map_points = poi.points.iter().copied().filter(|&p| is_explored(p));

    // Current item positions.
    let item_points =
        item_query.iter().map(|tf| tf.translation.truncate()).filter(|&p| is_explored(p));

    // 70px in each cardinal direction from the player, if explored.
    let cardinal_points = [
        player_pos + Vec2::new(0.0, 70.0),
        player_pos + Vec2::new(0.0, -70.0),
        player_pos + Vec2::new(70.0, 0.0),
        player_pos + Vec2::new(-70.0, 0.0),
    ]
    .into_iter()
    .filter(|&p| is_explored(p));

    let mut candidates: Vec<(Vec2, f32)> = map_points
        .chain(item_points)
        .chain(cardinal_points)
        .map(|p| (p, player_pos.distance(p)))
        .collect();

    candidates.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    candidates.dedup_by(|a, b| a.0.distance(b.0) < 1.0);

    goto_state.labels = candidates.into_iter().take(26).map(|(p, _)| p).collect();
    goto_state.computed = true;
}

pub fn reset_goto_on_close(palette: Res<CommandPaletteState>, mut goto_state: ResMut<GotoState>) {
    // Don't clear while a command is pending — execute_goto_command still needs the labels.
    if !palette.open && goto_state.computed && palette.pending_command.is_none() {
        goto_state.labels.clear();
        goto_state.computed = false;
    }
}

pub fn goto_completions(
    input: &str,
    goto_state: &GotoState,
    letter_map: &LetterMap,
    monster_query: &Query<(Entity, &Stats, &MonsterState, &Transform), With<Monster>>,
    current_fov: Option<&geo::MultiPolygon<f32>>,
) -> Vec<PaletteEntry> {
    if input.is_empty() {
        return vec![PaletteEntry {
            key: "g".to_string(),
            description: "Go to monster or location".to_string(),
            icon: None,
            is_complete: false,
        }];
    }
    if !input.starts_with('g') {
        return vec![];
    }
    crate::command_palette::targeting_sub_completions(
        input, 'g', "Go to", goto_state, letter_map, monster_query, current_fov,
    )
}

pub fn render_goto_markers(
    palette: Res<CommandPaletteState>,
    goto_state: Res<GotoState>,
    mut egui_context: bevy_egui::EguiContexts,
    camera_query: Query<(&Camera, &GlobalTransform)>,
) {
    let active = palette.input.starts_with('g') || palette.input.starts_with('m');
    if !palette.open || !active || goto_state.labels.is_empty() {
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

    for (i, pos) in goto_state.labels.iter().enumerate() {
        let world_pos = pos.extend(0.0);
        let viewport_pos = match camera.world_to_viewport(camera_transform, world_pos) {
            Ok(vp) => vp,
            Err(_) => continue,
        };

        // Skip if outside window bounds
        if viewport_pos.x < 0.0 || viewport_pos.y < 0.0 {
            continue;
        }

        let screen_pos = egui::Pos2::new(viewport_pos.x, viewport_pos.y);
        let radius = 10.0;
        let circle_color = egui::Color32::from_rgba_unmultiplied(100, 150, 255, 150);

        // Draw circle
        painter.circle_filled(screen_pos, radius, circle_color);

        // Draw letter label
        let letter = ('a' as u8 + i as u8) as char;
        let letter_str = letter.to_string();
        let text_color = egui::Color32::WHITE;

        painter.text(
            screen_pos,
            egui::Align2::CENTER_CENTER,
            letter_str,
            egui::FontId::monospace(12.0),
            text_color,
        );
    }
}

pub fn execute_goto_command(
    mut palette: ResMut<CommandPaletteState>,
    mut click_target: ResMut<PendingClickTarget>,
    goto_state: Res<GotoState>,
    letter_map: Res<LetterMap>,
    monster_query: Query<&Transform, With<Monster>>,
    mut player_query: Query<
        (Entity, &Transform, &mut MoveTarget, &mut AgentTarget2d),
        With<Player>,
    >,
    time: Res<Time>,
    mut commands: Commands,
) {
    let mut destination: Option<Vec2> = None;

    if let Some(cmd) = palette.pending_command.take() {
        if let Some(rest) = cmd.strip_prefix("g ") {
            let rest = rest.trim();
            if rest.len() == 1 {
                let c = rest.chars().next().unwrap();
                match c {
                    c if c.is_uppercase() => {
                        if let Some(entity) = letter_map.entity_for_letter(c) {
                            destination =
                                monster_query.get(entity).ok().map(|tf| tf.translation.truncate());
                        }
                    }
                    c if c.is_lowercase() => {
                        let idx = (c as u8).wrapping_sub(b'a') as usize;
                        if idx < goto_state.labels.len() {
                            destination = Some(goto_state.labels[idx]);
                        }
                    }
                    _ => {}
                }
            }
        } else {
            palette.pending_command = Some(cmd);
        }
    }

    if destination.is_none() {
        destination = click_target.goto_pos.take();
    }

    let Some(destination) = destination else { return };

    match player_query.single_mut() {
        Ok((entity, transform, mut move_target, mut agent_target)) => {
            let current_pos = transform.translation.truncate();
            move_target.destination = destination;
            move_target.origin = current_pos;
            move_target.active = true;
            move_target.time_set = time.elapsed();
            *agent_target = AgentTarget2d::Point(destination);
            commands.entity(entity).remove::<ExplorationGoal>();
        }
        Err(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_app(pending_cmd: &str) -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);

        let mut palette_state = CommandPaletteState::default();
        palette_state.pending_command =
            (!pending_cmd.is_empty()).then(|| pending_cmd.to_string());

        let mut goto_state = GotoState::default();
        goto_state.labels = vec![Vec2::new(50.0, 75.0), Vec2::new(300.0, 400.0)];
        goto_state.computed = true;

        let monster = app
            .world_mut()
            .spawn((Monster, Transform::from_xyz(100.0, 200.0, 0.0)))
            .id();

        let mut letter_map = LetterMap::default();
        letter_map.assign_monster(monster); // assigns 'A'

        app.insert_resource(palette_state);
        app.insert_resource(goto_state);
        app.insert_resource(letter_map);
        app.init_resource::<crate::command_palette::PendingClickTarget>();

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
    fn execute_goto_navigates_to_monster() {
        let (mut app, player) = make_app("g A");
        app.add_systems(Update, execute_goto_command);
        app.update();

        let move_target = app.world().entity(player).get::<MoveTarget>().unwrap();
        assert!(move_target.active, "move target should be active");
        assert_eq!(move_target.destination, Vec2::new(100.0, 200.0));
    }

    #[test]
    fn execute_goto_navigates_to_location() {
        let (mut app, player) = make_app("g a");
        app.add_systems(Update, execute_goto_command);
        app.update();

        let move_target = app.world().entity(player).get::<MoveTarget>().unwrap();
        assert!(move_target.active);
        assert_eq!(move_target.destination, Vec2::new(50.0, 75.0));
    }

    #[test]
    fn execute_goto_click_target_navigates_to_world_pos() {
        let (mut app, player) = make_app("");
        app.world_mut().resource_mut::<crate::command_palette::PendingClickTarget>().goto_pos =
            Some(Vec2::new(999.0, 888.0));
        app.add_systems(Update, execute_goto_command);
        app.update();

        let move_target = app.world().entity(player).get::<MoveTarget>().unwrap();
        assert!(move_target.active);
        assert_eq!(move_target.destination, Vec2::new(999.0, 888.0));
    }
}
