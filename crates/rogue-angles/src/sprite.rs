// SVG-based sprite system: loads an SVG (with optional color/text parameterization) and
// produces both a Bevy mesh (via `bevy_svg`, for world rendering) and an egui texture (via
// `resvg`, for HUD/palette icons), caching by (path, param) so identical sprites share
// handles. The engine owns this pipeline — every game wants *some* way to turn art assets
// into icons — but not the art itself: a game registers its own SVG bytes into `SvgSource`
// (by path, resolved at startup, typically via `include_bytes!`) rather than the engine
// shipping any sprites of its own. `IconId` (in `palette`) is the opaque handle the command
// palette deals in; a game bridges its own item/entity types to `IconId` and to the
// `svg_path`/`SpriteParam` this module actually renders.
use std::collections::HashMap;

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use bevy_mesh::VertexAttributeValues;
use bevy_svg::prelude::{Svg, Svg2d};

pub enum SpriteParam {
    Color(String),
    Text(String),
}

#[derive(Component)]
pub struct SvgSprite {
    pub svg_path: String,
    pub param: Option<SpriteParam>,
}

#[derive(Component)]
pub struct SpriteEguiData {
    pub texture: egui::TextureHandle,
}

/// A game's SVG asset bytes, registered by path (see `SvgSprite::svg_path`) — typically once
/// at startup via `include_bytes!`. The engine has no sprites of its own to ship.
#[derive(Resource, Default)]
pub struct SvgSource(HashMap<String, &'static [u8]>);

impl SvgSource {
    pub fn register(&mut self, path: impl Into<String>, bytes: &'static [u8]) {
        self.0.insert(path.into(), bytes);
    }

    fn get(&self, path: &str) -> Option<&'static [u8]> { self.0.get(path).copied() }
}

#[derive(Resource, Default)]
pub struct SpriteCache {
    svg_handles: HashMap<(String, String), Handle<Svg>>,
    egui_textures: HashMap<(String, String), egui::TextureHandle>,
}

fn param_cache_key(param: Option<&SpriteParam>) -> String {
    match param {
        None => String::new(),
        Some(SpriteParam::Color(hex)) => hex.clone(),
        Some(SpriteParam::Text(s)) => s.clone(),
    }
}

fn parameterize_svg(bytes: &[u8], param: Option<&SpriteParam>) -> Vec<u8> {
    let Some(param) = param else { return bytes.to_vec() };
    let mut s = String::from_utf8_lossy(bytes).into_owned();
    match param {
        SpriteParam::Color(hex) => {
            s = s.replace("#ff00ff", hex).replace("#FF00FF", hex);
        }
        SpriteParam::Text(text) => {
            s = s
                .replace("&amp;", text)
                .replace("font-family:'Arial'", "font-family:'Liberation Sans'");
        }
    }
    s.into_bytes()
}

fn read_svg_bytes(source: &SvgSource, svg_path: &str, param: Option<&SpriteParam>) -> Option<Vec<u8>> {
    let bytes = source.get(svg_path).or_else(|| {
        error!("Unknown SVG sprite: {svg_path} (not registered in SvgSource)");
        None
    })?;
    Some(parameterize_svg(bytes, param))
}

const EMBEDDED_FONT: &[u8] = include_bytes!("../fonts/LiberationSans-Regular.ttf");

// Building the font database scans every system font (and logs a warning per
// unloadable face). It never changes, so build it once and share the Arc across
// all Options, rather than re-scanning on every sprite load.
fn shared_fontdb() -> std::sync::Arc<resvg::usvg::fontdb::Database> {
    use std::sync::{Arc, OnceLock};
    static DB: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = resvg::usvg::fontdb::Database::new();
        db.load_font_data(EMBEDDED_FONT.to_vec());
        #[cfg(not(target_arch = "wasm32"))]
        db.load_system_fonts();
        Arc::new(db)
    })
    .clone()
}

fn make_usvg_options() -> resvg::usvg::Options<'static> {
    let mut options = resvg::usvg::Options::default();
    options.fontdb = shared_fontdb();
    options
}

// Pre-flattens SVG text to paths using usvg. This is needed for bevy_svg's
// WASM path since bevy_svg's from_bytes API doesn't expose the font database.
fn flatten_svg_text(bytes: &[u8]) -> Vec<u8> {
    let options = make_usvg_options();
    match resvg::usvg::Tree::from_data(bytes, &options) {
        Ok(tree) => tree.to_string(&resvg::usvg::WriteOptions::default()).into_bytes(),
        Err(_) => bytes.to_vec(),
    }
}

fn load_svg_handle(
    source: &SvgSource,
    svg_path: &str,
    param: Option<&SpriteParam>,
    meshes: &mut Assets<Mesh>,
    svgs: &mut Assets<Svg>,
    cache: &mut SpriteCache,
) -> Option<Handle<Svg>> {
    let key = (svg_path.to_string(), param_cache_key(param));
    if let Some(h) = cache.svg_handles.get(&key) {
        return Some(h.clone());
    }
    let modified = read_svg_bytes(source, svg_path, param)?;
    let flattened = flatten_svg_text(&modified);
    let mut svg = match Svg::from_bytes(&flattened, svg_path, None::<&std::path::Path>) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to parse SVG {svg_path}: {e}");
            return None;
        }
    };
    let mut mesh = svg.tessellate();
    // After Y-flip in bevy_svg, vertices span (0,0)–(w,−h). Shift so they're centered at
    // the origin, matching the old SVG layout that had translate(-half,−half) on the layer.
    if let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
    {
        let ox = -svg.size.x / 2.0;
        let oy = svg.size.y / 2.0;
        for p in positions.iter_mut() {
            p[0] += ox;
            p[1] += oy;
        }
    }
    svg.mesh = meshes.add(mesh);
    let handle = svgs.add(svg);
    cache.svg_handles.insert(key, handle.clone());
    Some(handle)
}

fn rasterize_svg_centered(
    bytes: &[u8],
    options: &resvg::usvg::Options,
) -> Option<egui::ColorImage> {
    let tree = match resvg::usvg::Tree::from_data(bytes, options) {
        Ok(t) => t,
        Err(e) => {
            error!("Failed to parse SVG for egui texture: {e}");
            return None;
        }
    };
    let size = tree.size();
    let w = size.width() as u32;
    let h = size.height() as u32;
    // Center the content bounding box in the pixmap. SVG sprites are designed
    // with content centered at (0,0) in pixel space (via translate transforms on
    // the layer), so without this offset the center appears at the top-left corner.
    let bbox = tree.root().abs_bounding_box();
    let tx = w as f32 / 2.0 - (bbox.left() + bbox.right()) / 2.0;
    let ty = h as f32 / 2.0 - (bbox.top() + bbox.bottom()) / 2.0;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_translate(tx, ty), &mut pixmap.as_mut());
    Some(egui::ColorImage::from_rgba_premultiplied([w as usize, h as usize], pixmap.data()))
}

/// Loads (or returns the cached) egui texture for `svg_path`/`param`. Exposed publicly so a
/// game can pre-warm textures before any entity carrying the sprite exists (e.g. so palette
/// icons are ready the first frame), in addition to the automatic per-entity loading
/// `register_egui_sprites` does.
pub fn load_egui_texture(
    source: &SvgSource,
    svg_path: &str,
    param: Option<&SpriteParam>,
    ctx: &egui::Context,
    cache: &mut SpriteCache,
) -> Option<egui::TextureHandle> {
    let key = (svg_path.to_string(), param_cache_key(param));
    if let Some(t) = cache.egui_textures.get(&key) {
        return Some(t.clone());
    }
    let modified = read_svg_bytes(source, svg_path, param)?;
    let options = make_usvg_options();
    let color_image = rasterize_svg_centered(&modified, &options)?;
    let tex_name = format!("{}/{}", svg_path, &key.1);
    let tex = ctx.load_texture(tex_name, color_image, Default::default());
    cache.egui_textures.insert(key, tex.clone());
    Some(tex)
}

// Runs in Update (before PostUpdate/Last) so that bevy_svg's add_origin_state and
// apply_origin systems see Changed<Origin> in the same frame.
pub fn insert_svg_components(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut svgs: ResMut<Assets<Svg>>,
    mut cache: ResMut<SpriteCache>,
    source: Res<SvgSource>,
    query: Query<(Entity, &SvgSprite), Added<SvgSprite>>,
) {
    for (entity, svg_sprite) in &query {
        let Some(handle) = load_svg_handle(
            &source,
            &svg_sprite.svg_path,
            svg_sprite.param.as_ref(),
            &mut meshes,
            &mut svgs,
            &mut cache,
        ) else {
            continue;
        };
        commands.entity(entity).insert(Svg2d(handle));
    }
}

// Registers egui textures for entities that have SvgSprite but not yet SpriteEguiData.
// Runs in EguiPrimaryContextPass to access the egui Context.
pub fn register_egui_sprites(
    mut commands: Commands,
    mut contexts: EguiContexts,
    mut cache: ResMut<SpriteCache>,
    source: Res<SvgSource>,
    query: Query<(Entity, &SvgSprite), Without<SpriteEguiData>>,
) -> Result {
    let ctx = contexts.ctx_mut()?;

    for (entity, svg_sprite) in &query {
        let Some(tex) =
            load_egui_texture(&source, &svg_sprite.svg_path, svg_sprite.param.as_ref(), ctx, &mut cache)
        else {
            continue;
        };
        commands.entity(entity).insert(SpriteEguiData { texture: tex });
    }

    Ok(())
}

/// World-mesh half of the sprite pipeline: `bevy_svg`'s own plugin, the sprite cache/asset
/// registry, and the system that turns a newly-added `SvgSprite` into a rendered `Svg2d`.
/// Safe to add even without an egui context (e.g. headless testing) — the egui-texture half
/// (`register_egui_sprites`) is a separate system a game adds to its own
/// `EguiPrimaryContextPass` wiring, since it only makes sense where egui is actually running.
pub struct SvgSpritePlugin;

impl Plugin for SvgSpritePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(bevy_svg::prelude::SvgPlugin)
            .init_resource::<SpriteCache>()
            .init_resource::<SvgSource>()
            .add_systems(Update, insert_svg_components);
    }
}
