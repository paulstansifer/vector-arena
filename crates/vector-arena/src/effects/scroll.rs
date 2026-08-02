// Scroll effects beyond plain teleportation. `item::execute_item_command` resolves a scroll's
// identified `ScrollEffect` and, for the four effects below, triggers one of these events; each
// observer carries its own system access (meshes, archipelago, exploration, monster queries) so
// the read handler itself stays small.
use avian2d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::prelude::*;
use geo::{Intersects, Line as GeoLine};
use rand::seq::SliceRandom;

use rogue_angles::{
    AGENT_RADIUS, GameLayer,
    dungeon::terrain::{self, DungeonState, random_near},
    fov::{self, ExplorationState, NeverExploredMeshMarker, WALL_FOV_DEPTH},
};

use crate::{
    GameState,
    command_palette::LetterMap,
    effects::{
        rope::{RopeVisuals, spawn_rope},
        unstable_sigils::spawn_unstable_sigil,
    },
    item::{Item, item_name, random_item_kind},
    monster::Monster,
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

#[derive(Event)]
pub struct BindingEvent {
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

const BINDING_RAY_MAX: f32 = 600.0;

/// Randomly picks a monster in LOS, casts a ray from it in its movement direction, and attaches
/// a rope between the monster and the first terrain surface the ray hits.
pub fn on_binding(
    trigger: On<BindingEvent>,
    monsters: Query<(Entity, &Transform, &LinearVelocity), With<Monster>>,
    dungeon_state: Res<DungeonState>,
    anchor_query: Query<(&GlobalTransform, &RigidBody)>,
    spatial_query: SpatialQuery,
    rope_visuals: Res<RopeVisuals>,
    mut commands: Commands,
    mut log: ResMut<MessageLog>,
) {
    let origin = trigger.event().origin;
    let mut rng = rand::thread_rng();

    let los_monsters: Vec<_> = monsters
        .iter()
        .filter(|(_, tf, _)| {
            let mpos = tf.translation.truncate();
            let seg = GeoLine::new(geo::Coord { x: origin.x, y: origin.y }, geo::Coord {
                x: mpos.x,
                y: mpos.y,
            });
            !dungeon_state.solid_rock.intersects(&seg)
        })
        .collect();

    let Some(&(_, tf, vel)) = los_monsters.choose(&mut rng) else {
        log.push("There is nothing nearby to bind.");
        return;
    };
    let monster_pos = tf.translation.truncate();

    // Use the monster's velocity direction; fall back to the direction away from the reader.
    let raw_dir = if vel.0.length() > 1.0 {
        vel.0
    } else {
        let d = monster_pos - origin;
        if d.length() > 1e-5 { d } else { Vec2::X }
    };
    let Ok(dir) = Dir2::new(raw_dir) else {
        log.push("The scroll fizzles.");
        return;
    };

    let wall_filter = SpatialQueryFilter::from_mask([GameLayer::Wall]);
    let Some(hit) = spatial_query.cast_ray(monster_pos, dir, BINDING_RAY_MAX, true, &wall_filter)
    else {
        log.push("The scroll fizzles.");
        return;
    };

    let terrain_point = monster_pos + *dir * hit.distance;
    spawn_rope(
        &mut commands,
        monster_pos,
        terrain_point,
        &spatial_query,
        &anchor_query,
        Some(&rope_visuals),
    );
    log.push("Mystic cords bind the creature to the wall!");
}
