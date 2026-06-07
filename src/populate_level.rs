// Spawns the inhabitants and interactables of a dungeon level: player, monsters,
// items, and the down staircase. Called after structural elements (terrain,
// navmesh, FOV) have been put in place by `spawn_game_world`.
use avian2d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::prelude::*;
use geo::{MultiPolygon, Rect};
use rand::prelude::*;

use crate::{
    AGENT_RADIUS, GameLayer, GameState, Staircase, StaircaseFogCopy,
    dungeon::terrain::random_in_playable_area,
    effects::projectile::MonsterShootTimer,
    fov,
    item::{ALL_ITEM_KINDS, Inventory, Item, ItemKind, item_name},
    monster::{MONSTER_MAX_HP, MONSTER_SPEED, Monster, MonsterDrop, MonsterState, Stats},
    objects::spawn_unstable_sigil,
    player::{MoveTarget, PLAYER_SPEED, Player},
    sprite::{SvgSprite, sprite_spec},
    status_effect::StatusEffects,
    ui::WorldTooltip,
};

pub fn populate(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    rooms: &[Rect<f32>],
    playable_area: &MultiPolygon<f32>,
    archipelago_id: Entity,
    depth: u32,
    saved_player: Option<(Stats, Inventory)>,
    monster_letters: &mut crate::command_palette::LetterMap,
) {
    let mut rng = rand::thread_rng();

    let room_center = |r: &Rect<f32>| {
        let c = r.center();
        Vec2::new(c.x, c.y)
    };

    // Pick the player's room by index so we can exclude it when placing the staircase.
    let player_room_idx = if rooms.is_empty() { 0 } else { rng.gen_range(0..rooms.len()) };
    let player_position = rooms.get(player_room_idx).map(room_center).unwrap_or(Vec2::ZERO);

    let non_player_centers: Vec<Vec2> = rooms
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != player_room_idx)
        .map(|(_, r)| room_center(r))
        .collect();

    let staircase_position = non_player_centers
        .choose(&mut rng)
        .copied()
        .unwrap_or(player_position + Vec2::new(50.0, 0.0));

    let (initial_stats, initial_inventory) = saved_player.unwrap_or((
        Stats { hp: 100.0, max_hp: 100.0, mana: 80.0, max_mana: 80.0 },
        Inventory::default(),
    ));

    commands.spawn((
        DespawnOnExit(GameState::InLevel),
        (Player, initial_inventory, initial_stats, StatusEffects::default()),
        SvgSprite { svg_path: "sprites/wizard.svg".into(), param: None },
        Transform::from_translation(player_position.extend(fov::MOVABLE_Z))
            .with_scale(Vec3::splat(0.4)),
        RigidBody::Dynamic,
        Collider::circle(AGENT_RADIUS),
        ColliderDensity(8.0),
        CollisionLayers::new(GameLayer::Dynamic, [GameLayer::Wall, GameLayer::Dynamic]),
        LockedAxes::ROTATION_LOCKED,
        MoveTarget::default(),
        Agent2dBundle {
            agent: Default::default(),
            settings: AgentSettings {
                radius: AGENT_RADIUS,
                desired_speed: PLAYER_SPEED,
                max_speed: PLAYER_SPEED * 1.2,
            },
            archipelago_ref: ArchipelagoRef2d::new(archipelago_id),
        },
        AgentTarget2d::None,
    ));

    let monster_mesh = meshes.add(Circle::new(AGENT_RADIUS));

    // One more monster per depth level (2 at depth 1, 3 at depth 2, …).
    let monster_count = (depth as usize + 1).min(non_player_centers.len());
    for &position in non_player_centers.iter().take(monster_count) {
        let drop = if rng.gen_bool(0.6) { ALL_ITEM_KINDS.choose(&mut rng).copied() } else { None };
        let monster = commands
            .spawn((
                DespawnOnExit(GameState::InLevel),
                (
                    Monster,
                    MonsterState::Sleeping { timer: rng.gen_range(3.0..5.0) },
                    Stats { hp: MONSTER_MAX_HP, max_hp: MONSTER_MAX_HP, ..default() },
                    StatusEffects::default(),
                ),
                WorldTooltip::default(),
                MonsterShootTimer::new(),
                Mesh2d(monster_mesh.clone()),
                MeshMaterial2d(materials.add(ColorMaterial::from(Color::srgb(0.85, 0.12, 0.12)))),
                Transform::from_translation(position.extend(fov::MOVABLE_Z)),
                RigidBody::Dynamic,
                Collider::circle(AGENT_RADIUS),
                CollisionLayers::new(GameLayer::Dynamic, [GameLayer::Wall, GameLayer::Dynamic]),
                LockedAxes::ROTATION_LOCKED,
                Agent2dBundle {
                    agent: Default::default(),
                    settings: AgentSettings {
                        radius: AGENT_RADIUS,
                        desired_speed: MONSTER_SPEED,
                        max_speed: MONSTER_SPEED * 1.2,
                    },
                    archipelago_ref: ArchipelagoRef2d::new(archipelago_id),
                },
                AgentTarget2d::None,
            ))
            .id();
        monster_letters.assign_monster(monster);
        if let Some(kind) = drop {
            commands.entity(monster).insert(MonsterDrop(kind));
        }
    }

    let item_count = rng.gen_range(4..=5);
    let chosen_kinds: Vec<ItemKind> =
        ALL_ITEM_KINDS.choose_multiple(&mut rng, item_count).copied().collect();

    for kind in chosen_kinds {
        let Some(pt) = random_in_playable_area(playable_area, &mut rng) else { continue };
        let pos = Vec3::new(pt.x, pt.y, fov::ON_FLOOR_Z);

        let (svg_path, param) = sprite_spec(kind);
        commands.spawn((
            DespawnOnExit(GameState::InLevel),
            Item(kind),
            WorldTooltip(item_name(kind, 1).to_string()),
            SvgSprite { svg_path: svg_path.into(), param: Some(param) },
            Transform::from_translation(pos).with_scale(Vec3::splat(0.4)),
        ));
    }

    let sigil_count = rng.gen_range(1..=5);
    for _ in 0..sigil_count {
        let Some(pt) = random_in_playable_area(playable_area, &mut rng) else { continue };
        spawn_unstable_sigil(commands, meshes, materials, pt);
    }

    spawn_staircase(commands, meshes, materials, staircase_position);
}

fn spawn_staircase(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    position: Vec2,
) {
    commands.spawn((
        DespawnOnExit(GameState::InLevel),
        Staircase,
        SvgSprite { svg_path: "sprites/hatch.svg".into(), param: None },
        Transform::from_translation(position.extend(fov::ON_FLOOR_Z)).with_scale(Vec3::splat(0.4)),
        Visibility::default(),
    ));
    // Hatch fill color (#ac9d93); the fog-copy mesh is rewritten each frame by
    // update_staircase_fog_copy via geo difference against the FOV polygon.
    let fog_color = materials.add(ColorMaterial::from(Color::srgb(0.675, 0.616, 0.576)));
    let fog_mesh = meshes.add(Mesh::new(
        bevy_mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    ));
    commands.spawn((
        DespawnOnExit(GameState::InLevel),
        StaircaseFogCopy,
        Visibility::default(),
        Mesh2d(fog_mesh),
        MeshMaterial2d(fog_color),
        // We need to subtract the FOV, so this needs an absolute location
        Transform::from_translation(Vec3::new(0.0, 0.0, fov::TERRAIN_Z)),
    ));
}
