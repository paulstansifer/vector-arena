use avian2d::prelude::*;
use bevy::prelude::*;
use bevy_verlet::prelude::*;
use std::time::Duration;

/// Build a minimal app that runs avian2d physics headlessly.
///
/// `Plugin::finish` (not `build`) is where avian2d registers resources like
/// `CollisionDiagnostics`.  `App::run` calls `finish` internally, but tests
/// drive `update` directly, so we call it here.
pub fn physics_app(gravity: Vec2, ropes: bool) -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        TransformPlugin,
        AssetPlugin::default(),
        bevy::scene::ScenePlugin,
        PhysicsPlugins::default(),
    ))
    .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
        1.0 / 60.0,
    )))
    .insert_resource(Time::<Virtual>::default())
    .insert_resource(Gravity(gravity));

    if ropes {
        app.add_plugins(VerletPlugin::default()).insert_resource(VerletConfig {
            gravity: gravity.extend(0.0),
            friction: 0.02,
            sticks_computation_depth: 5,
            parallel_processing: false,
        });
        vector_arena::effects::rope::add_rope_test_systems(&mut app);
    }

    app.finish();
    app
}

/// Advance the app by one 60 Hz frame.
pub fn tick(app: &mut App) {
    app.world_mut().resource_mut::<Time<Virtual>>().advance_by(Duration::from_secs_f32(1.0 / 60.0));
    app.update();
}

/// Read the world position of an entity's `Transform`.
pub fn loc(app: &App, entity: Entity) -> Vec3 {
    app.world().entity(entity).get::<Transform>().unwrap().translation
}
