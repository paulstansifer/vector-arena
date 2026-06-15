use bevy::prelude::*;
use vector_arena::{
    WORLD_HEIGHT, WORLD_WIDTH,
    game::GamePlugin,
    ui::{BOTTOM_PANEL_HEIGHT, TOP_PANEL_HEIGHT},
};

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Vector Arena".into(),
                        resolution: (
                            WORLD_WIDTH as u32,
                            (WORLD_HEIGHT + TOP_PANEL_HEIGHT + BOTTOM_PANEL_HEIGHT) as u32,
                        )
                            .into(),
                        ..default()
                    }),
                    ..default()
                })
                .set(bevy::log::LogPlugin {
                    // Avian logs a benign "Tried to wake body … that does not exist" warning for
                    // every dynamic body that's still in a sleeping island when the whole level is
                    // despawned in one frame on level change (rubble, missiles, etc.). Silence just
                    // that target; keep Bevy's defaults for everything else.
                    filter: format!(
                        "{},avian2d::dynamics::solver::islands::sleeping=error",
                        bevy::log::DEFAULT_FILTER
                    ),
                    ..default()
                }),
            GamePlugin::default(),
        ))
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands) {
    commands.insert_resource(ClearColor(Color::srgb(0.9, 0.9, 0.9)));
    // Offset the camera so the world (centered at origin) is centered between the two UI bars.
    let cam_y = (TOP_PANEL_HEIGHT - BOTTOM_PANEL_HEIGHT) / 2.0;
    commands.spawn((Camera2d, Transform::from_translation(Vec3::new(0.0, cam_y, 0.0))));
}
