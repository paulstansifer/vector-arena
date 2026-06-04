// Spawns the inhabitants and interactables of a dungeon level: player, monsters,
// items, and the down staircase. Called after structural elements (terrain,
// navmesh, FOV) have been put in place by `spawn_game_world`.
use avian2d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::prelude::*;
use geo::{MultiPolygon, Rect};
use rand::prelude::*;

use crate::{
    AGENT_RADIUS, GameLayer, GameState, Staircase,
    dungeon::terrain::random_in_playable_area,
    effects::projectile::MonsterShootTimer,
    fov,
    item::{ALL_ITEM_KINDS, Inventory, Item, ItemKind, item_name},
    monster::{MONSTER_MAX_HP, MONSTER_SPEED, Monster, MonsterDrop, MonsterState, Stats},
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

    let room_center = |r: &Rect<f32>| { let c = r.center(); Vec2::new(c.x, c.y) };

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
        Mesh2d(meshes.add(Circle::new(AGENT_RADIUS))),
        MeshMaterial2d(materials.add(ColorMaterial::from(Color::srgb(0.15, 0.65, 0.95)))),
        Transform::from_translation(player_position.extend(fov::MOVABLE_Z)),
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
                (Monster, MonsterState::Sleeping { timer: rng.gen_range(3.0..5.0) }, Stats { hp: MONSTER_MAX_HP, max_hp: MONSTER_MAX_HP, ..default() }, StatusEffects::default()),
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

    spawn_staircase(commands, meshes, materials, staircase_position);
}

fn spawn_staircase(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    position: Vec2,
) {
    const SIZE: f32 = 18.0;
    const LINE_W: f32 = 2.5;
    const LINE_L: f32 = 24.0;
    const LINE_SPACING: f32 = 6.0;

    let bg_mat = materials.add(ColorMaterial::from(Color::srgb(0.25, 0.18, 0.06)));
    let line_mat = materials.add(ColorMaterial::from(Color::srgb(0.60, 0.45, 0.15)));

    let bg_mesh = meshes.add(Rectangle::new(SIZE, SIZE));
    let line_mesh = meshes.add(Rectangle::new(LINE_W, LINE_L));

    let staircase = commands
        .spawn((
            DespawnOnExit(GameState::InLevel),
            Staircase,
            Transform::from_translation(position.extend(fov::ON_FLOOR_Z)),
            Visibility::default(),
        ))
        .id();

    let bg_child = commands
        .spawn((
            DespawnOnExit(GameState::InLevel),
            Mesh2d(bg_mesh),
            MeshMaterial2d(bg_mat),
            Transform::default(),
        ))
        .id();
    commands.entity(staircase).add_child(bg_child);

    // 3 parallel hatch lines at 45°, spaced along the perpendicular (-45°) direction.
    let perp = Vec2::new(1.0, -1.0).normalize();
    for i in [-1i32, 0, 1] {
        let offset = perp * (i as f32 * LINE_SPACING);
        let line_child = commands
            .spawn((
                DespawnOnExit(GameState::InLevel),
                Mesh2d(line_mesh.clone()),
                MeshMaterial2d(line_mat.clone()),
                Transform::from_translation(offset.extend(1.0))
                    .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_4)),
            ))
            .id();
        commands.entity(staircase).add_child(line_child);
    }
}
