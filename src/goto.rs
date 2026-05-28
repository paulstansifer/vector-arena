// Go-to-point-of-interest command: label explored points a-z by distance, navigate to them.
use bevy::prelude::*;
use bevy_egui::egui;
use bevy_landmass::prelude::*;
use geo::Contains;

use crate::{
    command_palette::{CommandPaletteState, PaletteEntry},
    dungeon::terrain::PointsOfInterest,
    fov::ExplorationState,
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
    if !palette.open || !palette.input.starts_with('g') || goto_state.computed {
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

pub fn goto_completions(input: &str, state: &GotoState) -> Vec<PaletteEntry> {
    if input.is_empty() {
        return vec![PaletteEntry {
            key: "g".to_string(),
            description: "Go to point of interest".to_string(),
            icon: None,
            is_complete: false,
        }];
    }

    if !input.starts_with('g') {
        return vec![];
    }

    if input.trim_end() == "g" {
        return state
            .labels
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let letter = ('a' as u8 + i as u8) as char;
                PaletteEntry {
                    key: format!("g {}", letter),
                    description: format!("Go to point {}", letter),
                    icon: None,
                    is_complete: true,
                }
            })
            .collect();
    }

    if input.len() < 2 || input.chars().nth(1) != Some(' ') {
        return vec![];
    }

    let letters_str = &input[2..];

    // Validate all letters are valid labels
    for c in letters_str.chars() {
        let idx = (c as u8).wrapping_sub(b'a') as usize;
        if idx >= state.labels.len() {
            return vec![];
        }
    }

    // Build description based on number of letters
    let description = if letters_str.len() == 1 {
        format!("Go to point {}", letters_str)
    } else {
        let letter_list: String = letters_str
            .chars()
            .enumerate()
            .map(|(i, c)| if i == 0 { c.to_string() } else { format!(", {}", c) })
            .collect();
        format!("Go to average of {}", letter_list)
    };

    vec![PaletteEntry { key: input.to_string(), description, icon: None, is_complete: true }]
}

pub fn render_goto_markers(
    palette: Res<CommandPaletteState>,
    goto_state: Res<GotoState>,
    mut egui_context: bevy_egui::EguiContexts,
    camera_query: Query<(&Camera, &GlobalTransform)>,
) {
    if !palette.open || !palette.input.starts_with('g') || goto_state.labels.is_empty() {
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
    goto_state: Res<GotoState>,
    mut player_query: Query<
        (Entity, &Transform, &mut MoveTarget, &mut AgentTarget2d),
        With<Player>,
    >,
    time: Res<Time>,
    mut commands: Commands,
) {
    let command = match palette.pending_command.take() {
        Some(cmd) => cmd,
        None => return,
    };

    let parts: Vec<&str> = command.split_whitespace().collect();
    if parts.len() != 2 || parts[0] != "g" {
        return;
    }

    let letters_str = parts[1];
    let mut destination = Vec2::ZERO;
    let mut count = 0;

    // Parse each letter and sum positions
    for c in letters_str.chars() {
        let idx = (c as u8).wrapping_sub(b'a') as usize;
        if idx >= goto_state.labels.len() {
            return;
        }
        destination += goto_state.labels[idx];
        count += 1;
    }

    // Average the positions
    if count > 0 {
        destination /= count as f32;
    }

    // Initiate movement (explored destination, no ExplorationGoal needed)
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
        Err(_) => return,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_app() -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);

        let mut palette_state = CommandPaletteState::default();
        palette_state.pending_command = Some("g a".to_string());

        let mut goto_state = GotoState::default();
        goto_state.labels = vec![Vec2::new(100.0, 200.0), Vec2::new(300.0, 400.0)];
        goto_state.computed = true;

        app.insert_resource(palette_state);
        app.insert_resource(goto_state);

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
    fn execute_goto_sets_move_target_to_labeled_point() {
        let (mut app, player) = make_app();
        app.add_systems(Update, execute_goto_command);
        app.update();

        let move_target = app.world().entity(player).get::<MoveTarget>().unwrap();
        assert!(move_target.active, "move target should be active");
        assert_eq!(move_target.destination, Vec2::new(100.0, 200.0));
    }

    #[test]
    fn execute_goto_multi_letter_navigates_to_centroid() {
        let (mut app, player) = make_app();
        // Override command to "g ab"
        app.world_mut().resource_mut::<CommandPaletteState>().pending_command =
            Some("g ab".to_string());
        app.add_systems(Update, execute_goto_command);
        app.update();

        let move_target = app.world().entity(player).get::<MoveTarget>().unwrap();
        assert!(move_target.active);
        // centroid of (100,200) and (300,400) = (200, 300)
        assert_eq!(move_target.destination, Vec2::new(200.0, 300.0));
    }
}
