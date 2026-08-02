//! Headless scripted game runner (native only; see `main.rs` for the wasm gate).
//!
//! Usage:  cargo run --bin headless -- 'wait 1s; snap /tmp/out.png'
//!
//! All commands are semicolon-separated in a single argument.
//! Progress is logged to /tmp/va-headless.log.
//!
//! Commands:
//! ```text
//! wait Ns          - advance N seconds at 60 fps
//! snap path        - save a PNG screenshot to path
//! cmd path         - drive the command palette directly (e.g. "cmd g h")
//! click left x y   - set the player's move target to world coords (x, y)
//! level blank      - replace the dungeon with an open 800×500 room
//! ```
use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
    sync::{Mutex, mpsc},
    time::Duration,
};

use bevy::{
    image::TextureFormatPixelInfo,
    prelude::*,
    render::{
        Extract, Render, RenderApp, RenderSystems,
        render_asset::RenderAssets,
        render_graph::{self, NodeRunError, RenderGraph, RenderGraphContext, RenderLabel},
        render_resource::{
            Buffer, BufferDescriptor, BufferUsages, CommandEncoderDescriptor, MapMode, PollType,
            TexelCopyBufferInfo, TexelCopyBufferLayout, TextureUsages,
        },
        renderer::{RenderContext, RenderDevice, RenderQueue},
        texture::GpuImage,
    },
    window::ExitCondition,
    winit::WinitPlugin,
};

use rogue_angles::{
    dungeon::terrain::{
        DungeonCollider, DungeonState, DungeonVisuals, geometry_to_collider, geometry_to_mesh,
    },
    nav::{DungeonNavMesh, playable_area_to_nav_mesh},
    util::safegeo::SafeMultiPolygon,
};
use vector_arena::{
    WORLD_HEIGHT, WORLD_WIDTH,
    effects::hit_particles::HitParticlesPlugin,
    game::{DungeonSeedOverride, GamePlugin},
    player::{MoveTarget, Player},
    ui::{BOTTOM_PANEL_HEIGHT, TOP_PANEL_HEIGHT},
    visuals::torpor_particles::TorporParticlesPlugin,
};

// ── Simple file logger ────────────────────────────────────────────────────────

struct Log(BufWriter<File>);

impl Log {
    fn open() -> Self {
        let f = OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/va-headless.log")
            .expect("open /tmp/va-headless.log");
        Log(BufWriter::new(f))
    }

    fn write(&mut self, msg: &str) {
        writeln!(self.0, "{msg}").ok();
        self.0.flush().ok();
    }
}

// ── Tick helper ───────────────────────────────────────────────────────────────

fn tick(app: &mut App) {
    app.world_mut().resource_mut::<Time<Virtual>>().advance_by(Duration::from_secs_f32(1.0 / 60.0));
    app.update();
}

// ── Snap infrastructure ───────────────────────────────────────────────────────
//
// Architecture mirrors tests/test_lib.rs preview::HeadlessImagePlugin:
//
//   setup_snap_camera  (Startup)
//     spawns Camera2d → render texture, GPU buffer, ImageCopier component
//
//   Render world, each frame:
//     SnapCopyDriver (render graph node) → copies texture to GPU buffer
//     readback_to_channel                → maps buffer, sends bytes via channel
//
//   save_snap() (called from main):
//     drains stale bytes, ticks pipeline, strips row padding, saves PNG

#[derive(Debug, PartialEq, Eq, Clone, Hash, RenderLabel)]
struct SnapCopyLabel;

#[derive(Clone, Component)]
struct ImageCopier {
    buffer: Buffer,
    src_image: Handle<Image>,
}

#[derive(Clone, Default, Resource, Deref, DerefMut)]
struct ExtractedCopiers(Vec<ImageCopier>);

fn extract_copiers(mut commands: Commands, copiers: Extract<Query<&ImageCopier>>) {
    commands.insert_resource(ExtractedCopiers(copiers.iter().cloned().collect()));
}

struct SnapCopyDriver;

impl render_graph::Node for SnapCopyDriver {
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
                        bytes_per_row: Some(std::num::NonZero::new(padded as u32).unwrap().into()),
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

#[derive(Resource)]
struct SnapBytesTx(mpsc::Sender<Vec<u8>>);

#[derive(Resource)]
struct SnapBytesRx(Mutex<mpsc::Receiver<Vec<u8>>>);

impl SnapBytesRx {
    fn try_recv(&self) -> Option<Vec<u8>> { self.0.lock().unwrap().try_recv().ok() }
}

fn readback_to_channel(
    copiers: Res<ExtractedCopiers>,
    render_device: Res<RenderDevice>,
    tx: Res<SnapBytesTx>,
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

#[derive(Component)]
struct SnapCpuImageHandle(Handle<Image>);

struct SnapPlugin;

impl Plugin for SnapPlugin {
    fn build(&self, app: &mut App) {
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        app.insert_resource(SnapBytesRx(Mutex::new(rx)));

        let render_app = app.sub_app_mut(RenderApp);
        {
            let mut graph = render_app.world_mut().resource_mut::<RenderGraph>();
            graph.add_node(SnapCopyLabel, SnapCopyDriver);
            graph.add_node_edge(bevy::render::graph::CameraDriverLabel, SnapCopyLabel);
        }
        render_app
            .insert_resource(SnapBytesTx(tx))
            .add_systems(ExtractSchedule, extract_copiers)
            .add_systems(Render, readback_to_channel.after(RenderSystems::Render));
    }
}

fn setup_snap_camera(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    render_device: Res<RenderDevice>,
) {
    commands.insert_resource(ClearColor(Color::srgb(0.9, 0.9, 0.9)));

    let width = WORLD_WIDTH as u32;
    let height = WORLD_HEIGHT as u32;

    let mut render_image = Image::new_target_texture(
        width,
        height,
        bevy::render::render_resource::TextureFormat::bevy_default(),
        None,
    );
    render_image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let render_handle = images.add(render_image);

    let cpu_image = Image::new_target_texture(
        width,
        height,
        bevy::render::render_resource::TextureFormat::bevy_default(),
        None,
    );
    let cpu_handle = images.add(cpu_image);

    let padded = RenderDevice::align_copy_bytes_per_row(width as usize * 4) * height as usize;
    let buffer = render_device.create_buffer(&BufferDescriptor {
        label: None,
        size: padded as u64,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    commands.spawn(ImageCopier { buffer, src_image: render_handle.clone() });
    commands.spawn(SnapCpuImageHandle(cpu_handle));

    // Mirror the main camera's offset so the game world fills the frame.
    let cam_y = (TOP_PANEL_HEIGHT - BOTTOM_PANEL_HEIGHT) / 2.0;
    commands.spawn((
        Camera2d,
        bevy::camera::RenderTarget::Image(render_handle.into()),
        Transform::from_translation(Vec3::new(0.0, cam_y, 0.0)),
    ));
}

fn save_snap(app: &mut App, log: &mut Log, path: &str) {
    // Drain stale readback bytes from previous frames.
    while app.world().resource::<SnapBytesRx>().try_recv().is_some() {}

    // Tick several frames to let the GPU pipeline produce a fresh frame.
    for _ in 0..6 {
        tick(app);
    }

    // Wait for readback bytes, ticking more frames if needed.
    let mut data = None;
    for _ in 0..8 {
        if let Some(d) = app.world().resource::<SnapBytesRx>().try_recv() {
            data = Some(d);
            break;
        }
        tick(app);
    }

    let data = match data {
        Some(d) => d,
        None => {
            log.write(&format!("snap: GPU readback timed out, skipping {path}"));
            return;
        }
    };

    // Get the CPU image, fill it with the (de-padded) pixel data, then save.
    let mut cpu_q = app.world_mut().query::<&SnapCpuImageHandle>();
    let cpu_handle = cpu_q.single(app.world()).ok().map(|h| h.0.clone());

    let Some(cpu_handle) = cpu_handle else {
        log.write("snap: no SnapCpuImageHandle found");
        return;
    };

    let height = WORLD_HEIGHT as u32;
    let mut images = app.world_mut().resource_mut::<Assets<Image>>();
    let img = images.get_mut(&cpu_handle).unwrap();

    let row_bytes = img.width() as usize * img.texture_descriptor.format.pixel_size().unwrap();
    let aligned = RenderDevice::align_copy_bytes_per_row(row_bytes);

    img.data = Some(if row_bytes == aligned {
        data
    } else {
        data.chunks(aligned)
            .take(height as usize)
            .flat_map(|row| &row[..row_bytes.min(row.len())])
            .cloned()
            .collect()
    });

    match img.clone().try_into_dynamic() {
        Ok(dyn_img) => match dyn_img.to_rgba8().save(path) {
            Ok(()) => log.write(&format!("snap: saved {path}")),
            Err(e) => log.write(&format!("snap: save error: {e}")),
        },
        Err(e) => log.write(&format!("snap: image conversion error: {e:?}")),
    }
}

// ── Command handlers ──────────────────────────────────────────────────────────

fn cmd_wait(app: &mut App, log: &mut Log, arg: &str) {
    let secs: f32 = arg.trim_end_matches('s').trim().parse().unwrap_or_else(|_| {
        log.write(&format!("wait: bad duration '{arg}'"));
        0.0
    });
    let frames = (secs * 60.0).round() as u32;
    for _ in 0..frames {
        tick(app);
    }
    log.write(&format!("wait: {secs}s ({frames} frames)"));
}

fn cmd_cmd(app: &mut App, log: &mut Log, key: &str) {
    let key = key.trim().to_string();
    let ran = rogue_angles::palette::execute_path_string(app.world_mut(), &key);
    tick(app);
    if ran {
        log.write(&format!("cmd: '{key}'"));
    } else {
        log.write(&format!("cmd: '{key}' did not resolve to a runnable command"));
    }
}

fn cmd_click_left(app: &mut App, log: &mut Log, args: &str) {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() < 2 {
        log.write(&format!("click left: need x y, got '{args}'"));
        return;
    }
    let Ok(x) = parts[0].parse::<f32>() else {
        log.write(&format!("click left: bad x '{}'", parts[0]));
        return;
    };
    let Ok(y) = parts[1].parse::<f32>() else {
        log.write(&format!("click left: bad y '{}'", parts[1]));
        return;
    };
    let destination = Vec2::new(x, y);

    let now = app.world().resource::<Time<Virtual>>().elapsed();

    let mut player_q = app.world_mut().query_filtered::<(Entity, &Transform), With<Player>>();
    let player_info = player_q.single(app.world()).ok().map(|(e, t)| (e, t.translation.truncate()));

    let Some((player_entity, origin)) = player_info else {
        log.write("click left: no player found");
        return;
    };

    {
        let world = app.world_mut();
        let mut e = world.entity_mut(player_entity);
        let mut move_target = e.get_mut::<MoveTarget>().unwrap();
        move_target.destination = destination;
        move_target.origin = origin;
        move_target.active = true;
        move_target.time_set = now;
    }
    {
        use bevy_landmass::prelude::AgentTarget2d;
        let world = app.world_mut();
        let mut e = world.entity_mut(player_entity);
        let mut agent_target = e.get_mut::<AgentTarget2d>().unwrap();
        *agent_target = AgentTarget2d::Point(destination);
    }

    log.write(&format!("click left: ({x}, {y})"));
    tick(app);
}

fn cmd_level_blank(app: &mut App, log: &mut Log) {
    use bevy_landmass::prelude::NavMesh2d;
    use geo::{LineString, Polygon, coord};

    let w = WORLD_WIDTH / 2.0;
    let h = WORLD_HEIGHT / 2.0;
    let rw = 400.0f32;
    let rh = 250.0f32;

    // World boundary (counterclockwise) with room as a hole (clockwise).
    let world_ring = LineString::new(vec![
        coord! { x: -w, y: -h },
        coord! { x:  w, y: -h },
        coord! { x:  w, y:  h },
        coord! { x: -w, y:  h },
        coord! { x: -w, y: -h },
    ]);
    let room_ring_hole = LineString::new(vec![
        coord! { x: -rw, y: -rh },
        coord! { x: -rw, y:  rh },
        coord! { x:  rw, y:  rh },
        coord! { x:  rw, y: -rh },
        coord! { x: -rw, y: -rh },
    ]);
    let room_ring_open = LineString::new(vec![
        coord! { x: -rw, y: -rh },
        coord! { x:  rw, y: -rh },
        coord! { x:  rw, y:  rh },
        coord! { x: -rw, y:  rh },
        coord! { x: -rw, y: -rh },
    ]);

    let solid_rock =
        SafeMultiPolygon::from(geo::MultiPolygon::new(vec![Polygon::new(world_ring, vec![
            room_ring_hole,
        ])]));
    let playable_area =
        SafeMultiPolygon::from(geo::MultiPolygon::new(vec![Polygon::new(room_ring_open, vec![])]));

    let terrain_mesh = geometry_to_mesh(&solid_rock);
    let terrain_collider = geometry_to_collider(&solid_rock);
    let nav_mesh = playable_area_to_nav_mesh(&playable_area, &[]);

    let world = app.world_mut();
    let terrain_mesh_handle = world.resource_mut::<Assets<Mesh>>().add(terrain_mesh);

    // Update nav mesh asset in place via the existing handle.
    let nav_handle = world.resource::<DungeonNavMesh>().0.clone();
    world.resource_mut::<Assets<NavMesh2d>>().get_mut(&nav_handle).unwrap().nav_mesh = nav_mesh;

    world.insert_resource(DungeonState {
        solid_rock,
        playable_area,
        glass_walls: SafeMultiPolygon::empty(),
        slow_zones: vec![],
    });
    world.insert_resource(DungeonVisuals(terrain_mesh_handle));
    world.insert_resource(DungeonCollider(terrain_collider));

    // sync_dungeon_to_entities runs on the next tick.
    tick(app);
    log.write("level blank: done");
}

// ── Main ──────────────────────────────────────────────────────────────────────

pub fn run() {
    let mut log = Log::open();

    // Parse: all argv args joined, then split by ';'.
    let script = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let commands: Vec<String> =
        script.split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();

    log.write(&format!("headless: script = {:?}", commands));

    // Build the headless full-game app.
    let mut app = App::new();
    app.add_plugins((
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .set(bevy::log::LogPlugin {
                // Suppress Bevy's INFO spam; only show errors.
                filter: "error".to_string(),
                ..default()
            })
            .disable::<WinitPlugin>()
            // Manual app.update() requires pipelined rendering to be disabled.
            .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>(),
        GamePlugin { headless: true },
        TorporParticlesPlugin,
        HitParticlesPlugin,
        SnapPlugin,
    ))
    .insert_resource(DungeonSeedOverride(42))
    .add_systems(Startup, setup_snap_camera);

    app.finish();

    // Let the game world fully initialize (OnEnter(Restart) → InLevel).
    log.write("headless: starting up...");
    for _ in 0..120 {
        tick(&mut app);
    }
    log.write("headless: startup complete");

    // Execute the script.
    for cmd in &commands {
        let cmd = cmd.trim();
        log.write(&format!("headless: > {cmd}"));

        if let Some(arg) = cmd.strip_prefix("wait ") {
            cmd_wait(&mut app, &mut log, arg);
        } else if let Some(arg) = cmd.strip_prefix("cmd ") {
            cmd_cmd(&mut app, &mut log, arg);
        } else if let Some(arg) = cmd.strip_prefix("click left ") {
            cmd_click_left(&mut app, &mut log, arg);
        } else if cmd == "level blank" {
            cmd_level_blank(&mut app, &mut log);
        } else if let Some(path) = cmd.strip_prefix("snap ") {
            save_snap(&mut app, &mut log, path.trim());
        } else {
            log.write(&format!("headless: unknown command '{cmd}'"));
        }
    }

    log.write("headless: done");
}
