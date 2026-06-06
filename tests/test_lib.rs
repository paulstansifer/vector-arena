// Shared headless-physics helpers used by multiple integration test files.
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

// ---------------------------------------------------------------------------
// Generic Bevy preview window for test geometry visualization.
//
// Usage:
//   PREVIEW_MS=100  cargo test <name>   – live window, 100 ms between frames
//   IMAGE_DIR=/tmp  cargo test <name>   – headless render, saves 0000.png …
// ---------------------------------------------------------------------------
pub mod preview {
    use bevy::prelude::*;
    use geo::MultiPolygon;
    use std::{
        path::PathBuf,
        sync::{Mutex, mpsc},
    };
    use vector_arena::dungeon::terrain;

    // ── Public frame types ───────────────────────────────────────────────────

    /// One renderable layer in a preview frame.
    pub struct Layer {
        pub mesh: Mesh,
        pub color: Color,
        /// World-space x/y.  For geometry meshes whose coords are already in world
        /// space use `Vec2::ZERO`; for point markers use the point's position.
        pub position: Vec2,
        /// Render depth (higher = in front).
        pub z: f32,
    }

    /// A snapshot to display in the preview window.
    pub struct Frame {
        pub layers: Vec<Layer>,
    }

    /// Convenience: build a `Layer` from a `MultiPolygon`.
    pub fn poly_layer(poly: &MultiPolygon<f32>, color: Color, z: f32) -> Layer {
        Layer { mesh: terrain::geometry_to_mesh(poly), color, position: Vec2::ZERO, z }
    }

    /// Convenience: build a circular point-marker `Layer`.
    pub fn point_layer(pos: Vec2, radius: f32, color: Color, z: f32) -> Layer {
        Layer { mesh: Mesh::from(Circle::new(radius)), color, position: pos, z }
    }

    // ── Shared internal message type ─────────────────────────────────────────

    enum Msg {
        Show(Frame),
        /// Headless mode: render, save to disk, then unblock the sender.
        Save {
            frame: Frame,
            notify: mpsc::SyncSender<()>,
        },
        Quit,
    }

    // ── Shared entity marker ─────────────────────────────────────────────────

    #[derive(Component)]
    struct FrameEntity;

    fn despawn_frame(commands: &mut Commands, q: &Query<Entity, With<FrameEntity>>) {
        for e in q.iter() {
            commands.entity(e).despawn();
        }
    }

    fn spawn_layers(
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<ColorMaterial>,
        layers: Vec<Layer>,
    ) {
        for layer in layers {
            commands.spawn((
                FrameEntity,
                Mesh2d(meshes.add(layer.mesh)),
                MeshMaterial2d(materials.add(ColorMaterial::from(layer.color))),
                Transform::from_translation(layer.position.extend(layer.z)),
            ));
        }
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // WINDOWED MODE
    // ═══════════════════════════════════════════════════════════════════════════

    #[derive(Resource)]
    struct WindowedRx(Mutex<mpsc::Receiver<Msg>>);
    impl WindowedRx {
        fn try_recv(&self) -> Option<Msg> { self.0.lock().unwrap().try_recv().ok() }
    }

    #[derive(Resource)]
    struct CamCenter(Vec2);

    fn windowed_setup_camera(cam: Res<CamCenter>, mut commands: Commands) {
        commands.spawn((Camera2d, Transform::from_translation(cam.0.extend(0.0))));
    }

    fn windowed_handle_msgs(
        rx: Res<WindowedRx>,
        mut exit: MessageWriter<AppExit>,
        mut commands: Commands,
        frame_entities: Query<Entity, With<FrameEntity>>,
        mut meshes: ResMut<Assets<Mesh>>,
        mut materials: ResMut<Assets<ColorMaterial>>,
    ) {
        while let Some(msg) = rx.try_recv() {
            match msg {
                Msg::Show(frame) => {
                    despawn_frame(&mut commands, &frame_entities);
                    spawn_layers(&mut commands, &mut meshes, &mut materials, frame.layers);
                }
                Msg::Quit => {
                    exit.write(AppExit::Success);
                }
                Msg::Save { .. } => {} // not used in windowed mode
            }
        }
    }

    fn launch_windowed(
        title: String,
        camera_center: Vec2,
    ) -> (mpsc::Sender<Msg>, std::thread::JoinHandle<()>) {
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            App::new()
                .add_plugins(
                    DefaultPlugins
                        .set(WindowPlugin {
                            primary_window: Some(Window {
                                title,
                                resolution: (900u32, 500u32).into(),
                                ..default()
                            }),
                            ..default()
                        })
                        .set(bevy::winit::WinitPlugin { run_on_any_thread: true, ..default() }),
                )
                .insert_resource(WindowedRx(Mutex::new(rx)))
                .insert_resource(CamCenter(camera_center))
                .add_systems(Startup, windowed_setup_camera)
                .add_systems(Update, windowed_handle_msgs)
                .run();
        });
        (tx, handle)
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // HEADLESS / IMAGE-SAVE MODE
    // ═══════════════════════════════════════════════════════════════════════════
    //
    // Architecture (follows bevy/examples/app/headless_renderer.rs):
    //
    //   Main world  ──Msg::Save──►  HeadlessRx
    //                               receive_headless_msgs rebuilds scene,
    //                               sets HeadlessCapture::pending
    //
    //   Render world, each frame:
    //     ImageCopyDriver (render graph node)
    //       copies rendered texture → GPU buffer
    //     readback_to_channel
    //       maps buffer, sends Vec<u8> → HeadlessBytesTx → HeadlessBytesRx
    //
    //   Main world update:
    //     headless_save_image watches HeadlessBytesRx
    //     after pre_roll frames: strips row-padding, saves PNG, unblocks test

    use bevy::{
        app::ScheduleRunnerPlugin,
        image::TextureFormatPixelInfo,
        render::{
            Extract, Render, RenderApp, RenderSystems,
            render_asset::RenderAssets,
            render_graph::{self, NodeRunError, RenderGraph, RenderGraphContext, RenderLabel},
            render_resource::{
                Buffer, BufferDescriptor, BufferUsages, CommandEncoderDescriptor, Extent3d,
                MapMode, PollType, TexelCopyBufferInfo, TexelCopyBufferLayout, TextureUsages,
            },
            renderer::{RenderContext, RenderDevice, RenderQueue},
            texture::GpuImage,
        },
        window::ExitCondition,
        winit::WinitPlugin,
    };

    // ── Resources ────────────────────────────────────────────────────────────

    #[derive(Resource)]
    struct HeadlessRx(Mutex<mpsc::Receiver<Msg>>);
    impl HeadlessRx {
        fn try_recv(&self) -> Option<Msg> { self.0.lock().unwrap().try_recv().ok() }
    }

    #[derive(Resource)]
    struct HeadlessDir(PathBuf);

    #[derive(Resource, Default)]
    struct HeadlessCapture {
        pending: Option<PendingCapture>,
        pre_roll: u32,
        frame_idx: u32,
        should_quit: bool,
    }

    struct PendingCapture {
        notify: mpsc::SyncSender<()>,
    }

    /// Bytes channel: render world → main world.
    #[derive(Resource)]
    struct HeadlessBytesTx(mpsc::Sender<Vec<u8>>);

    #[derive(Resource)]
    struct HeadlessBytesRx(Mutex<mpsc::Receiver<Vec<u8>>>);
    impl HeadlessBytesRx {
        fn try_recv(&self) -> Option<Vec<u8>> { self.0.lock().unwrap().try_recv().ok() }
    }

    /// Handle to the CPU-side image asset (pixel data is filled from the buffer).
    #[derive(Component)]
    struct CpuImageHandle(Handle<Image>);

    // ── Render-graph node ────────────────────────────────────────────────────

    #[derive(Debug, PartialEq, Eq, Clone, Hash, RenderLabel)]
    struct HeadlessCopyLabel;

    /// Extracted copier state: lives in the render world.
    #[derive(Clone, Default, Resource, Deref, DerefMut)]
    struct ExtractedCopiers(Vec<ImageCopier>);

    #[derive(Clone, Component)]
    struct ImageCopier {
        buffer: Buffer,
        src_image: Handle<Image>,
    }

    fn extract_copiers(mut commands: Commands, copiers: Extract<Query<&ImageCopier>>) {
        commands.insert_resource(ExtractedCopiers(copiers.iter().cloned().collect()));
    }

    struct HeadlessCopyDriver;

    impl render_graph::Node for HeadlessCopyDriver {
        fn run(
            &self,
            _graph: &mut RenderGraphContext,
            render_context: &mut RenderContext,
            world: &World,
        ) -> Result<(), NodeRunError> {
            let copiers = world.resource::<ExtractedCopiers>();
            let gpu_images = world.resource::<RenderAssets<GpuImage>>();

            for copier in copiers.iter() {
                let src = match gpu_images.get(&copier.src_image) {
                    Some(s) => s,
                    None => continue,
                };
                let mut encoder = render_context
                    .render_device()
                    .create_command_encoder(&CommandEncoderDescriptor::default());

                let (bx, _) = src.texture_format.block_dimensions();
                let block_size = src.texture_format.block_copy_size(None).unwrap();
                let padded = RenderDevice::align_copy_bytes_per_row(
                    (src.size.width as usize / bx as usize) * block_size as usize,
                );

                encoder.copy_texture_to_buffer(
                    src.texture.as_image_copy(),
                    TexelCopyBufferInfo {
                        buffer: &copier.buffer,
                        layout: TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(
                                std::num::NonZero::new(padded as u32).unwrap().into(),
                            ),
                            rows_per_image: None,
                        },
                    },
                    src.size,
                );
                world.resource::<RenderQueue>().submit(std::iter::once(encoder.finish()));
            }
            Ok(())
        }
    }

    fn readback_to_channel(
        copiers: Res<ExtractedCopiers>,
        render_device: Res<RenderDevice>,
        tx: Res<HeadlessBytesTx>,
    ) {
        for copier in copiers.iter() {
            let slice = copier.buffer.slice(..);
            let (s, r) = mpsc::channel();
            slice.map_async(MapMode::Read, move |res| {
                s.send(res).ok();
            });
            render_device.poll(PollType::wait_indefinitely()).expect("poll failed");
            r.recv().ok();
            let _ = tx.0.send(slice.get_mapped_range().to_vec());
            copier.buffer.unmap();
        }
    }

    // ── Plugin that wires up the render graph ────────────────────────────────

    struct HeadlessImagePlugin;

    impl Plugin for HeadlessImagePlugin {
        fn build(&self, app: &mut App) {
            let (bytes_tx, bytes_rx) = mpsc::channel::<Vec<u8>>();
            app.insert_resource(HeadlessBytesRx(Mutex::new(bytes_rx)));

            let render_app = app.sub_app_mut(RenderApp);
            {
                let mut graph = render_app.world_mut().resource_mut::<RenderGraph>();
                graph.add_node(HeadlessCopyLabel, HeadlessCopyDriver);
                graph.add_node_edge(bevy::render::graph::CameraDriverLabel, HeadlessCopyLabel);
            }
            render_app
                .insert_resource(HeadlessBytesTx(bytes_tx))
                .add_systems(ExtractSchedule, extract_copiers)
                .add_systems(Render, readback_to_channel.after(RenderSystems::Render));
        }
    }

    // ── Main-world systems ────────────────────────────────────────────────────

    fn headless_setup(
        mut commands: Commands,
        mut images: ResMut<Assets<Image>>,
        cam: Res<CamCenter>,
        render_device: Res<RenderDevice>,
    ) {
        let size = Extent3d { width: 900, height: 500, ..default() };

        let mut render_image = Image::new_target_texture(
            size.width,
            size.height,
            bevy::render::render_resource::TextureFormat::bevy_default(),
            None,
        );
        render_image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
        let render_handle = images.add(render_image);

        let cpu_image = Image::new_target_texture(
            size.width,
            size.height,
            bevy::render::render_resource::TextureFormat::bevy_default(),
            None,
        );
        let cpu_handle = images.add(cpu_image);

        let padded =
            RenderDevice::align_copy_bytes_per_row(size.width as usize * 4) * size.height as usize;
        let buffer = render_device.create_buffer(&BufferDescriptor {
            label: None,
            size: padded as u64,
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        commands.spawn(ImageCopier { buffer, src_image: render_handle.clone() });
        commands.spawn(CpuImageHandle(cpu_handle));

        commands.spawn((
            Camera2d,
            bevy::camera::RenderTarget::Image(render_handle.into()),
            Transform::from_translation(cam.0.extend(0.0)),
        ));
    }

    fn receive_headless_msgs(
        mut capture: ResMut<HeadlessCapture>,
        rx: Res<HeadlessRx>,
        mut commands: Commands,
        frame_entities: Query<Entity, With<FrameEntity>>,
        mut meshes: ResMut<Assets<Mesh>>,
        mut materials: ResMut<Assets<ColorMaterial>>,
    ) {
        if capture.pending.is_some() {
            return; // still rendering previous frame
        }
        match rx.try_recv() {
            Some(Msg::Save { frame, notify }) => {
                despawn_frame(&mut commands, &frame_entities);
                spawn_layers(&mut commands, &mut meshes, &mut materials, frame.layers);
                capture.pending = Some(PendingCapture { notify });
                capture.pre_roll = 8; // let the pipeline warm up
            }
            Some(Msg::Quit) => {
                capture.should_quit = true;
            }
            _ => {}
        }
    }

    fn headless_save_image(
        mut capture: ResMut<HeadlessCapture>,
        bytes_rx: Res<HeadlessBytesRx>,
        dir: Res<HeadlessDir>,
        mut images: ResMut<Assets<Image>>,
        cpu_q: Query<&CpuImageHandle>,
        mut exit: MessageWriter<AppExit>,
    ) {
        if capture.pending.is_some() {
            if capture.pre_roll > 0 {
                // Drain stale bytes from previous frames.
                while bytes_rx.try_recv().is_some() {}
                capture.pre_roll -= 1;
                return;
            }

            if let Some(data) = bytes_rx.try_recv() {
                if let Ok(cpu) = cpu_q.single() {
                    let img = images.get_mut(&cpu.0).unwrap();

                    let row_bytes =
                        img.width() as usize * img.texture_descriptor.format.pixel_size().unwrap();
                    let aligned = RenderDevice::align_copy_bytes_per_row(row_bytes);

                    img.data = Some(if row_bytes == aligned {
                        data.clone()
                    } else {
                        data.chunks(aligned)
                            .take(img.height() as usize)
                            .flat_map(|row| &row[..row_bytes.min(row.len())])
                            .cloned()
                            .collect()
                    });

                    let path = dir.0.join(format!("{:04}.png", capture.frame_idx));
                    img.clone().try_into_dynamic().unwrap().to_rgba8().save(&path).unwrap();
                    capture.frame_idx += 1;

                    capture.pending.take().unwrap().notify.send(()).ok();
                }
            }
        }

        if capture.pending.is_none() && capture.should_quit {
            exit.write(AppExit::Success);
        }
    }

    fn launch_headless(
        dir: PathBuf,
        camera_center: Vec2,
    ) -> (mpsc::Sender<Msg>, std::thread::JoinHandle<()>) {
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            App::new()
                .add_plugins(
                    DefaultPlugins
                        .set(WindowPlugin {
                            primary_window: None,
                            exit_condition: ExitCondition::DontExit,
                            ..default()
                        })
                        .disable::<WinitPlugin>(),
                )
                .add_plugins((
                    ScheduleRunnerPlugin::run_loop(std::time::Duration::from_secs_f64(1.0 / 60.0)),
                    HeadlessImagePlugin,
                ))
                .insert_resource(HeadlessRx(Mutex::new(rx)))
                .insert_resource(HeadlessDir(dir))
                .insert_resource(CamCenter(camera_center))
                .init_resource::<HeadlessCapture>()
                .add_systems(Startup, headless_setup)
                .add_systems(Update, (receive_headless_msgs, headless_save_image).chain())
                .run();
        });
        (tx, handle)
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // PUBLIC API
    // ═══════════════════════════════════════════════════════════════════════════

    enum PreviewMode {
        Live { delay_ms: u64 },
        Save,
    }

    /// A live Bevy window or headless image saver for test geometry visualization.
    ///
    /// | Env var         | Effect                                          |
    /// |-----------------|--------------------------------------------------|
    /// | `PREVIEW_MS=N`  | Opens a window; waits N ms between frames        |
    /// | `IMAGE_DIR=dir` | Headless render; saves `0000.png`, `0001.png`, … |
    ///
    /// The window/renderer closes and its thread is joined on drop.
    pub struct PreviewWindow {
        tx: mpsc::Sender<Msg>,
        handle: Option<std::thread::JoinHandle<()>>,
        mode: PreviewMode,
    }

    impl PreviewWindow {
        /// Open a preview window or headless renderer if the relevant env var is set.
        ///
        /// `camera_center` is the world-space point to center the 900×500 viewport on.
        /// At scale 1.0 (1 world unit = 1 px) that covers ±450 in x and ±250 in y.
        pub fn from_env(title: &str, camera_center: Vec2) -> Option<Self> {
            if let Ok(dir_str) = std::env::var("IMAGE_DIR") {
                let dir = PathBuf::from(dir_str);
                std::fs::create_dir_all(&dir).expect("IMAGE_DIR: could not create directory");
                let (tx, handle) = launch_headless(dir, camera_center);
                Some(Self { tx, handle: Some(handle), mode: PreviewMode::Save })
            } else if let Ok(ms_str) = std::env::var("PREVIEW_MS") {
                let delay_ms = ms_str.parse().ok()?;
                let (tx, handle) = launch_windowed(title.to_owned(), camera_center);
                Some(Self { tx, handle: Some(handle), mode: PreviewMode::Live { delay_ms } })
            } else {
                None
            }
        }

        /// Push one frame.
        ///
        /// - **Live mode**: sends the frame to the window and sleeps `delay_ms` ms.
        /// - **Save mode**: sends the frame, blocks until the PNG has been written to disk.
        pub fn show(&self, frame: Frame) {
            match &self.mode {
                PreviewMode::Live { delay_ms } => {
                    self.tx.send(Msg::Show(frame)).ok();
                    std::thread::sleep(std::time::Duration::from_millis(*delay_ms));
                }
                PreviewMode::Save => {
                    let (notify_tx, notify_rx) = mpsc::sync_channel(0);
                    self.tx.send(Msg::Save { frame, notify: notify_tx }).ok();
                    notify_rx.recv().ok(); // blocks until PNG saved
                }
            }
        }
    }

    impl Drop for PreviewWindow {
        fn drop(&mut self) {
            self.tx.send(Msg::Quit).ok();
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
        }
    }
}
