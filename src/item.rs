// Items, inventory, and pickup animation.
// When the player walks within PICKUP_RADIUS, a `PickingUp` component is added
// to the item entity.  The 0.25s ease-in animation runs on `Real` time so it
// plays at normal speed even during bullet-time.
use avian2d::prelude::{LinearVelocity, Position};
use bevy::{prelude::*, time::Real};

use crate::{
    command_palette::{CommandPaletteState, PaletteCommand, PaletteCommandKind, PaletteRegistry},
    dungeon::terrain::{DungeonState, random_in_playable_area},
    monster::{Monster, Stats},
    player::{ExplorationGoal, Player},
    status_effect::{StatusEffects, random_status_effect},
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PotionColor {
    Red,
    Green,
    Blue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScrollName {
    Readme,
    Agents,
    License,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemKind {
    Potion(PotionColor),
    Scroll(ScrollName),
}

pub const ALL_ITEM_KINDS: &[ItemKind] = &[
    ItemKind::Potion(PotionColor::Red),
    ItemKind::Potion(PotionColor::Green),
    ItemKind::Potion(PotionColor::Blue),
    ItemKind::Scroll(ScrollName::Readme),
    ItemKind::Scroll(ScrollName::Agents),
    ItemKind::Scroll(ScrollName::License),
];

#[derive(Component)]
pub struct Item(pub ItemKind);

#[derive(Component, Default)]
pub struct Inventory(pub Vec<ItemKind>);

#[derive(Component)]
pub struct PickingUp {
    progress: f32,
    origin: Vec2,
    target: Vec2,
    initial_scale: Vec3,
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
                initial_scale: item_transform.scale,
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
        (Entity, &mut Transform, &mut PickingUp, Option<&MeshMaterial2d<ColorMaterial>>, &Item),
        Without<Player>,
    >,
    player_query: Query<&Transform, With<Player>>,
    mut inventory_query: Query<&mut Inventory, With<Player>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut log: ResMut<MessageLog>,
    mut letter_map: ResMut<crate::command_palette::LetterMap>,
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
        transform.scale = picking_up.initial_scale * (1.0 - t * 0.85);

        if let Some(mat_handle) = mat_handle {
            if let Some(mat) = materials.get_mut(mat_handle.0.id()) {
                mat.color = mat.color.with_alpha(1.0 - t);
            }
        }

        if picking_up.progress >= 1.0 {
            log.push(format!("You pick up {}.", item_name(item.0, 1)));
            inventory.0.push(item.0);
            letter_map.get_or_assign_item(item.0);
            commands.entity(entity).despawn();
        }
    }
}

// ── Command palette integration ──────────────────────────────────────────────

pub fn register_item_commands(mut registry: ResMut<PaletteRegistry>) {
    registry.commands.push(PaletteCommand {
        key: "q".to_string(),
        description: "quaff a potion".to_string(),
        icon: None,
        kind: PaletteCommandKind::InventoryTarget {
            item_filter: |i| matches!(i, ItemKind::Potion(_)),
        },
    });
    registry.commands.push(PaletteCommand {
        key: "r".to_string(),
        description: "read a scroll".to_string(),
        icon: None,
        kind: PaletteCommandKind::InventoryTarget {
            item_filter: |i| matches!(i, ItemKind::Scroll(_)),
        },
    });
}

/// Verify `target` passes `filter`, then remove its first occurrence from inventory.
fn find_and_remove_item(
    inventory: &mut Inventory,
    filter: fn(&ItemKind) -> bool,
    target: ItemKind,
) -> Option<ItemKind> {
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
        (
            Entity,
            &mut Inventory,
            &mut Stats,
            &mut Position,
            &mut Transform,
            &mut LinearVelocity,
            &mut StatusEffects,
        ),
        (With<Player>, Without<Monster>),
    >,
    dungeon_state: Res<DungeonState>,
    mut log: ResMut<MessageLog>,
    mut commands: Commands,
    letter_map: Res<crate::command_palette::LetterMap>,
) {
    let Some(cmd) = palette.pending_command.take() else { return };

    let Ok((
        entity,
        mut inventory,
        mut stats,
        mut position,
        mut transform,
        mut velocity,
        mut player_effects,
    )) = player_query.single_mut()
    else {
        return;
    };

    let parts: Vec<&str> = cmd.split_whitespace().collect();
    match parts.as_slice() {
        ["q", letter_str] => {
            let Some(ch) = letter_str.chars().next() else { return };
            let Some(item_kind) = letter_map.item_for_letter(ch) else { return };
            if let Some(item) = find_and_remove_item(
                &mut inventory,
                |i| matches!(i, ItemKind::Potion(_)),
                item_kind,
            ) {
                let gained = 20.0_f32.min(stats.max_hp - stats.hp);
                stats.hp = (stats.hp + 20.0).min(stats.max_hp);
                log.push(format!("You quaff {}. (+{} HP)", item_name(item, 1), gained as i32));
                let mut rng = rand::thread_rng();
                let (kind, duration) = random_status_effect(&mut rng);
                player_effects.add(kind, duration);
            }
        }
        ["r", letter_str] => {
            let Some(ch) = letter_str.chars().next() else { return };
            let Some(item_kind) = letter_map.item_for_letter(ch) else { return };
            if let Some(item) = find_and_remove_item(
                &mut inventory,
                |i| matches!(i, ItemKind::Scroll(_)),
                item_kind,
            ) {
                if let Some(dest) =
                    random_in_playable_area(&dungeon_state.playable_area, &mut rand::thread_rng())
                {
                    position.0 = dest;
                    transform.translation.x = dest.x;
                    transform.translation.y = dest.y;
                    velocity.0 = Vec2::ZERO;
                    commands.entity(entity).remove::<ExplorationGoal>();
                    log.push(format!("You read {}. You are teleported!", item_name(item, 1)));
                }
            }
        }
        _ => {
            palette.pending_command = Some(cmd);
        }
    }
}
