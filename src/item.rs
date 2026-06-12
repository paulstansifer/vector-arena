// Items, inventory, and pickup animation.
// When the player walks within PICKUP_RADIUS, a `PickingUp` component is added
// to the item entity.  The 0.25s ease-in animation runs on `Real` time so it
// plays at normal speed even during bullet-time.
use std::collections::{HashMap, HashSet};

use avian2d::prelude::{LinearVelocity, Position};
use bevy::{prelude::*, time::Real};
use rand::{Rng, seq::SliceRandom};

use crate::{
    command_palette::{CommandPaletteState, PaletteCommand, PaletteCommandKind, PaletteRegistry},
    dungeon::terrain::{DungeonState, random_in_playable_area},
    effects::scroll::{
        AcquirementEvent, MagicMappingEvent, MonsterConfusionEvent, SummonMonsterEvent,
    },
    monster::{Monster, Stats},
    player::{ExplorationGoal, Player},
    status_effect::{StatusEffect, StatusEffects},
    ui::{MessageLog, WorldTooltip},
};

// TODO: use the 'strum' crate to get lowercase colors and ALL CAPS scroll titles.

/// The "appearance" name: what the player sees before an item has been identified.
pub fn item_name(item: ItemKind, count: u16) -> String {
    match item {
        ItemKind::Potion(color) if count == 1 => format!("a {color:?} potion"),
        ItemKind::Potion(color) => format!("{count} {color:?} potions"),
        ItemKind::Scroll(name) if count == 1 => format!("a scroll titled '{name:?}'"),
        ItemKind::Scroll(name) => format!("{count} scrolls titled '{name:?}'"),
    }
}

/// The displayed name: once identified, describes the effect ("a Potion of Blindness");
/// otherwise falls back to the appearance name from `item_name`.
pub fn item_display_name(item: ItemKind, count: u16, identities: &ItemIdentities) -> String {
    if !identities.is_identified(item) {
        return item_name(item, count);
    }
    match item {
        ItemKind::Potion(color) => {
            let e = identities.potion_effect(color).name();
            if count == 1 { format!("a Potion of {e}") } else { format!("{count} Potions of {e}") }
        }
        ItemKind::Scroll(name) => {
            let e = identities.scroll_effect(name).name();
            if count == 1 { format!("a Scroll of {e}") } else { format!("{count} Scrolls of {e}") }
        }
    }
}

const PICKUP_RADIUS: f32 = 22.0;
const ANIM_SECS: f32 = 0.25;

// Potion colors and scroll titles are *appearances*: their pairing with an effect is
// randomized each new game (see `ItemIdentities`). There is one appearance per effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PotionColor {
    Red,
    Green,
    Blue,
    Yellow,
    Purple,
    Cyan,
    Gray,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScrollName {
    Readme,
    Agents,
    License,
    DoNotReadme,
    CodeOfConduct,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemKind {
    Potion(PotionColor),
    Scroll(ScrollName),
}

const ALL_POTION_COLORS: &[PotionColor] = &[
    PotionColor::Red,
    PotionColor::Green,
    PotionColor::Blue,
    PotionColor::Yellow,
    PotionColor::Purple,
    PotionColor::Cyan,
    PotionColor::Gray,
];

const ALL_SCROLL_NAMES: &[ScrollName] = &[
    ScrollName::Readme,
    ScrollName::Agents,
    ScrollName::License,
    ScrollName::DoNotReadme,
    ScrollName::CodeOfConduct,
];

pub const ALL_ITEM_KINDS: &[ItemKind] = &[
    ItemKind::Potion(PotionColor::Red),
    ItemKind::Potion(PotionColor::Green),
    ItemKind::Potion(PotionColor::Blue),
    ItemKind::Potion(PotionColor::Yellow),
    ItemKind::Potion(PotionColor::Purple),
    ItemKind::Potion(PotionColor::Cyan),
    ItemKind::Potion(PotionColor::Gray),
    ItemKind::Scroll(ScrollName::Readme),
    ItemKind::Scroll(ScrollName::Agents),
    ItemKind::Scroll(ScrollName::License),
    ItemKind::Scroll(ScrollName::DoNotReadme),
    ItemKind::Scroll(ScrollName::CodeOfConduct),
];

/// The hidden effect a potion appearance maps to this game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PotionEffect {
    Confusion,
    Blindness,
    Haste,
    Slowness,
    Boosting,
    Draining,
    Displacement,
}

const ALL_POTION_EFFECTS: &[PotionEffect] = &[
    PotionEffect::Confusion,
    PotionEffect::Blindness,
    PotionEffect::Haste,
    PotionEffect::Slowness,
    PotionEffect::Boosting,
    PotionEffect::Draining,
    PotionEffect::Displacement,
];

impl PotionEffect {
    pub fn name(self) -> &'static str {
        match self {
            PotionEffect::Confusion => "Confusion",
            PotionEffect::Blindness => "Blindness",
            PotionEffect::Haste => "Haste",
            PotionEffect::Slowness => "Slowness",
            PotionEffect::Boosting => "Boosting",
            PotionEffect::Draining => "Draining",
            PotionEffect::Displacement => "Displacement",
        }
    }
}

/// The hidden effect a scroll appearance maps to this game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScrollEffect {
    Teleport,
    SummonMonster,
    MagicMapping,
    MonsterConfusion,
    Acquirement,
}

const ALL_SCROLL_EFFECTS: &[ScrollEffect] = &[
    ScrollEffect::Teleport,
    ScrollEffect::SummonMonster,
    ScrollEffect::MagicMapping,
    ScrollEffect::MonsterConfusion,
    ScrollEffect::Acquirement,
];

impl ScrollEffect {
    pub fn name(self) -> &'static str {
        match self {
            ScrollEffect::Teleport => "Teleportation",
            ScrollEffect::SummonMonster => "Monster Summoning",
            ScrollEffect::MagicMapping => "Magic Mapping",
            ScrollEffect::MonsterConfusion => "Monster Confusion",
            ScrollEffect::Acquirement => "Acquirement",
        }
    }
}

/// Per-game roguelike identification state: which appearance maps to which effect (randomized
/// at the start of every new game) and which item kinds the player has identified by using them.
#[derive(Resource, Default)]
pub struct ItemIdentities {
    potion: HashMap<PotionColor, PotionEffect>,
    scroll: HashMap<ScrollName, ScrollEffect>,
    identified: HashSet<ItemKind>,
}

impl ItemIdentities {
    /// Reshuffle the appearance→effect pairings and forget all identifications. Call once per
    /// new game (not on descent).
    pub fn randomize(&mut self, rng: &mut impl Rng) {
        self.identified.clear();

        let mut potion_effects = ALL_POTION_EFFECTS.to_vec();
        potion_effects.shuffle(rng);
        self.potion = ALL_POTION_COLORS.iter().copied().zip(potion_effects).collect();

        let mut scroll_effects = ALL_SCROLL_EFFECTS.to_vec();
        scroll_effects.shuffle(rng);
        self.scroll = ALL_SCROLL_NAMES.iter().copied().zip(scroll_effects).collect();
    }

    pub fn potion_effect(&self, color: PotionColor) -> PotionEffect {
        self.potion.get(&color).copied().unwrap_or(PotionEffect::Confusion)
    }

    pub fn scroll_effect(&self, name: ScrollName) -> ScrollEffect {
        self.scroll.get(&name).copied().unwrap_or(ScrollEffect::Teleport)
    }

    pub fn is_identified(&self, item: ItemKind) -> bool { self.identified.contains(&item) }

    pub fn identify(&mut self, item: ItemKind) { self.identified.insert(item); }
}

/// Maps an identified potion effect to a concrete status effect + duration applied on quaff.
fn potion_status_effect(effect: PotionEffect, rng: &mut impl Rng) -> (StatusEffect, f32) {
    let duration = rng.gen_range(4.0..8.0);
    let kind = match effect {
        PotionEffect::Confusion => {
            let a = rng.gen_range(0.0..std::f32::consts::TAU);
            StatusEffect::Confused { wander_dir: Vec2::new(a.cos(), a.sin()) }
        }
        PotionEffect::Blindness => StatusEffect::Blind,
        PotionEffect::Haste => StatusEffect::SpeedMod(2.0),
        PotionEffect::Slowness => StatusEffect::SpeedMod(0.5),
        PotionEffect::Boosting => StatusEffect::MissileMod(2.0),
        PotionEffect::Draining => StatusEffect::MissileMod(0.5),
        PotionEffect::Displacement => StatusEffect::Displacing,
    };
    (kind, duration)
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
    identities: Res<ItemIdentities>,
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

        if let Some(mat_handle) = mat_handle
            && let Some(mat) = materials.get_mut(mat_handle.0.id())
        {
            mat.color = mat.color.with_alpha(1.0 - t);
        }

        if picking_up.progress >= 1.0 {
            log.push(format!("You pick up {}.", item_display_name(item.0, 1, &identities)));
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
    mut identities: ResMut<ItemIdentities>,
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
                let ItemKind::Potion(color) = item else { return };
                let gained = 20.0_f32.min(stats.max_hp - stats.hp);
                stats.hp = (stats.hp + 20.0).min(stats.max_hp);
                let mut rng = rand::thread_rng();
                let (kind, duration) =
                    potion_status_effect(identities.potion_effect(color), &mut rng);
                player_effects.add(kind, duration);
                identities.identify(item);
                log.push(format!(
                    "You quaff {}. (+{} HP)",
                    item_display_name(item, 1, &identities),
                    gained as i32
                ));
            }
        }
        ["r", letter_str] => {
            let Some(ch) = letter_str.chars().next() else { return };
            let Some(item_kind) = letter_map.item_for_letter(ch) else { return };
            let Some(item) = find_and_remove_item(
                &mut inventory,
                |i| matches!(i, ItemKind::Scroll(_)),
                item_kind,
            ) else {
                return;
            };
            let ItemKind::Scroll(name) = item else { return };
            let effect = identities.scroll_effect(name);
            identities.identify(item);

            // Every scroll restores 20 MP in addition to its identified effect.
            let mana_gained = 20.0_f32.min(stats.max_mana - stats.mana);
            stats.mana = (stats.mana + 20.0).min(stats.max_mana);
            let player_pos = transform.translation.truncate();
            log.push(format!(
                "You read {}. (+{} MP)",
                item_display_name(item, 1, &identities),
                mana_gained as i32
            ));

            match effect {
                ScrollEffect::Teleport => {
                    if let Some(dest) = random_in_playable_area(
                        &dungeon_state.playable_area,
                        &mut rand::thread_rng(),
                    ) {
                        position.0 = dest;
                        transform.translation.x = dest.x;
                        transform.translation.y = dest.y;
                        velocity.0 = Vec2::ZERO;
                        commands.entity(entity).remove::<ExplorationGoal>();
                        log.push("You are teleported!");
                    }
                }
                ScrollEffect::SummonMonster => {
                    commands.trigger(SummonMonsterEvent { origin: player_pos });
                }
                ScrollEffect::MagicMapping => {
                    commands.trigger(MagicMappingEvent);
                }
                ScrollEffect::MonsterConfusion => {
                    commands.trigger(MonsterConfusionEvent { origin: player_pos });
                }
                ScrollEffect::Acquirement => {
                    commands.trigger(AcquirementEvent { origin: player_pos });
                }
            }
        }
        _ => {
            palette.pending_command = Some(cmd);
        }
    }
}

/// Keeps ground-item tooltips in sync with the current identification state, so an item dropped
/// before its kind was identified shows its revealed name once the player learns it.
pub fn refresh_item_tooltips(
    identities: Res<ItemIdentities>,
    mut query: Query<(&Item, &mut WorldTooltip)>,
) {
    for (item, mut tooltip) in query.iter_mut() {
        let name = item_display_name(item.0, 1, &identities);
        if tooltip.0 != name {
            tooltip.0 = name;
        }
    }
}
