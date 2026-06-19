// Scroll effects beyond plain teleportation. `item::execute_item_command` resolves a scroll's
// identified `ScrollEffect` and, for the four effects below, triggers one of these events; each
// observer carries its own system access (meshes, archipelago, exploration, monster queries) so
// the read handler itself stays small.
use bevy::prelude::*;
use bevy_landmass::prelude::*;
use geo::{BooleanOps, Buffer, Simplify};

use crate::{
    AGENT_RADIUS, GameState,
    command_palette::LetterMap,
    dungeon::terrain::{self, DungeonState, random_near},
    fov::{self, ExplorationState, NeverExploredMeshMarker, WALL_FOV_DEPTH},
    item::{Item, item_name, random_item_kind},
    objects::spawn_unstable_sigil,
    populate_level::spawn_monster,
    sprite::{SvgSprite, sprite_spec},
    ui::{MessageLog, WorldTooltip},
};

#[derive(Event)]
pub struct SummonMonsterEvent {
    pub origin: Vec2,
}

#[derive(Event)]
pub struct MagicMappingEvent;

#[derive(Event)]
pub struct InstabilityEvent {
    pub origin: Vec2,
}

#[derive(Event)]
pub struct MonsterConfusionEvent {
    pub origin: Vec2,
}

#[derive(Event)]
pub struct AcquirementEvent {
    pub origin: Vec2,
}

/// Spawns a single monster 40-80 units from the reader.
pub fn on_summon_monster(
    trigger: On<SummonMonsterEvent>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    archipelago: Single<Entity, With<Archipelago2d>>,
    dungeon_state: Res<DungeonState>,
    mut monster_letters: ResMut<LetterMap>,
    mut log: ResMut<MessageLog>,
) {
    let origin = trigger.event().origin;
    let mut rng = rand::thread_rng();
    let Some(pos) = random_near(&dungeon_state.playable_area, origin, 40.0, 80.0, &mut rng) else {
        log.push("Nothing happens.");
        return;
    };
    let mesh = meshes.add(Circle::new(AGENT_RADIUS));
    spawn_monster(
        &mut commands,
        &mut materials,
        mesh,
        *archipelago,
        pos,
        &mut monster_letters,
        None,
        &mut rng,
    );
    log.push("A monster appears!");
}

/// Marks the whole walkable area (buffered out by `WALL_FOV_DEPTH` so wall edges show) as explored.
pub fn on_magic_mapping(
    _trigger: On<MagicMappingEvent>,
    dungeon_state: Res<DungeonState>,
    mut exploration: ResMut<ExplorationState>,
    never_explored: Query<&Mesh2d, With<NeverExploredMeshMarker>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut log: ResMut<MessageLog>,
) {
    let mapped = dungeon_state.playable_area.buffer(WALL_FOV_DEPTH);
    exploration.0 = exploration.0.difference(&mapped).simplify(1e-1);
    // Rebuild the fog mesh now: `update_fov` skips recomputation while the player is stationary.
    if let Ok(handle) = never_explored.single()
        && let Some(mesh) = meshes.get_mut(&handle.0)
    {
        *mesh = terrain::geometry_to_mesh(&exploration.0);
    }
    log.push("Hm, this scroll seems to have a map on it.");
}

/// Spawns three unstable sigils 40-120 units from the reader.
pub fn on_instability(
    trigger: On<InstabilityEvent>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    dungeon_state: Res<DungeonState>,
    mut log: ResMut<MessageLog>,
) {
    let origin = trigger.event().origin;
    let mut rng = rand::thread_rng();
    let mut spawned = 0;
    for _ in 0..3 {
        let Some(pos) = random_near(&dungeon_state.playable_area, origin, 40.0, 120.0, &mut rng)
        else {
            continue;
        };
        spawn_unstable_sigil(&mut commands, &mut meshes, &mut materials, pos, &mut rng);
        spawned += 1;
    }
    if spawned == 0 {
        log.push("The scroll crumbles to dust.");
    } else {
        log.push("Unstable sigils flicker into existence nearby!");
    }
}

/// Spawns three random items 20-60 units from the reader.
pub fn on_acquirement(
    trigger: On<AcquirementEvent>,
    mut commands: Commands,
    dungeon_state: Res<DungeonState>,
    mut log: ResMut<MessageLog>,
) {
    let origin = trigger.event().origin;
    let mut rng = rand::thread_rng();
    for _ in 0..3 {
        let Some(pos) = random_near(&dungeon_state.playable_area, origin, 20.0, 60.0, &mut rng)
        else {
            continue;
        };
        let kind = random_item_kind(&mut rng);
        let (svg_path, param) = sprite_spec(kind);
        commands.spawn((
            DespawnOnExit(GameState::InLevel),
            Item(kind),
            WorldTooltip(item_name(kind, 1).to_string()),
            SvgSprite { svg_path: svg_path.into(), param: Some(param) },
            Transform::from_translation(pos.extend(fov::ON_FLOOR_Z)).with_scale(Vec3::splat(0.4)),
        ));
    }

    log.push("Were those things there before?");
}
