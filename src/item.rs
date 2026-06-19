// Items, inventory, and pickup animation.
// When the player walks within PICKUP_RADIUS, a `PickingUp` component is added
// to the item entity.  The 0.25s ease-in animation runs on `Real` time so it
// plays at normal speed even during bullet-time.
use std::collections::{HashMap, HashSet};

use avian2d::prelude::{Collider, LinearVelocity, Position, SpatialQuery, SpatialQueryFilter};
use bevy::{prelude::*, time::Real};
use geo::{Intersects, Line as GeoLine};
use rand::{Rng, seq::SliceRandom};

use crate::{
    GameLayer,
    command_palette::{CommandPaletteState, PaletteCommand, PaletteCommandKind, PaletteRegistry},
    dungeon::terrain::{DungeonState, random_in_playable_area},
    effects::scroll::{
        AcquirementEvent, InstabilityEvent, MagicMappingEvent, MonsterConfusionEvent,
        SummonMonsterEvent,
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
        ItemKind::Wand(gem) if count == 1 => format!("{} wand", gem.article_name()),
        ItemKind::Wand(gem) => format!("{count} {} wands", gem.name()),
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
        ItemKind::Wand(gem) => {
            let e = identities.wand_effect(gem).name();
            if count == 1 { format!("a Wand of {e}") } else { format!("{count} Wands of {e}") }
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
    Passwd,
    Hosts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WandGem {
    Ruby,
    Onyx,
    Amber,
    Emerald,
}

impl WandGem {
    pub fn name(self) -> &'static str {
        match self {
            WandGem::Ruby => "ruby",
            WandGem::Onyx => "onyx",
            WandGem::Amber => "amber",
            WandGem::Emerald => "emerald",
        }
    }

    pub fn article_name(self) -> &'static str {
        match self {
            WandGem::Ruby => "a ruby",
            WandGem::Onyx => "an onyx",
            WandGem::Amber => "an amber",
            WandGem::Emerald => "an emerald",
        }
    }
}

const ALL_WAND_GEMS: &[WandGem] = &[WandGem::Ruby, WandGem::Onyx, WandGem::Amber, WandGem::Emerald];

/// The hidden effect a wand gem maps to this game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WandEffect {
    Swapping,
    Confusion,
    Crumbling,
    Attraction,
}

const ALL_WAND_EFFECTS: &[WandEffect] =
    &[WandEffect::Swapping, WandEffect::Confusion, WandEffect::Crumbling, WandEffect::Attraction];

impl WandEffect {
    pub fn name(self) -> &'static str {
        match self {
            WandEffect::Swapping => "Swapping",
            WandEffect::Confusion => "Confusion",
            WandEffect::Crumbling => "Crumbling",
            WandEffect::Attraction => "Attraction",
        }
    }
}

#[derive(Event)]
pub struct WandCrumblingEvent {
    pub target: Vec2,
}

#[derive(Event)]
pub struct WandAttractionEvent {
    pub target: Vec2,
    pub player_pos: Vec2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemKind {
    Potion(PotionColor),
    Scroll(ScrollName),
    Wand(WandGem),
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
    ScrollName::Passwd,
    ScrollName::Hosts,
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
    ItemKind::Scroll(ScrollName::Passwd),
    ItemKind::Scroll(ScrollName::Hosts),
    ItemKind::Wand(WandGem::Ruby),
    ItemKind::Wand(WandGem::Onyx),
    ItemKind::Wand(WandGem::Amber),
    ItemKind::Wand(WandGem::Emerald),
];

/// Returns a random item kind with weighted distribution: 40% potions, 40% scrolls, 20% wands.
pub fn random_item_kind(rng: &mut impl Rng) -> ItemKind {
    let roll = rng.gen_range(0..10u32);
    if roll < 4 {
        ItemKind::Potion(*ALL_POTION_COLORS.choose(rng).unwrap())
    } else if roll < 8 {
        ItemKind::Scroll(*ALL_SCROLL_NAMES.choose(rng).unwrap())
    } else {
        ItemKind::Wand(*ALL_WAND_GEMS.choose(rng).unwrap())
    }
}

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
    Instability,
    Acquirement,
    Identify,
    Forgetting,
}

const ALL_SCROLL_EFFECTS: &[ScrollEffect] = &[
    ScrollEffect::Teleport,
    ScrollEffect::SummonMonster,
    ScrollEffect::MagicMapping,
    ScrollEffect::Instability,
    ScrollEffect::Acquirement,
    ScrollEffect::Identify,
    ScrollEffect::Forgetting,
];

impl ScrollEffect {
    pub fn name(self) -> &'static str {
        match self {
            ScrollEffect::Teleport => "Teleportation",
            ScrollEffect::SummonMonster => "Monster Summoning",
            ScrollEffect::MagicMapping => "Magic Mapping",
            ScrollEffect::Instability => "Instability",
            ScrollEffect::Acquirement => "Acquirement",
            ScrollEffect::Identify => "Identify",
            ScrollEffect::Forgetting => "Forgetting",
        }
    }
}

/// Per-game roguelike identification state: which appearance maps to which effect (randomized
/// at the start of every new game) and which item kinds the player has identified by using them.
#[derive(Resource, Default)]
pub struct ItemIdentities {
    potion: HashMap<PotionColor, PotionEffect>,
    scroll: HashMap<ScrollName, ScrollEffect>,
    wand: HashMap<WandGem, WandEffect>,
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

        let mut wand_effects = ALL_WAND_EFFECTS.to_vec();
        wand_effects.shuffle(rng);
        self.wand = ALL_WAND_GEMS.iter().copied().zip(wand_effects).collect();
    }

    pub fn potion_effect(&self, color: PotionColor) -> PotionEffect {
        self.potion.get(&color).copied().unwrap_or(PotionEffect::Confusion)
    }

    pub fn scroll_effect(&self, name: ScrollName) -> ScrollEffect {
        self.scroll.get(&name).copied().unwrap_or(ScrollEffect::Teleport)
    }

    pub fn wand_effect(&self, gem: WandGem) -> WandEffect {
        self.wand.get(&gem).copied().unwrap_or(WandEffect::Swapping)
    }

    pub fn is_identified(&self, item: ItemKind) -> bool { self.identified.contains(&item) }

    pub fn identify(&mut self, item: ItemKind) { self.identified.insert(item); }

    /// Implements a Scroll of Forgetting: pick an item type (potions or scrolls) that has at least
    /// two identified appearances, choose up to three of them, forget their identifications, and
    /// permute (derange) the chosen appearances' hidden effects among one another. Returns the
    /// number of forgotten appearances and the type word for the message, or `None` if neither
    /// type had enough known appearances to scramble.
    pub fn forget(&mut self, rng: &mut impl Rng) -> Option<(usize, &'static str)> {
        let known_potions: Vec<PotionColor> = ALL_POTION_COLORS
            .iter()
            .copied()
            .filter(|c| self.identified.contains(&ItemKind::Potion(*c)))
            .collect();
        let known_scrolls: Vec<ScrollName> = ALL_SCROLL_NAMES
            .iter()
            .copied()
            .filter(|n| self.identified.contains(&ItemKind::Scroll(*n)))
            .collect();

        // Each type is a candidate only if it has at least two known appearances to permute.
        let mut candidates: Vec<bool> = Vec::new(); // true = potions, false = scrolls
        if known_potions.len() >= 2 {
            candidates.push(true);
        }
        if known_scrolls.len() >= 2 {
            candidates.push(false);
        }
        let &do_potions = candidates.choose(rng)?;

        if do_potions {
            let chosen: Vec<PotionColor> = known_potions.choose_multiple(rng, 3).copied().collect();
            let effects: Vec<PotionEffect> = chosen.iter().map(|c| self.potion[c]).collect();
            let perm = derange_indices(chosen.len(), rng);
            for (i, &c) in chosen.iter().enumerate() {
                self.potion.insert(c, effects[perm[i]]);
                self.identified.remove(&ItemKind::Potion(c));
            }
            Some((chosen.len(), "potion"))
        } else {
            let chosen: Vec<ScrollName> = known_scrolls.choose_multiple(rng, 3).copied().collect();
            let effects: Vec<ScrollEffect> = chosen.iter().map(|n| self.scroll[n]).collect();
            let perm = derange_indices(chosen.len(), rng);
            for (i, &n) in chosen.iter().enumerate() {
                self.scroll.insert(n, effects[perm[i]]);
                self.identified.remove(&ItemKind::Scroll(n));
            }
            Some((chosen.len(), "scroll"))
        }
    }
}

/// Produces a derangement of `0..n`: a permutation where no index maps to itself (for n >= 2), so
/// every chosen appearance ends up with a *different* effect than it started with. The effects only
/// move among the chosen appearances, keeping the global appearance→effect bijection intact.
fn derange_indices(n: usize, rng: &mut impl Rng) -> Vec<usize> {
    let mut perm: Vec<usize> = (0..n).collect();
    if n < 2 {
        return perm;
    }
    loop {
        perm.shuffle(rng);
        if perm.iter().enumerate().all(|(i, &j)| i != j) {
            return perm;
        }
    }
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
            requires_target: false,
        },
    });
    registry.commands.push(PaletteCommand {
        key: "r".to_string(),
        description: "read a scroll".to_string(),
        icon: None,
        kind: PaletteCommandKind::InventoryTarget {
            item_filter: |i| matches!(i, ItemKind::Scroll(_)),
            requires_target: false,
        },
    });
    registry.commands.push(PaletteCommand {
        key: "w".to_string(),
        description: "wave a wand".to_string(),
        icon: None,
        kind: PaletteCommandKind::InventoryTarget {
            item_filter: |i| matches!(i, ItemKind::Wand(_)),
            requires_target: true,
        },
    });
}

const WAND_MANA_COST: f32 = 5.0;

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
    mut monster_query: Query<
        (&mut Position, &mut Transform, &mut LinearVelocity, &mut StatusEffects),
        (With<Monster>, Without<Player>),
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

    let mut wand_step2 = false;
    let mut put_back = false;
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
                stats.hp = (stats.hp + 15.0).min(stats.max_hp);
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
                ScrollEffect::Instability => {
                    commands.trigger(InstabilityEvent { origin: player_pos });
                }
                ScrollEffect::Acquirement => {
                    commands.trigger(AcquirementEvent { origin: player_pos });
                }
                ScrollEffect::Identify => {
                    // Defer to a follow-up palette prompt so the player can choose which of their
                    // unidentified items to identify.
                    if inventory.0.iter().any(|i| !identities.is_identified(*i)) {
                        palette.open = true;
                        palette.identify_mode = true;
                        palette.input = "identify ".to_string();
                        palette.selected_idx = 0;
                    } else {
                        log.push("...but you have nothing left to identify.");
                    }
                }
                ScrollEffect::Forgetting => match identities.forget(&mut rand::thread_rng()) {
                    Some((_n, type_word)) => {
                        log.push(format!(
                            "You're not as sure as you used to be about those {type_word}s."
                        ));
                    }
                    None => {
                        log.push("You don't remember having forgotten anything.");
                    }
                },
            }
        }
        ["identify", letter_str] => {
            let Some(ch) = letter_str.chars().next() else { return };
            let Some(item_kind) = letter_map.item_for_letter(ch) else { return };
            let count = inventory.0.iter().filter(|i| **i == item_kind).count() as u16;
            if count > 0 && !identities.is_identified(item_kind) {
                identities.identify(item_kind);
                log.push(format!("It is {}.", item_display_name(item_kind, count, &identities)));
            }
        }
        ["w", _] if palette.pending_target.is_some() => {
            // Step 2: world-click target — pending_target set by the click handler.
            wand_step2 = true;
        }
        ["w", letter_str] => {
            // Step 1: validate the wand and open the targeting sub-palette.
            let Some(ch) = letter_str.chars().next() else { return };
            let Some(item_kind) = letter_map.item_for_letter(ch) else { return };
            if !matches!(item_kind, ItemKind::Wand(_)) {
                return;
            }
            if !inventory.0.contains(&item_kind) {
                return;
            }
            if stats.mana < WAND_MANA_COST {
                log.push("You don't have enough mana...");
                return;
            }
            palette.pending_wand = Some(item_kind);
            palette.open = true;
            palette.target_los_only = true;
            palette.input = format!("w {letter_str} ");
            palette.selected_idx = 0;
        }
        ["w", _, _] => {
            // Step 2: letter target — pending_wand, pending_target, pending_target_entity set.
            wand_step2 = true;
        }
        _ => {
            put_back = true;
        }
    }
    drop(parts);
    if put_back {
        palette.pending_command = Some(cmd);
        return;
    }
    if wand_step2 {
        let Some(target_pos) = palette.pending_target.take() else { return };
        let target_entity = palette.pending_target_entity.take();
        // Wand kind: from pending_wand (fallback/legacy path) or from the command string.
        let wand_kind = palette.pending_wand.take().or_else(|| {
            cmd.split_whitespace()
                .nth(1)
                .and_then(|s| s.chars().next())
                .and_then(|ch| letter_map.item_for_letter(ch))
        });
        let Some(wand_kind) = wand_kind else { return };
        let Some(item) =
            find_and_remove_item(&mut inventory, |i| matches!(i, ItemKind::Wand(_)), wand_kind)
        else {
            return;
        };
        let ItemKind::Wand(gem) = item else { return };
        let effect = identities.wand_effect(gem);
        identities.identify(item);
        stats.mana -= WAND_MANA_COST;
        log.push(format!(
            "You wave the {}. (−{} MP)",
            item_display_name(item, 1, &identities),
            WAND_MANA_COST as i32
        ));
        match effect {
            WandEffect::Swapping => {
                let Some(target_entity) = target_entity else {
                    log.push("Nothing to swap with here.");
                    return;
                };
                let Ok((mut m_pos, mut m_tf, mut m_vel, _)) = monster_query.get_mut(target_entity)
                else {
                    return;
                };
                let player_pos = position.0;
                let monster_pos = m_pos.0;
                position.0 = monster_pos;
                transform.translation.x = monster_pos.x;
                transform.translation.y = monster_pos.y;
                velocity.0 = Vec2::ZERO;
                m_pos.0 = player_pos;
                m_tf.translation.x = player_pos.x;
                m_tf.translation.y = player_pos.y;
                m_vel.0 = Vec2::ZERO;
                commands.entity(entity).remove::<ExplorationGoal>();
                log.push("You swap places!");
            }
            WandEffect::Confusion => {
                let Some(target_entity) = target_entity else {
                    log.push("Nothing to confuse here.");
                    return;
                };
                let Ok((_, _, _, mut m_effects)) = monster_query.get_mut(target_entity) else {
                    return;
                };
                let mut rng = rand::thread_rng();
                let a = rng.gen_range(0.0..std::f32::consts::TAU);
                m_effects.add(
                    StatusEffect::Confused { wander_dir: Vec2::new(a.cos(), a.sin()) },
                    rng.gen_range(6.0..10.0),
                );
                log.push("The monster looks confused!");
            }
            WandEffect::Crumbling => {
                commands.trigger(WandCrumblingEvent { target: target_pos });
            }
            WandEffect::Attraction => {
                let player_pos = transform.translation.truncate();
                commands.trigger(WandAttractionEvent { target: target_pos, player_pos });
            }
        }
    }
}

/// Confuses every monster in the reader's line of sight.
pub fn on_monster_confusion(
    trigger: On<MonsterConfusionEvent>,
    mut monsters: Query<(&Transform, &mut StatusEffects), With<Monster>>,
    dungeon_state: Res<DungeonState>,
    mut log: ResMut<MessageLog>,
) {
    let origin = trigger.event().origin;
    let mut rng = rand::thread_rng();
    let mut count = 0;
    for (tf, mut effects) in monsters.iter_mut() {
        let mpos = tf.translation.truncate();
        let seg = GeoLine::new(geo::Coord { x: mpos.x, y: mpos.y }, geo::Coord {
            x: origin.x,
            y: origin.y,
        });
        if dungeon_state.solid_rock.intersects(&seg) {
            continue; // a wall blocks line of sight
        }
        let a = rng.gen_range(0.0..std::f32::consts::TAU);
        effects.add(
            StatusEffect::Confused { wander_dir: Vec2::new(a.cos(), a.sin()) },
            rng.gen_range(6.0..10.0),
        );
        count += 1;
    }
    if count == 0 {
        log.push("You hear perfectly normal laughter in the distance.");
    } else if count == 1 {
        log.push(format!("The monster seems confused!"));
    } else {
        log.push(format!("{count} monsters start acting confused!"));
    }
}

const ATTRACTION_RADIUS: f32 = 70.0;
const ATTRACTION_SPEED: f32 = 250.0;

/// Pulls every physics entity within `ATTRACTION_RADIUS` of the target toward the player,
/// and starts the pickup animation for any ground items in the same radius.
pub fn on_wand_attraction(
    trigger: On<WandAttractionEvent>,
    mut commands: Commands,
    spatial_query: SpatialQuery,
    mut body_query: Query<(&Transform, &mut LinearVelocity), Without<Player>>,
    items: Query<(Entity, &Transform), (With<Item>, Without<PickingUp>)>,
) {
    let WandAttractionEvent { target, player_pos } = *trigger.event();

    let hits = spatial_query.shape_intersections(
        &Collider::circle(ATTRACTION_RADIUS),
        target,
        0.0,
        &SpatialQueryFilter::from_mask(GameLayer::Dynamic),
    );
    for entity in hits {
        let Ok((tf, mut vel)) = body_query.get_mut(entity) else { continue };
        let dir = (player_pos - tf.translation.truncate()).normalize_or_zero();
        vel.0 = dir * ATTRACTION_SPEED;
    }

    for (entity, tf) in &items {
        let item_pos = tf.translation.truncate();
        if item_pos.distance(target) <= ATTRACTION_RADIUS {
            commands.entity(entity).insert(PickingUp {
                progress: 0.0,
                origin: item_pos,
                target: player_pos,
                initial_scale: tf.scale,
            });
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

#[cfg(test)]
mod tests {
    use rand::{SeedableRng, rngs::StdRng};

    use super::*;

    #[test]
    fn forget_scrambles_three_known_potions() {
        let mut rng = StdRng::seed_from_u64(7);
        let mut ids = ItemIdentities::default();
        ids.randomize(&mut rng);

        // Identify four potion appearances (and nothing else), so Forgetting must target potions.
        let known = [PotionColor::Red, PotionColor::Green, PotionColor::Blue, PotionColor::Yellow];
        for c in known {
            ids.identify(ItemKind::Potion(c));
        }
        let before: Vec<(PotionColor, PotionEffect)> =
            ALL_POTION_COLORS.iter().map(|&c| (c, ids.potion_effect(c))).collect();

        let (n, word) = ids.forget(&mut rng).expect("had four known potions to forget");
        assert_eq!(word, "potion");
        assert_eq!(n, 3, "should scramble exactly three known appearances");

        // The appearances that changed effect are exactly the ones that became unidentified, and
        // each changed appearance now has a *different* effect (a true derangement).
        let mut changed = 0;
        for &(c, old_effect) in &before {
            let new_effect = ids.potion_effect(c);
            if new_effect != old_effect {
                assert!(
                    !ids.is_identified(ItemKind::Potion(c)),
                    "scrambled potion stays identified"
                );
                changed += 1;
            }
        }
        assert_eq!(changed, 3);

        // The global appearance→effect pairing is still a bijection (no effect duplicated/lost).
        let effects: HashSet<PotionEffect> =
            ALL_POTION_COLORS.iter().map(|&c| ids.potion_effect(c)).collect();
        assert_eq!(effects.len(), ALL_POTION_COLORS.len());
    }

    #[test]
    fn forget_with_too_few_known_does_nothing() {
        let mut rng = StdRng::seed_from_u64(7);
        let mut ids = ItemIdentities::default();
        ids.randomize(&mut rng);
        ids.identify(ItemKind::Potion(PotionColor::Red)); // only one known appearance
        assert!(ids.forget(&mut rng).is_none());
    }
}
