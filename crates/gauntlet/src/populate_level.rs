// Spawns the inhabitants of a level: player, monsters, and the level's exit — a staircase
// on depths before FINAL_DEPTH, the amulet on FINAL_DEPTH. Called after structural elements
// (terrain, navmesh, FOV) are in place by `game::spawn_game_world`.
use avian2d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::prelude::*;
use rand::prelude::*;

use rogue_angles::{
    AGENT_RADIUS, GameLayer, LevelEntity,
    dungeon::terrain::{SlowZoneMultiplier, random_in_playable_area},
    fov::{self, ON_FLOOR_Z},
    hud::WorldTooltip,
    movement::{MovementModifiers, Viewer},
    util::safegeo::SafeMultiPolygon,
};

use crate::{
    AmuletOfWinning, FINAL_DEPTH, Staircase,
    monster::{Monster, MonsterState},
    player::{FireCooldown, MoveTarget, PLAYER_MAX_HP, PLAYER_SPEED, Player, Stats},
    status_effect::StatusEffects,
};

pub fn populate(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    playable_area: &SafeMultiPolygon,
    archipelago_id: Entity,
    depth: u32,
    saved_player_hp: Option<(f32, f32)>,
    rng: &mut impl Rng,
) {
    let random_pos =
        |rng: &mut _| random_in_playable_area(playable_area, rng).unwrap_or(Vec2::ZERO);

    let player_position = random_pos(rng);
    let exit_position = random_pos(rng);

    let (hp, max_hp) = saved_player_hp.unwrap_or((PLAYER_MAX_HP, PLAYER_MAX_HP));

    commands.spawn((
        LevelEntity,
        (
            Player,
            Viewer,
            MovementModifiers::default(),
            StatusEffects::default(),
            SlowZoneMultiplier(1.0),
            Stats { hp, max_hp },
            FireCooldown(0.0),
            MoveTarget::default(),
        ),
        Mesh2d(meshes.add(Circle::new(AGENT_RADIUS))),
        MeshMaterial2d(materials.add(ColorMaterial::from(Color::srgb(0.3, 0.5, 0.9)))),
        Transform::from_translation(player_position.extend(fov::MOVABLE_Z)),
        RigidBody::Dynamic,
        Collider::circle(AGENT_RADIUS),
        ColliderDensity(8.0),
        CollisionLayers::new(GameLayer::Dynamic, [GameLayer::Wall, GameLayer::Dynamic]),
        LockedAxes::ROTATION_LOCKED,
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
    let monster_material = materials.add(ColorMaterial::from(Color::srgb(0.85, 0.15, 0.15)));

    // One more monster per depth level (2 at depth 1, up to 6 at depth 5).
    let monster_count = (depth as usize + 1).min(6);
    for _ in 0..monster_count {
        let position = random_pos(rng);
        commands.spawn((
            LevelEntity,
            (
                Monster,
                MonsterState::Sleeping { timer: rng.gen_range(1.0..4.0) },
                MovementModifiers::default(),
                StatusEffects::default(),
                SlowZoneMultiplier(1.0),
                WorldTooltip::default(),
            ),
            Mesh2d(monster_mesh.clone()),
            MeshMaterial2d(monster_material.clone()),
            Transform::from_translation(position.extend(fov::MOVABLE_Z)),
            RigidBody::Dynamic,
            Collider::circle(AGENT_RADIUS),
            CollisionLayers::new(GameLayer::Dynamic, [GameLayer::Wall, GameLayer::Dynamic]),
            LockedAxes::ROTATION_LOCKED,
            Agent2dBundle {
                agent: Default::default(),
                settings: AgentSettings {
                    radius: AGENT_RADIUS,
                    desired_speed: 0.0,
                    max_speed: crate::monster::MONSTER_SPEED * 1.2,
                },
                archipelago_ref: ArchipelagoRef2d::new(archipelago_id),
            },
            AgentTarget2d::None,
        ));
    }

    if depth >= FINAL_DEPTH {
        commands.spawn((
            LevelEntity,
            AmuletOfWinning,
            Mesh2d(meshes.add(RegularPolygon::new(14.0, 6))),
            MeshMaterial2d(materials.add(ColorMaterial::from(Color::srgb(0.95, 0.8, 0.15)))),
            Transform::from_translation(exit_position.extend(ON_FLOOR_Z)),
        ));
    } else {
        commands.spawn((
            LevelEntity,
            Staircase,
            Mesh2d(meshes.add(Rectangle::new(22.0, 22.0))),
            MeshMaterial2d(materials.add(ColorMaterial::from(Color::srgb(0.4, 0.35, 0.3)))),
            Transform::from_translation(exit_position.extend(ON_FLOOR_Z)),
        ));
    }
}
