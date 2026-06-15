// Full-game smoke test and interactive listen-mode test harness.
//
// Run the smoke test:
//   cargo test start_complete_game
//
// Run in listen mode (reads commands from stdin):
//   GAME_LISTEN=1 cargo test start_complete_game -- --nocapture
//
// Listen-mode commands (one per line):
//   wait 0.5s          — advance game time by 0.5 seconds (30 frames at 60 Hz)
//   cmd d              — issue command-palette command with key "d"
//   click left 200 100 — move player toward world coordinate (200, 100)
//   level blank        — replace the current dungeon with an empty open room
//   snap /tmp/foo.png  — saves a snapshot of the current screen

#[path = "test_lib.rs"]
mod test_lib;
use test_lib::{headless_game_app, tick};

use bevy::prelude::*;
use std::time::Duration;

#[test]
fn start_complete_game() {
    let mut app = headless_game_app(Some(42));

    // Let startup systems run (OnEnter(Restart) → spawn world → transition to InLevel).
    for _ in 0..120 {
        tick(&mut app);
    }

    // Positive assertion: the game must have transitioned to InLevel and spawned a player.
    let state = app.world().resource::<State<vector_arena::GameState>>();
    assert_eq!(
        *state.get(),
        vector_arena::GameState::InLevel,
        "expected GameState::InLevel after startup"
    );
    let player_count =
        app.world_mut().query::<&vector_arena::player::Player>().iter(app.world()).count();
    assert_eq!(player_count, 1, "expected exactly one Player entity after startup");

    if std::env::var("GAME_LISTEN").is_ok() {
        eprintln!("[game_tests] listen mode active — reading commands from stdin");
        listen_loop(&mut app);
    }
}

fn listen_loop(app: &mut App) {
    use std::io::BufRead;
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l.trim().to_string(),
            Err(_) => break,
        };
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.splitn(5, ' ').collect();
        match parts.as_slice() {
            ["wait", duration_str] => cmd_wait(app, duration_str),
            ["cmd", key] => cmd_palette(app, key),
            ["click", "left", x_str, y_str] => cmd_click(app, x_str, y_str),
            ["level", "blank"] => cmd_level_blank(app),
            ["snap", path] => cmd_snap(app, path),
            _ => eprintln!("[game_tests] unknown command: {line}"),
        }
    }
    eprintln!("[game_tests] stdin closed, exiting listen mode");
}

fn cmd_wait(app: &mut App, duration_str: &str) {
    let secs: f32 = match duration_str.trim_end_matches('s').parse() {
        Ok(v) => v,
        Err(_) => {
            eprintln!("[game_tests] wait: could not parse duration '{duration_str}'");
            return;
        }
    };
    let frames = (secs * 60.0).round() as u32;
    eprintln!("[game_tests] wait {secs}s ({frames} frames)");
    for _ in 0..frames {
        app.world_mut()
            .resource_mut::<Time<Virtual>>()
            .advance_by(Duration::from_secs_f32(1.0 / 60.0));
        app.update();
    }
}

fn cmd_palette(app: &mut App, key: &str) {
    eprintln!("[game_tests] cmd {key}");
    app.world_mut()
        .resource_mut::<vector_arena::command_palette::CommandPaletteState>()
        .pending_command = Some(key.to_string());
    tick(app);
}

fn cmd_click(app: &mut App, x_str: &str, y_str: &str) {
    let (x, y): (f32, f32) = match (x_str.parse(), y_str.parse()) {
        (Ok(x), Ok(y)) => (x, y),
        _ => {
            eprintln!("[game_tests] click: could not parse coordinates '{x_str}' '{y_str}'");
            return;
        }
    };
    let destination = Vec2::new(x, y);
    eprintln!("[game_tests] click left {x} {y}");

    // Find the player and set its move target directly.
    let world = app.world_mut();
    let mut player_q =
        world.query_filtered::<(Entity, &Transform), With<vector_arena::player::Player>>();
    let Ok((player_entity, transform)) = player_q.single(world) else {
        eprintln!("[game_tests] click: no player entity found");
        return;
    };
    let origin = transform.translation.truncate();
    let now = world.resource::<Time>().elapsed();
    world.entity_mut(player_entity).get_mut::<vector_arena::player::MoveTarget>().map(|mut mt| {
        mt.destination = destination;
        mt.origin = origin;
        mt.active = true;
        mt.time_set = now;
    });
    // Also set the landmass agent target for pathfinding.
    world.entity_mut(player_entity).get_mut::<bevy_landmass::prelude::AgentTarget2d>().map(
        |mut at| {
            *at = bevy_landmass::prelude::AgentTarget2d::Entity(player_entity);
        },
    );
    drop(player_q);

    tick(app);
}

fn cmd_snap(app: &mut App, path: &str) {
    eprintln!("[game_tests] snap {path}");
    test_lib::snap::save(app, path);
    eprintln!("[game_tests] snap saved to {path}");
}

fn cmd_level_blank(app: &mut App) {
    use geo::{LineString, MultiPolygon, Polygon, coord};

    eprintln!("[game_tests] level blank");
    let half_w = vector_arena::WORLD_WIDTH / 2.0 - 20.0;
    let half_h = vector_arena::WORLD_HEIGHT / 2.0 - 20.0;

    let playable = MultiPolygon::new(vec![Polygon::new(
        LineString::from(vec![
            coord! {x: -half_w, y: -half_h},
            coord! {x:  half_w, y: -half_h},
            coord! {x:  half_w, y:  half_h},
            coord! {x: -half_w, y:  half_h},
            coord! {x: -half_w, y: -half_h},
        ]),
        vec![],
    )]);

    let mut dungeon =
        app.world_mut().resource_mut::<vector_arena::dungeon::terrain::DungeonState>();
    dungeon.solid_rock = MultiPolygon::new(vec![]);
    dungeon.playable_area = playable;
    dungeon.torpor_zones = vec![];

    tick(app);
}
