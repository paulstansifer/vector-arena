// Items, inventory, and pickup animation.
// When the player walks within PICKUP_RADIUS, a `PickingUp` component is added
// to the item entity.  The 0.25s ease-in animation runs on `Real` time so it
// plays at normal speed even during bullet-time.
use avian2d::prelude::{
    Collider, LinearVelocity, Position, RigidBody, SpatialQuery, SpatialQueryFilter,
};
use bevy::{prelude::*, time::Real};
use geo::{Intersects, Line as GeoLine};
use rand::{Rng, seq::SliceRandom};

pub const WAND_COOLDOWN_SECS: f32 = 15.0;

use rogue_angles::{
    GameLayer,
    dungeon::terrain::{DungeonState, random_in_playable_area},
    hud::{MessageLog, WorldTooltip},
    identity::IdentityTable,
    palette::{
        CommandInvocation, EntryOutcome, IconId, LabelPool, PaletteCommand, PaletteEntry,
        PalettePath, PaletteRegistry, Target, TargetFilter,
    },
};

use crate::{
    effects::{
        projectile::{KNOCKBACK_SPEED, MagicMissile, apply_knockback},
        scroll::{
            AcquirementEvent, BindingEvent, InstabilityEvent, MagicMappingEvent,
            MonsterConfusionEvent, SummonMonsterEvent,
        },
    },
    monster::{Monster, Stats},
    player::{Boredom, ExplorationGoal, Player},
    status_effect::{StatusEffect, StatusEffects},
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
pub struct WandAttractionEvent {
    pub target: Vec2,
    pub player_pos: Vec2,
}

/// Per-wand-instance cooldown timers. Each entry is one wand currently recharging.
/// Multiple entries with the same gem mean that many instances of that wand are on cooldown.
#[derive(Resource, Default)]
pub struct WandCooldowns(pub Vec<(WandGem, f32)>);

impl WandCooldowns {
    pub fn add(&mut self, gem: WandGem) { self.0.push((gem, WAND_COOLDOWN_SECS)); }

    /// How many wands of `gem` are ready (not on cooldown), given `total` in inventory.
    pub fn ready_count(&self, gem: WandGem, total: u16) -> u16 {
        let on_cooldown = self.0.iter().filter(|(g, _)| *g == gem).count() as u16;
        total.saturating_sub(on_cooldown)
    }

    /// Shortest remaining cooldown for any wand of `gem`, if any are cooling down.
    pub fn shortest_remaining(&self, gem: WandGem) -> Option<f32> {
        self.0.iter().filter(|(g, _)| *g == gem).map(|(_, t)| *t).reduce(f32::min)
    }

    pub fn clear(&mut self) { self.0.clear(); }
}

/// Tick down wand cooldowns on virtual time so they respect bullet-time and pause.
pub fn tick_wand_cooldowns(mut cooldowns: ResMut<WandCooldowns>, time: Res<Time>) {
    for (_, remaining) in &mut cooldowns.0 {
        *remaining -= time.delta_secs();
    }
    cooldowns.0.retain(|(_, t)| *t > 0.0);
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
    Forgetting,
    Binding,
}

const ALL_SCROLL_EFFECTS: &[ScrollEffect] = &[
    ScrollEffect::Teleport,
    ScrollEffect::SummonMonster,
    ScrollEffect::MagicMapping,
    ScrollEffect::Instability,
    ScrollEffect::Acquirement,
    ScrollEffect::Forgetting,
    ScrollEffect::Binding,
];

impl ScrollEffect {
    pub fn name(self) -> &'static str {
        match self {
            ScrollEffect::Teleport => "Teleportation",
            ScrollEffect::SummonMonster => "Monster Summoning",
            ScrollEffect::MagicMapping => "Magic Mapping",
            ScrollEffect::Instability => "Instability",
            ScrollEffect::Acquirement => "Acquirement",
            ScrollEffect::Forgetting => "Forgetting",
            ScrollEffect::Binding => "Binding",
        }
    }
}

/// Per-game roguelike identification state: which appearance maps to which effect (randomized
/// at the start of every new game) and which item kinds the player has identified by using
/// them. Three separate engine `IdentityTable`s — one per identifiable category — rather than
/// one table keyed by the umbrella `ItemKind`, since the engine's table doesn't know `ItemKind`
/// exists; `is_identified`/`identify`/`forget` below are this game's `ItemKind`-keyed API on
/// top of that.
#[derive(Resource, Default)]
pub struct ItemIdentities {
    potion: IdentityTable<PotionColor, PotionEffect>,
    scroll: IdentityTable<ScrollName, ScrollEffect>,
    wand: IdentityTable<WandGem, WandEffect>,
}

impl ItemIdentities {
    /// Reshuffle the appearance→effect pairings and forget all identifications. Call once per
    /// new game (not on descent).
    pub fn randomize(&mut self, rng: &mut impl Rng) {
        self.potion.randomize(&ALL_POTION_COLORS, &ALL_POTION_EFFECTS, rng);
        self.scroll.randomize(&ALL_SCROLL_NAMES, &ALL_SCROLL_EFFECTS, rng);
        self.wand.randomize(&ALL_WAND_GEMS, &ALL_WAND_EFFECTS, rng);
    }

    pub fn potion_effect(&self, color: PotionColor) -> PotionEffect {
        self.potion.effect_of(color).unwrap_or(PotionEffect::Confusion)
    }

    pub fn scroll_effect(&self, name: ScrollName) -> ScrollEffect {
        self.scroll.effect_of(name).unwrap_or(ScrollEffect::Teleport)
    }

    pub fn wand_effect(&self, gem: WandGem) -> WandEffect {
        self.wand.effect_of(gem).unwrap_or(WandEffect::Swapping)
    }

    pub fn is_identified(&self, item: ItemKind) -> bool {
        match item {
            ItemKind::Potion(c) => self.potion.is_identified(c),
            ItemKind::Scroll(n) => self.scroll.is_identified(n),
            ItemKind::Wand(g) => self.wand.is_identified(g),
        }
    }

    pub fn identify(&mut self, item: ItemKind) {
        match item {
            ItemKind::Potion(c) => self.potion.identify(c),
            ItemKind::Scroll(n) => self.scroll.identify(n),
            ItemKind::Wand(g) => self.wand.identify(g),
        }
    }

    /// Implements a Scroll of Forgetting: pick an item type (potions or scrolls) that has at least
    /// two identified appearances and scramble up to three of them (see
    /// `IdentityTable::forget_some`). Returns the number of forgotten appearances and the type
    /// word for the message, or `None` if neither type had enough known appearances to scramble.
    pub fn forget(&mut self, rng: &mut impl Rng) -> Option<(usize, &'static str)> {
        // Each type is a candidate only if it has at least two known appearances to permute.
        let mut candidates: Vec<bool> = Vec::new(); // true = potions, false = scrolls
        if self.potion.known_count() >= 2 {
            candidates.push(true);
        }
        if self.scroll.known_count() >= 2 {
            candidates.push(false);
        }
        let &do_potions = candidates.choose(rng)?;

        if do_potions {
            self.potion.forget_some(3, rng).map(|n| (n, "potion"))
        } else {
            self.scroll.forget_some(3, rng).map(|n| (n, "scroll"))
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
    mut labels: ResMut<LabelPool<ItemKind>>,
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
            labels.get_or_assign(item.0);
            commands.entity(entity).despawn();
        }
    }
}

// ── Command palette integration ──────────────────────────────────────────────
//
// Four independent commands (q/r/w/e), each a Submenu (list matching inventory
// items, stable-lettered via LabelPool<ItemKind>) leading to a Run handler —
// except "w", whose submenu entries lead to PickTarget instead, since waving a
// wand needs a target. No more polled mailbox: the engine calls each handler
// directly with the resolved CommandInvocation.

/// Stable icon handle for an item, keyed by its position in `ALL_ITEM_KINDS`.
pub fn icon_id_for_item(item: ItemKind) -> IconId {
    IconId(ALL_ITEM_KINDS.iter().position(|k| *k == item).unwrap_or(0) as u32)
}

pub fn item_for_icon_id(id: IconId) -> Option<ItemKind> {
    ALL_ITEM_KINDS.get(id.0 as usize).copied()
}

pub fn register_item_commands(world: &mut World) {
    let quaff_submenu_id = world.register_system(quaff_submenu);
    let read_submenu_id = world.register_system(read_submenu);
    let wave_submenu_id = world.register_system(wave_submenu);
    let examine_submenu_id = world.register_system(examine_submenu);
    let quaff_handler = world.register_system(execute_quaff);
    let read_handler = world.register_system(execute_read);
    let wave_handler = world.register_system(execute_wave);
    let examine_handler = world.register_system(execute_examine);

    let mut registry = world.resource_mut::<PaletteRegistry>();
    registry.commands.push(PaletteCommand {
        key: "q".to_string(),
        description: "quaff a potion".to_string(),
        icon: None,
        outcome: EntryOutcome::Submenu(quaff_submenu_id),
        handler: quaff_handler,
    });
    registry.commands.push(PaletteCommand {
        key: "r".to_string(),
        description: "read a scroll".to_string(),
        icon: None,
        outcome: EntryOutcome::Submenu(read_submenu_id),
        handler: read_handler,
    });
    registry.commands.push(PaletteCommand {
        key: "w".to_string(),
        description: "wave a wand".to_string(),
        icon: None,
        outcome: EntryOutcome::Submenu(wave_submenu_id),
        handler: wave_handler,
    });
    registry.commands.push(PaletteCommand {
        key: "e".to_string(),
        description: "examine inventory".to_string(),
        icon: None,
        outcome: EntryOutcome::Submenu(examine_submenu_id),
        handler: examine_handler,
    });
}

/// Builds terminal entries for each unique inventory item matching `filter`,
/// keyed by its stable `LabelPool` letter. `leaf_outcome` is `Run` for
/// immediate-use items, or `PickTarget` for ones that need a target next
/// (wands).
fn inventory_entries(
    prefix: &str,
    inventory: &Inventory,
    item_filter: impl Fn(ItemKind) -> bool,
    labels: &LabelPool<ItemKind>,
    identities: &ItemIdentities,
    leaf_outcome: EntryOutcome,
) -> Vec<PaletteEntry> {
    let mut unique: Vec<(ItemKind, u16)> = Vec::new();
    for item in &inventory.0 {
        if item_filter(*item) {
            if let Some(entry) = unique.iter_mut().find(|(t, _)| t == item) {
                entry.1 += 1;
            } else {
                unique.push((*item, 1));
            }
        }
    }
    let mut entries: Vec<PaletteEntry> = unique
        .iter()
        .filter_map(|(item, count)| {
            labels.get(*item).map(|letter| PaletteEntry {
                key: format!("{prefix} {letter}"),
                description: item_display_name(*item, *count, identities),
                icon: Some(icon_id_for_item(*item)),
                outcome: leaf_outcome.clone(),
            })
        })
        .collect();
    entries.sort_by_key(|e| e.key.clone());
    entries
}

fn quaff_submenu(
    In(path): In<PalettePath>,
    inventory: Query<&Inventory, With<Player>>,
    labels: Res<LabelPool<ItemKind>>,
    identities: Res<ItemIdentities>,
) -> Vec<PaletteEntry> {
    let Ok(inventory) = inventory.single() else { return Vec::new() };
    inventory_entries(
        &path.join(" "),
        inventory,
        |i| matches!(i, ItemKind::Potion(_)),
        &labels,
        &identities,
        EntryOutcome::Run,
    )
}

fn read_submenu(
    In(path): In<PalettePath>,
    inventory: Query<&Inventory, With<Player>>,
    labels: Res<LabelPool<ItemKind>>,
    identities: Res<ItemIdentities>,
) -> Vec<PaletteEntry> {
    let Ok(inventory) = inventory.single() else { return Vec::new() };
    inventory_entries(
        &path.join(" "),
        inventory,
        |i| matches!(i, ItemKind::Scroll(_)),
        &labels,
        &identities,
        EntryOutcome::Run,
    )
}

fn examine_submenu(
    In(path): In<PalettePath>,
    inventory: Query<&Inventory, With<Player>>,
    labels: Res<LabelPool<ItemKind>>,
    identities: Res<ItemIdentities>,
) -> Vec<PaletteEntry> {
    let Ok(inventory) = inventory.single() else { return Vec::new() };
    inventory_entries(&path.join(" "), inventory, |_| true, &labels, &identities, EntryOutcome::Run)
}

fn wave_submenu(
    In(path): In<PalettePath>,
    inventory: Query<&Inventory, With<Player>>,
    labels: Res<LabelPool<ItemKind>>,
    identities: Res<ItemIdentities>,
) -> Vec<PaletteEntry> {
    let Ok(inventory) = inventory.single() else { return Vec::new() };
    inventory_entries(
        &path.join(" "),
        inventory,
        |i| matches!(i, ItemKind::Wand(_)),
        &labels,
        &identities,
        EntryOutcome::PickTarget { verb: "Wave at".to_string(), filter: TargetFilter::Any },
    )
}

const WAND_MANA_COST: f32 = 5.0;
/// HP every potion restores, on top of its identified effect.
const POTION_HEAL: f32 = 15.0;

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

/// Identifies an item, reducing boredom if it was previously unknown.
pub fn identify_item(item: ItemKind, identities: &mut ItemIdentities, boredom: &mut Boredom) {
    if !identities.is_identified(item) {
        boredom.reduce(15.0);
    }
    identities.identify(item);
}

fn letter_from_path(path: &PalettePath) -> Option<char> {
    path.get(1).and_then(|s| s.chars().next())
}

fn execute_quaff(
    In(invocation): In<CommandInvocation>,
    mut player_query: Query<(&mut Inventory, &mut Stats, &mut StatusEffects), With<Player>>,
    labels: Res<LabelPool<ItemKind>>,
    mut identities: ResMut<ItemIdentities>,
    mut boredom: ResMut<Boredom>,
    mut log: ResMut<MessageLog>,
) {
    let Some(letter) = letter_from_path(&invocation.path) else { return };
    let Some(item_kind) = labels.key_for_letter(letter) else { return };
    let Ok((mut inventory, mut stats, mut player_effects)) = player_query.single_mut() else {
        return;
    };
    let Some(item) =
        find_and_remove_item(&mut inventory, |i| matches!(i, ItemKind::Potion(_)), item_kind)
    else {
        return;
    };
    let ItemKind::Potion(color) = item else { return };
    // Compute the gain from the amount actually healed, not a separate literal: these were
    // 20.0 and 15.0 respectively, so a quaff at low HP claimed "+20 HP" while restoring 15.
    let gained = POTION_HEAL.min(stats.max_hp - stats.hp).max(0.0);
    stats.hp = (stats.hp + POTION_HEAL).min(stats.max_hp);
    let mut rng = rand::thread_rng();
    let (kind, duration) = potion_status_effect(identities.potion_effect(color), &mut rng);
    player_effects.add(kind, duration);
    identify_item(item, &mut identities, &mut boredom);
    log.push(format!(
        "You quaff {}. (+{} HP)",
        item_display_name(item, 1, &identities),
        gained as i32,
    ));
}

fn execute_read(
    In(invocation): In<CommandInvocation>,
    mut player_query: Query<
        (Entity, &mut Inventory, &mut Stats, &mut Position, &mut Transform, &mut LinearVelocity),
        With<Player>,
    >,
    labels: Res<LabelPool<ItemKind>>,
    mut identities: ResMut<ItemIdentities>,
    mut boredom: ResMut<Boredom>,
    mut log: ResMut<MessageLog>,
    dungeon_state: Res<DungeonState>,
    mut commands: Commands,
) {
    let Some(letter) = letter_from_path(&invocation.path) else { return };
    let Some(item_kind) = labels.key_for_letter(letter) else { return };
    let Ok((entity, mut inventory, mut stats, mut position, mut transform, mut velocity)) =
        player_query.single_mut()
    else {
        return;
    };
    let Some(item) =
        find_and_remove_item(&mut inventory, |i| matches!(i, ItemKind::Scroll(_)), item_kind)
    else {
        return;
    };
    let ItemKind::Scroll(name) = item else { return };
    let effect = identities.scroll_effect(name);
    identify_item(item, &mut identities, &mut boredom);

    // Every scroll restores 20 MP in addition to its identified effect.
    let mana_gained = 20.0_f32.min(stats.max_mana - stats.mana);
    stats.mana = (stats.mana + 20.0).min(stats.max_mana);
    let player_pos = transform.translation.truncate();
    log.push(format!(
        "You read {}. (+{} MP)",
        item_display_name(item, 1, &identities),
        mana_gained as i32,
    ));

    match effect {
        ScrollEffect::Teleport => {
            if let Some(dest) =
                random_in_playable_area(&dungeon_state.playable_area, &mut rand::thread_rng())
            {
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
        ScrollEffect::Binding => {
            commands.trigger(BindingEvent { origin: player_pos });
        }
        ScrollEffect::Forgetting => match identities.forget(&mut rand::thread_rng()) {
            Some((_n, type_word)) => {
                log.push(format!("You're not as sure as you used to be about those {type_word}s."));
            }
            None => {
                log.push("You don't remember having forgotten anything.");
            }
        },
    }
}

fn execute_examine(In(_invocation): In<CommandInvocation>) {
    // Read-only; no effect.
}

fn execute_wave(
    In(invocation): In<CommandInvocation>,
    mut player_query: Query<
        (Entity, &mut Inventory, &mut Stats, &mut Position, &mut Transform, &mut LinearVelocity),
        (With<Player>, Without<Monster>),
    >,
    mut monster_query: Query<
        (&mut Position, &mut Transform, &mut LinearVelocity, &mut StatusEffects),
        (With<Monster>, Without<Player>),
    >,
    labels: Res<LabelPool<ItemKind>>,
    mut identities: ResMut<ItemIdentities>,
    mut wand_cooldowns: ResMut<WandCooldowns>,
    mut boredom: ResMut<Boredom>,
    mut log: ResMut<MessageLog>,
    mut commands: Commands,
) {
    let Some(letter) = letter_from_path(&invocation.path) else { return };
    let Some(wand_kind) = labels.key_for_letter(letter) else { return };
    let ItemKind::Wand(gem) = wand_kind else { return };
    let Some(target) = invocation.target else { return };

    let Ok((entity, inventory, mut stats, mut position, mut transform, mut velocity)) =
        player_query.single_mut()
    else {
        return;
    };
    if !inventory.0.contains(&wand_kind) {
        return;
    }
    let total = inventory.0.iter().filter(|i| **i == wand_kind).count() as u16;
    if wand_cooldowns.ready_count(gem, total) == 0 {
        log.push("That wand is still recharging...");
        return;
    }
    if stats.mana < WAND_MANA_COST {
        log.push("You don't have enough mana...");
        return;
    }

    let effect = identities.wand_effect(gem);
    identify_item(wand_kind, &mut identities, &mut boredom);
    stats.mana -= WAND_MANA_COST;
    wand_cooldowns.add(gem);
    log.push(format!(
        "You wave the {}. (−{} MP)",
        item_display_name(wand_kind, 1, &identities),
        WAND_MANA_COST as i32,
    ));

    let target_entity = if let Target::Entity(e) = target { Some(e) } else { None };
    let target_pos = match target {
        Target::Point(p) => p,
        Target::Entity(e) => monster_query
            .get(e)
            .map(|(pos, _, _, _)| pos.0)
            .unwrap_or_else(|_| transform.translation.truncate()),
    };

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
            commands.trigger(rogue_angles::effects::crumble_terrain::CrumbleTerrainRequest {
                target: target_pos,
            });
        }
        WandEffect::Attraction => {
            let player_pos = transform.translation.truncate();
            commands.trigger(WandAttractionEvent { target: target_pos, player_pos });
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
        log.push("The monster seems confused!");
    } else {
        log.push(format!("{count} monsters start acting confused!"));
    }
}

const ATTRACTION_RADIUS: f32 = 70.0;

/// Pulls every physics entity within `ATTRACTION_RADIUS` of the target toward the player,
/// and starts the pickup animation for any ground items in the same radius.
pub fn on_wand_attraction(
    trigger: On<WandAttractionEvent>,
    mut commands: Commands,
    spatial_query: SpatialQuery,
    mut body_query: Query<
        (&Transform, &mut LinearVelocity),
        (Without<MagicMissile>, With<RigidBody>, Without<Player>),
    >,
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
        apply_knockback(&mut commands, &mut vel, entity, dir, KNOCKBACK_SPEED * 4.0);
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
    use std::collections::HashSet;

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

    // ── Command palette: inventory listing ──────────────────────────────────

    /// Inventory: Red×2, Readme×2, Blue×1, Agents×1 (with stable letters:
    /// Red='a', Readme='b', Blue='c', Agents='d').
    fn test_inventory() -> Inventory {
        Inventory(vec![
            ItemKind::Potion(PotionColor::Red),
            ItemKind::Potion(PotionColor::Red),
            ItemKind::Scroll(ScrollName::Readme),
            ItemKind::Potion(PotionColor::Blue),
            ItemKind::Scroll(ScrollName::Readme),
            ItemKind::Scroll(ScrollName::Agents),
        ])
    }

    fn test_label_pool() -> LabelPool<ItemKind> {
        let mut pool = LabelPool::default();
        pool.get_or_assign(ItemKind::Potion(PotionColor::Red));
        pool.get_or_assign(ItemKind::Scroll(ScrollName::Readme));
        pool.get_or_assign(ItemKind::Potion(PotionColor::Blue));
        pool.get_or_assign(ItemKind::Scroll(ScrollName::Agents));
        pool
    }

    fn keys(entries: &[PaletteEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.key.as_str()).collect()
    }

    #[test]
    fn quaff_uses_stable_letters() {
        let inv = test_inventory();
        let pool = test_label_pool();
        let identities = ItemIdentities::default();
        let entries = inventory_entries(
            "q",
            &inv,
            |i| matches!(i, ItemKind::Potion(_)),
            &pool,
            &identities,
            EntryOutcome::Run,
        );
        assert_eq!(keys(&entries), ["q a", "q c"]);
        assert_eq!(entries[0].description, "2 Red potions");
        assert_eq!(entries[1].description, "a Blue potion");
    }

    #[test]
    fn read_uses_stable_letters() {
        let inv = test_inventory();
        let pool = test_label_pool();
        let identities = ItemIdentities::default();
        let entries = inventory_entries(
            "r",
            &inv,
            |i| matches!(i, ItemKind::Scroll(_)),
            &pool,
            &identities,
            EntryOutcome::Run,
        );
        assert_eq!(keys(&entries), ["r b", "r d"]);
        assert_eq!(entries[0].description, "2 scrolls titled 'Readme'");
        assert_eq!(entries[1].description, "a scroll titled 'Agents'");
    }

    #[test]
    fn wave_entries_lead_to_pick_target_not_run() {
        let mut pool: LabelPool<ItemKind> = LabelPool::default();
        pool.get_or_assign(ItemKind::Wand(WandGem::Ruby));
        let inv = Inventory(vec![ItemKind::Wand(WandGem::Ruby)]);
        let identities = ItemIdentities::default();
        let entries = inventory_entries(
            "w",
            &inv,
            |i| matches!(i, ItemKind::Wand(_)),
            &pool,
            &identities,
            EntryOutcome::PickTarget { verb: "Wave at".to_string(), filter: TargetFilter::Any },
        );
        assert_eq!(keys(&entries), ["w a"]);
        assert!(matches!(entries[0].outcome, EntryOutcome::PickTarget { .. }));
    }

    #[test]
    fn icon_id_roundtrips_through_all_item_kinds() {
        for &kind in ALL_ITEM_KINDS {
            assert_eq!(item_for_icon_id(icon_id_for_item(kind)), Some(kind));
        }
    }
}
