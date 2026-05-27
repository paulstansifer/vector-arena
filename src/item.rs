// Items, inventory, and pickup animation.
// When the player walks within PICKUP_RADIUS, a `PickingUp` component is added
// to the item entity.  The 0.25s ease-in animation runs on `Real` time so it
// plays at normal speed even during bullet-time.
use avian2d::prelude::{LinearVelocity, Position};
use bevy::{prelude::*, time::Real};
use geo::{BoundingRect, Contains};
use rand::Rng as _;

use crate::{
    command_palette::{
        CommandPaletteRegistry, CommandPaletteState, PaletteEntry, PaletteProvider, letter_to_idx,
    },
    dungeon::terrain::DungeonState,
    monster::Stats,
    player::{ExplorationGoal, Player},
    ui::MessageLog,
};

// TODO: use the 'strum' crate to get lowercase colors and ALL CAPS scroll titles.
pub fn item_name(item: ItemKind, count: u16) -> String {
    match item {
        ItemKind::Potion(color) if count == 1 => format!("a {color:?} potion"),
        ItemKind::Potion(color) => format!("{count} {color:?} potions"),
        ItemKind::Scroll(name) if count == 1 => format!("a scroll titled '{name:?}'"),
        ItemKind::Scroll(name) => format!("{count} scrolls titled '{name:?}'"),
    }
}

const PICKUP_RADIUS: f32 = 22.0;
const ANIM_SECS: f32 = 0.25;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PotionColor {
    Red,
    Green,
    Blue,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScrollName {
    Readme,
    Agents,
    License,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ItemKind {
    Potion(PotionColor),
    Scroll(ScrollName),
}

#[derive(Component)]
pub struct Item(pub ItemKind);

#[derive(Component, Default)]
pub struct Inventory(pub Vec<ItemKind>);

#[derive(Component)]
pub struct PickingUp {
    progress: f32,
    origin: Vec2,
    target: Vec2,
}

/// When the player walks within range of a ground item, start the pickup animation.
pub fn pickup_items(
    mut commands: Commands,
    player_query: Query<&Transform, With<Player>>,
    items: Query<(Entity, &Transform), (With<Item>, Without<PickingUp>)>,
) {
    let Ok(player_transform) = player_query.single() else {
        return;
    };
    let player_pos = player_transform.translation.truncate();

    for (entity, item_transform) in items.iter() {
        let item_pos = item_transform.translation.truncate();
        if player_pos.distance(item_pos) < PICKUP_RADIUS {
            commands.entity(entity).insert(PickingUp {
                progress: 0.0,
                origin: item_pos,
                target: player_pos,
            });
        }
    }
}

/// Animate items flying toward the player, shrinking and fading, then add them to inventory.
/// Uses real time so the animation runs at full speed even during bullet-time.
pub fn animate_pickup(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut items: Query<
        (Entity, &mut Transform, &mut PickingUp, &MeshMaterial2d<ColorMaterial>, &Item),
        Without<Player>,
    >,
    player_query: Query<&Transform, With<Player>>,
    mut inventory_query: Query<&mut Inventory, With<Player>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut log: ResMut<MessageLog>,
) {
    // Update target each frame so the item tracks the player if they move.
    let player_pos = player_query.single().map(|t| t.translation.truncate()).ok();

    let Ok(mut inventory) = inventory_query.single_mut() else {
        return;
    };

    for (entity, mut transform, mut picking_up, mat_handle, item) in items.iter_mut() {
        picking_up.progress += time.delta_secs() / ANIM_SECS;
        let t = picking_up.progress.min(1.0);

        // Ease-in movement toward player (quadratic: slow start, fast finish)
        let target = player_pos.unwrap_or(picking_up.target);
        let pos = picking_up.origin.lerp(target, t * t);
        transform.translation.x = pos.x;
        transform.translation.y = pos.y;
        transform.scale = Vec3::splat(1.0 - t * 0.85);

        if let Some(mat) = materials.get_mut(mat_handle.0.id()) {
            mat.color = mat.color.with_alpha(1.0 - t);
        }

        if picking_up.progress >= 1.0 {
            log.push(format!("You pick up {}.", item_name(item.0, 1)));
            inventory.0.push(item.0);
            commands.entity(entity).despawn();
        }
    }
}

// ── Command palette integration ──────────────────────────────────────────────

/// Build deduplicated palette entries for `command` from inventory items matching `filter`.
/// Letters are assigned by global first-appearance order across ALL item types (Rogue-style),
/// so potions and scrolls share a single letter namespace per inventory.
fn deduplicated_entries(
    command: &str,
    inventory: &Inventory,
    filter: fn(&ItemKind) -> bool,
) -> Vec<PaletteEntry> {
    // Global unique-type order across all item types (for consistent letter assignment)
    let mut global_order: Vec<ItemKind> = Vec::new();
    for item in &inventory.0 {
        if !global_order.contains(item) {
            global_order.push(*item);
        }
    }

    // Deduped counts for filtered items only
    let mut unique: Vec<(ItemKind, u16)> = Vec::new();
    for item in &inventory.0 {
        if filter(item) {
            if let Some(entry) = unique.iter_mut().find(|(t, _)| t == item) {
                entry.1 += 1;
            } else {
                unique.push((*item, 1));
            }
        }
    }

    unique
        .iter()
        .map(|(item, count)| {
            let global_idx = global_order.iter().position(|t| t == item).unwrap_or(0);
            let letter = (b'a' + global_idx as u8) as char;

            PaletteEntry {
                key: format!("{command} {letter}"),
                description: item_name(*item, *count),
                icon: Some(*item),
                is_complete: true,
            }
        })
        .collect()
}

/// Provides `q` (quaff) and `r` (read) completions.
pub struct ItemCommandProvider;

impl PaletteProvider for ItemCommandProvider {
    fn completions(&self, input: &str, inventory: &Inventory) -> Vec<PaletteEntry> {
        let tokens: Vec<&str> = input.split_whitespace().collect();
        match tokens.as_slice() {
            [] => vec![
                PaletteEntry {
                    key: "q".into(),
                    description: "quaff a potion".into(),
                    icon: None,
                    is_complete: false,
                },
                PaletteEntry {
                    key: "r".into(),
                    description: "read a scroll".into(),
                    icon: None,
                    is_complete: false,
                },
            ],
            // Match both "q" (browsing sub-menu) and "q <letter>" (complete command)
            ["q"] | ["q", _] => {
                deduplicated_entries("q", inventory, |i| matches!(i, ItemKind::Potion(_)))
            }
            ["r"] | ["r", _] => {
                deduplicated_entries("r", inventory, |i| matches!(i, ItemKind::Scroll(_)))
            }
            _ => vec![],
        }
    }
}

pub fn register_item_commands(mut registry: ResMut<CommandPaletteRegistry>) {
    registry.register(Box::new(ItemCommandProvider));
}

/// Look up the item type at `global_idx` in the inventory's global unique-type order,
/// verify it passes `filter`, and remove its first occurrence.
fn find_and_remove_item(
    inventory: &mut Inventory,
    filter: fn(&ItemKind) -> bool,
    global_idx: usize,
) -> Option<ItemKind> {
    let mut global_order: Vec<ItemKind> = Vec::new();
    for item in &inventory.0 {
        if !global_order.contains(item) {
            global_order.push(*item);
        }
    }
    let target = *global_order.get(global_idx)?;
    if !filter(&target) {
        return None;
    }
    let inv_idx = inventory.0.iter().position(|i| *i == target)?;
    Some(inventory.0.remove(inv_idx))
}

/// Consumes `CommandPaletteState::pending_command` and applies item effects.
pub fn execute_item_command(
    mut palette: ResMut<CommandPaletteState>,
    mut player_query: Query<
        (Entity, &mut Inventory, &mut Stats, &mut Position, &mut Transform, &mut LinearVelocity),
        With<Player>,
    >,
    dungeon_state: Res<DungeonState>,
    mut log: ResMut<MessageLog>,
    mut commands: Commands,
) {
    let Some(cmd) = palette.pending_command.take() else { return };

    let Ok((entity, mut inventory, mut stats, mut position, mut transform, mut velocity)) =
        player_query.single_mut()
    else {
        return;
    };

    let parts: Vec<&str> = cmd.split_whitespace().collect();
    match parts.as_slice() {
        ["q", letter_str] => {
            let Some(ch) = letter_str.chars().next() else { return };
            let Some(idx) = letter_to_idx(ch) else { return };
            if let Some(item) =
                find_and_remove_item(&mut inventory, |i| matches!(i, ItemKind::Potion(_)), idx)
            {
                let gained = 20.0_f32.min(stats.max_hp - stats.hp);
                stats.hp = (stats.hp + 20.0).min(stats.max_hp);
                log.push(format!("You quaff {}. (+{} HP)", item_name(item, 1), gained as i32));
            }
        }
        ["r", letter_str] => {
            let Some(ch) = letter_str.chars().next() else { return };
            let Some(idx) = letter_to_idx(ch) else { return };
            if let Some(item) =
                find_and_remove_item(&mut inventory, |i| matches!(i, ItemKind::Scroll(_)), idx)
            {
                if let Some(dest) = random_in_playable_area(&dungeon_state) {
                    position.0 = dest;
                    transform.translation.x = dest.x;
                    transform.translation.y = dest.y;
                    velocity.0 = Vec2::ZERO;
                    commands.entity(entity).remove::<ExplorationGoal>();
                    log.push(format!("You read {}. You are teleported!", item_name(item, 1)));
                }
            }
        }
        _ => {}
    }
}

fn random_in_playable_area(dungeon_state: &DungeonState) -> Option<Vec2> {
    let bbox = dungeon_state.playable_area.bounding_rect()?;
    let mut rng = rand::thread_rng();
    for _ in 0..1000 {
        let x = rng.gen_range(bbox.min().x..bbox.max().x);
        let y = rng.gen_range(bbox.min().y..bbox.max().y);
        if dungeon_state.playable_area.contains(&geo::Point::new(x, y)) {
            return Some(Vec2::new(x, y));
        }
    }
    None
}
