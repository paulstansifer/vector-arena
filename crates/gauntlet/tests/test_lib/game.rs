use bevy::prelude::*;
use std::time::Duration;

/// Advance the app by one 60 Hz frame.
pub fn tick(app: &mut App) {
    app.world_mut().resource_mut::<Time<Virtual>>().advance_by(Duration::from_secs_f32(1.0 / 60.0));
    app.update();
}

/// Build a headless full-game app (no window, no Winit event loop).
///
/// Passes `seed` to the dungeon generator when `Some`; otherwise uses thread_rng.
pub fn headless_game_app(seed: Option<u64>) -> App {
    use bevy::{window::ExitCondition, winit::WinitPlugin};
    use gauntlet::game::{DungeonSeedOverride, GamePlugin};

    let mut app = App::new();
    app.add_plugins((
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .disable::<WinitPlugin>()
            .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>(),
        GamePlugin { headless: true },
    ));
    if let Some(s) = seed {
        app.insert_resource(DungeonSeedOverride(s));
    }
    app.finish();
    app
}
