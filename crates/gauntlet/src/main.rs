use bevy::prelude::*;
use gauntlet::{
    WORLD_HEIGHT, WORLD_WIDTH,
    game::GamePlugin,
    ui::{BOTTOM_PANEL_HEIGHT, TOP_PANEL_HEIGHT},
};

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Gauntlet".into(),
                    resolution: (
                        WORLD_WIDTH as u32,
                        (WORLD_HEIGHT + TOP_PANEL_HEIGHT + BOTTOM_PANEL_HEIGHT) as u32,
                    )
                        .into(),
                    #[cfg(target_arch = "wasm32")]
                    canvas: Some("#bevy-canvas".into()),
                    ..default()
                }),
                ..default()
            }),
            GamePlugin::default(),
        ))
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands) {
    commands.insert_resource(ClearColor(Color::srgb(0.12, 0.12, 0.14)));
    let cam_y = (TOP_PANEL_HEIGHT - BOTTOM_PANEL_HEIGHT) / 2.0;
    commands.spawn((Camera2d, Transform::from_translation(Vec3::new(0.0, cam_y, 0.0))));
}
