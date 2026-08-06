// This game's sprite vocabulary, built on the engine's generic `rogue_angles::sprite`
// pipeline: which SVG asset + color/text parameterization each `ItemKind` maps to, and an
// `ItemKind`-keyed egui texture cache for the HUD/palette (the engine's own cache is keyed by
// (svg_path, param), which the palette/HUD don't want to reconstruct on every lookup).
use std::collections::HashMap;

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use rogue_angles::sprite::{SpriteEguiData, SpriteParam, SvgSource, SvgSpritePlugin, load_egui_texture};

use crate::item::{ALL_ITEM_KINDS, Item, ItemKind, PotionColor, ScrollName, WandGem};

#[derive(Resource, Default)]
pub struct SpriteEguiTextures {
    textures: HashMap<ItemKind, egui::TextureHandle>,
}

impl SpriteEguiTextures {
    pub fn get(&self, item: ItemKind) -> Option<&egui::TextureHandle> { self.textures.get(&item) }

    fn insert(&mut self, item: ItemKind, handle: egui::TextureHandle) {
        self.textures.entry(item).or_insert(handle);
    }
}

pub fn potion_hex(color: PotionColor) -> String {
    match color {
        PotionColor::Red => "#cc3333".to_string(),
        PotionColor::Green => "#33cc33".to_string(),
        PotionColor::Blue => "#3366ff".to_string(),
        PotionColor::Yellow => "#eebb22".to_string(),
        PotionColor::Purple => "#aa22ff".to_string(),
        PotionColor::Cyan => "#11eeff".to_string(),
        PotionColor::Gray => "#777777".to_string(),
    }
}

pub fn scroll_letter(name: ScrollName) -> &'static str {
    match name {
        ScrollName::Readme => "R",
        ScrollName::Agents => "A",
        ScrollName::License => "L",
        ScrollName::DoNotReadme => "D",
        ScrollName::CodeOfConduct => "C",
        ScrollName::Passwd => "P",
        ScrollName::Hosts => "H",
    }
}

pub fn wand_hex(gem: WandGem) -> String {
    match gem {
        WandGem::Ruby => "#cc2222".to_string(),
        WandGem::Onyx => "#333333".to_string(),
        WandGem::Amber => "#dd8800".to_string(),
        WandGem::Emerald => "#22bb44".to_string(),
    }
}

pub fn sprite_spec(kind: ItemKind) -> (&'static str, SpriteParam) {
    match kind {
        ItemKind::Potion(color) => ("sprites/potion.svg", SpriteParam::Color(potion_hex(color))),
        ItemKind::Scroll(name) => {
            ("sprites/scroll.svg", SpriteParam::Text(scroll_letter(name).to_string()))
        }
        ItemKind::Wand(gem) => ("sprites/wand.svg", SpriteParam::Color(wand_hex(gem))),
    }
}

/// Registers this game's embedded SVG assets with the engine's sprite pipeline. Must run
/// before anything tries to resolve a `svg_path` — a `Startup` system is early enough, since
/// every sprite-loading system runs in `Update`/`EguiPrimaryContextPass`.
pub fn register_svg_assets(mut source: ResMut<SvgSource>) {
    source.register("sprites/potion.svg", include_bytes!("../sprites/potion.svg"));
    source.register("sprites/scroll.svg", include_bytes!("../sprites/scroll.svg"));
    source.register("sprites/wand.svg", include_bytes!("../sprites/wand.svg"));
    source.register("sprites/hatch.svg", include_bytes!("../sprites/hatch.svg"));
    source.register("sprites/wizard.svg", include_bytes!("../sprites/wizard.svg"));
}

pub fn init_sprite_egui_textures(
    mut done: Local<bool>,
    mut contexts: EguiContexts,
    mut cache: ResMut<rogue_angles::sprite::SpriteCache>,
    source: Res<SvgSource>,
    mut sprite_textures: ResMut<SpriteEguiTextures>,
) -> Result {
    if *done {
        return Ok(());
    }

    let ctx = contexts.ctx_mut()?;
    *done = true;

    for &kind in ALL_ITEM_KINDS {
        let (path, param) = sprite_spec(kind);
        if let Some(tex) = load_egui_texture(&source, path, Some(&param), ctx, &mut cache) {
            sprite_textures.insert(kind, tex);
        }
    }

    Ok(())
}

/// Lazily mirrors the engine's per-entity `SpriteEguiData` into this game's `ItemKind`-keyed
/// cache, for any item entity the engine has already loaded a texture for. Idempotent (checked
/// via a `HashMap` entry, not `Added<T>`) rather than chained tightly after
/// `register_egui_sprites` in the same frame, since that system's `SpriteEguiData` insert is
/// deferred through `Commands` and wouldn't reliably be visible to an `Added<T>` filter in a
/// system running later in the same pass.
pub fn sync_item_egui_textures(
    mut sprite_textures: ResMut<SpriteEguiTextures>,
    query: Query<(&SpriteEguiData, &Item)>,
) {
    for (data, item) in &query {
        sprite_textures.insert(item.0, data.texture.clone());
    }
}

pub struct SpritePlugin;

impl Plugin for SpritePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(SvgSpritePlugin)
            .init_resource::<SpriteEguiTextures>()
            .add_systems(Startup, register_svg_assets)
            .add_systems(
                EguiPrimaryContextPass,
                (
                    init_sprite_egui_textures,
                    rogue_angles::sprite::register_egui_sprites,
                    sync_item_egui_textures,
                )
                    .chain(),
            );
    }
}
