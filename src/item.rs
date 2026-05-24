// Items, inventory, and pickup animation.
// When the player walks within PICKUP_RADIUS, a `PickingUp` component is added
// to the item entity.  The 0.25s ease-in animation runs on `Real` time so it
// plays at normal speed even during bullet-time.
use avian2d::prelude::{LinearVelocity, Position};
use bevy::{prelude::*, time::Real};
use bevy_landmass::prelude::AgentTarget2d;
use geo::{BoundingRect, Contains};
use rand::Rng as _;

use crate::{
    dungeon::terrain::DungeonState,
    monster::Stats,
    player::{ExplorationGoal, MoveTarget, Player},
    ui::MessageLog,
};

fn item_name(item: ItemKind) -> &'static str {
    match item {
        ItemKind::Potion(PotionColor::Red) => "a red potion",
        ItemKind::Potion(PotionColor::Green) => "a green potion",
        ItemKind::Potion(PotionColor::Blue) => "a blue potion",
        ItemKind::Scroll(ScrollName::Readme) => "a scroll labeled README",
        ItemKind::Scroll(ScrollName::Agents) => "a scroll labeled AGENTS.md",
        ItemKind::Scroll(ScrollName::License) => "a scroll labeled LICENSE",
    }
}

pub fn item_display_name(item: ItemKind) -> &'static str {
    match item {
        ItemKind::Potion(PotionColor::Red) => "Red Potion",
        ItemKind::Potion(PotionColor::Green) => "Green Potion",
        ItemKind::Potion(PotionColor::Blue) => "Blue Potion",
        ItemKind::Scroll(ScrollName::Readme) => "Scroll titled 'README'",
        ItemKind::Scroll(ScrollName::Agents) => "Scroll titled 'AGENTS'",
        ItemKind::Scroll(ScrollName::License) => "Scroll titled 'LICENSE'",
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
            log.push(format!("You pick up {}.", item_name(item.0)));
            inventory.0.push(item.0);
            commands.entity(entity).despawn();
        }
    }
}

// ── Item use dialog ──────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ItemDialogKind {
    Potions,
    Scrolls,
}

/// Tracks whether the quaff/read selection window is open, and any item the
/// player clicked in the UI that should be applied on the next Update tick.
#[derive(Resource, Default)]
pub struct ItemUseDialog {
    pub open: Option<ItemDialogKind>,
    /// Set by the UI when a button is clicked; consumed by `apply_item_use`.
    pub pending_use: Option<ItemKind>,
}

/// Opens the potion or scroll dialog when the player presses Q or R,
/// and immediately halts their movement.
pub fn open_item_dialog(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut dialog: ResMut<ItemUseDialog>,
    mut player_query: Query<
        (Entity, &mut MoveTarget, &mut AgentTarget2d, &mut LinearVelocity),
        With<Player>,
    >,
    mut commands: Commands,
) {
    if dialog.open.is_some() {
        return;
    }

    let kind = if keyboard.just_pressed(KeyCode::KeyQ) {
        Some(ItemDialogKind::Potions)
    } else if keyboard.just_pressed(KeyCode::KeyR) {
        Some(ItemDialogKind::Scrolls)
    } else {
        None
    };

    let Some(kind) = kind else { return };
    dialog.open = Some(kind);

    if let Ok((entity, mut move_target, mut agent_target, mut velocity)) = player_query.single_mut()
    {
        move_target.active = false;
        *agent_target = AgentTarget2d::None;
        velocity.0 = Vec2::ZERO;
        commands.entity(entity).remove::<ExplorationGoal>();
    }
}

/// TODO: Break this up so that items can be defined independently!
/// Consumes `ItemUseDialog::pending_use` and applies the item effect.
/// Potions restore 20 HP; scrolls teleport to a random position inside the
/// playable area (rejection-sampled from its bounding box).
pub fn apply_item_use(
    mut dialog: ResMut<ItemUseDialog>,
    mut player_query: Query<
        (Entity, &mut Inventory, &mut Stats, &mut Position, &mut Transform, &mut LinearVelocity),
        With<Player>,
    >,
    dungeon_state: Res<DungeonState>,
    mut log: ResMut<MessageLog>,
    mut commands: Commands,
) {
    let Some(item) = dialog.pending_use.take() else { return };

    let Ok((entity, mut inventory, mut stats, mut position, mut transform, mut velocity)) =
        player_query.single_mut()
    else {
        return;
    };

    let Some(idx) = inventory.0.iter().position(|&i| i == item) else {
        return;
    };
    inventory.0.remove(idx);

    match item {
        ItemKind::Potion(_) => {
            let gained = 20.0_f32.min(stats.max_hp - stats.hp);
            stats.hp = (stats.hp + 20.0).min(stats.max_hp);
            log.push(format!("You quaff {}. (+{} HP)", item_name(item), gained as i32));
        }
        ItemKind::Scroll(_) => {
            if let Some(dest) = random_in_playable_area(&dungeon_state) {
                position.0 = dest;
                transform.translation.x = dest.x;
                transform.translation.y = dest.y;
                velocity.0 = Vec2::ZERO;
                commands.entity(entity).remove::<ExplorationGoal>();
                log.push(format!("You read {}. You are teleported!", item_name(item)));
            }
        }
    }

    dialog.open = None;
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
