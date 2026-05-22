// Spawns the inhabitants and interactables of a dungeon level: player, monsters,
// items, and the down staircase. Called after structural elements (terrain,
// navmesh, FOV) have been put in place by `spawn_game_world`.
use avian2d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::prelude::*;
use geo::Rect;
use rand::prelude::*;

use crate::{
    AGENT_RADIUS, GameLayer, GameState, Staircase,
    effects::projectile::MonsterShootTimer,
    fov,
    item::{Inventory, Item, ItemKind, PotionColor, ScrollName, item_display_name},
    monster::{MONSTER_MAX_HP, MONSTER_SPEED, Monster, Stats},
    player::{MoveTarget, PLAYER_SPEED, Player},
    ui::WorldTooltip,
};

pub fn populate(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    rooms: &[Rect<f32>],
    archipelago_id: Entity,
    depth: u32,
    saved_player: Option<(Stats, Inventory)>,
) {
    let mut rng = rand::thread_rng();

    // Pick the player's room by index so we can exclude it when placing the staircase.
    let player_room_idx = if rooms.is_empty() { 0 } else { rng.gen_range(0..rooms.len()) };

    let player_position = rooms
        .get(player_room_idx)
        .map(|r| {
            let c = r.center();
            Vec2::new(c.x, c.y)
        })
        .unwrap_or(Vec2::ZERO);

    let staircase_position = rooms
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != player_room_idx)
        .map(|(_, r)| {
            let c = r.center();
            Vec2::new(c.x, c.y)
        })
        .collect::<Vec<_>>()
        .choose(&mut rng)
        .copied()
        .unwrap_or(player_position + Vec2::new(50.0, 0.0));

    let (initial_stats, initial_inventory) = saved_player.unwrap_or((
        Stats { hp: 100.0, max_hp: 100.0, mana: 80.0, max_mana: 80.0 },
        Inventory::default(),
    ));

    let player = commands
        .spawn((
            DespawnOnExit(GameState::InLevel),
            Player,
            initial_inventory,
            initial_stats,
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
        ))
        .id();

    let monster_mesh = meshes.add(Circle::new(AGENT_RADIUS));

    let monster_positions: Vec<Vec2> = rooms
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != player_room_idx)
        .map(|(_, r)| {
            let c = r.center();
            Vec2::new(c.x, c.y)
        })
        .collect();

    // One more monster per depth level (2 at depth 1, 3 at depth 2, …).
    let monster_count = (depth as usize + 1).min(monster_positions.len());
    for position in monster_positions.into_iter().take(monster_count) {
        commands.spawn((
            DespawnOnExit(GameState::InLevel),
            Monster,
            Stats { hp: MONSTER_MAX_HP, max_hp: MONSTER_MAX_HP, ..default() },
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
            AgentTarget2d::Entity(player),
        ));
    }

    let all_item_kinds = [
        ItemKind::Potion(PotionColor::Red),
        ItemKind::Potion(PotionColor::Green),
        ItemKind::Potion(PotionColor::Blue),
        ItemKind::Scroll(ScrollName::Readme),
        ItemKind::Scroll(ScrollName::Agents),
        ItemKind::Scroll(ScrollName::License),
    ];
    let item_count = rng.gen_range(4..=5);
    let chosen_kinds: Vec<ItemKind> =
        all_item_kinds.choose_multiple(&mut rng, item_count).copied().collect();

    let potion_mesh = meshes.add(RegularPolygon::new(7.0, 3));
    let scroll_mesh = meshes.add(Rectangle::new(12.0, 12.0));

    for kind in chosen_kinds {
        let room = rooms.choose(&mut rng).unwrap();
        let center = room.center();
        let half_w = (room.width() / 2.0 - 18.0).max(5.0);
        let half_h = (room.height() / 2.0 - 18.0).max(5.0);
        let x = center.x + rng.gen_range(-half_w..=half_w);
        let y = center.y + rng.gen_range(-half_h..=half_h);
        let pos = Vec3::new(x, y, fov::ON_FLOOR_Z);

        // Each item gets its own material so the pickup fade can be applied independently.
        match kind {
            ItemKind::Potion(_) => {
                commands.spawn((
                    DespawnOnExit(GameState::InLevel),
                    Item(kind),
                    WorldTooltip(item_display_name(kind).to_string()),
                    Mesh2d(potion_mesh.clone()),
                    MeshMaterial2d(materials.add(ColorMaterial::from(Color::srgb(0.2, 0.85, 0.3)))),
                    Transform::from_translation(pos),
                ));
            }
            ItemKind::Scroll(_) => {
                commands.spawn((
                    DespawnOnExit(GameState::InLevel),
                    Item(kind),
                    WorldTooltip(item_display_name(kind).to_string()),
                    Mesh2d(scroll_mesh.clone()),
                    MeshMaterial2d(materials.add(ColorMaterial::from(Color::srgb(0.8, 0.8, 0.75)))),
                    Transform::from_translation(pos),
                ));
            }
        }
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
