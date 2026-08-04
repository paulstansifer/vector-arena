// The player's only offensive-ish tool: a small shot that crumbles a bit of terrain when
// it hits a wall, or harmlessly knocks back and briefly slows a monster it hits. It never
// damages a monster — this game has no kill loop, only maneuvering and digging past
// obstacles to reach the amulet.
use avian2d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::NavMesh2d;

use rogue_angles::{
    GameLayer, LevelEntity,
    dungeon::terrain::{DungeonCollider, DungeonState, DungeonVisuals},
    effects::crumble_terrain::{RubbleMaterial, create_circle_polygon, subtract_polygon_from_terrain},
    fov::MOVABLE_Z,
    hud::MessageLog,
    nav::DungeonNavMesh,
    util::safegeo::SafeMultiPolygon,
};

use crate::{
    monster::Monster,
    player::{FireCooldown, Player},
    status_effect::{StatusEffect, StatusEffects},
};

pub const FIRE_COOLDOWN_SECS: f32 = 0.4;
const SHOT_SPEED: f32 = 900.0;
const SHOT_MAX_DISTANCE: f32 = 500.0;
const SHOT_RADIUS: f32 = 6.0;
/// Radius of the "small (approximate) circle" a shot carves out of terrain.
const DIG_RADIUS: f32 = 32.0;
const DIG_CIRCLE_POINTS: usize = 14;
const KNOCKBACK_SPEED: f32 = 380.0;
const SLOW_DURATION: f32 = 2.5;
const SLOW_FACTOR: f32 = 0.35;

#[derive(Resource)]
pub struct DigShotAssets {
    mesh: Handle<Mesh>,
    material: Handle<ColorMaterial>,
}

pub fn init_dig_shot_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    commands.insert_resource(DigShotAssets {
        mesh: meshes.add(Circle::new(SHOT_RADIUS)),
        material: materials.add(ColorMaterial::from(Color::srgb(0.75, 0.6, 0.3))),
    });
}

#[derive(Component)]
pub struct DigShot {
    velocity: Vec2,
    distance_traveled: f32,
}

pub fn fire_dig_shot(
    mut commands: Commands,
    window: Single<&Window>,
    camera_query: Single<(&Camera, &GlobalTransform)>,
    mouse_button_input: Res<ButtonInput<MouseButton>>,
    mut player_query: Query<(&Transform, &mut FireCooldown), With<Player>>,
    assets: Res<DigShotAssets>,
) {
    if !mouse_button_input.just_pressed(MouseButton::Right) {
        return;
    }

    let Some(cursor_position) = window.cursor_position() else { return };
    let (camera, camera_transform) = *camera_query;
    let Ok(target) = camera.viewport_to_world_2d(camera_transform, cursor_position) else {
        return;
    };

    let Ok((transform, mut cooldown)) = player_query.single_mut() else { return };
    if cooldown.0 > 0.0 {
        return;
    }
    cooldown.0 = FIRE_COOLDOWN_SECS;

    let origin = transform.translation.truncate();
    let direction = (target - origin).normalize_or(Vec2::Y);

    commands.spawn((
        LevelEntity,
        DigShot { velocity: direction * SHOT_SPEED, distance_traveled: 0.0 },
        Mesh2d(assets.mesh.clone()),
        MeshMaterial2d(assets.material.clone()),
        Transform::from_translation((origin + direction * 20.0).extend(MOVABLE_Z + 1.0)),
    ));
}

/// Advances every in-flight shot and resolves the first wall-crumble hit of the frame (at
/// most one — `subtract_polygon_from_terrain` consumes its `ResMut` terrain resources by
/// value, so a second simultaneous wall hit just resolves on the next frame instead; a shot
/// this small and this rare to double up is not worth restructuring around).
#[allow(clippy::too_many_arguments)]
pub fn update_dig_shots(
    mut commands: Commands,
    time: Res<Time>,
    spatial_query: SpatialQuery,
    mut shots: Query<(Entity, &mut Transform, &mut DigShot)>,
    monster_query: Query<(), With<Monster>>,
    mut velocity_query: Query<&mut LinearVelocity>,
    mut status_query: Query<&mut StatusEffects>,
    mut message_log: ResMut<MessageLog>,
    dungeon_state: Option<ResMut<DungeonState>>,
    dungeon_visuals: Option<ResMut<DungeonVisuals>>,
    dungeon_collider: Option<ResMut<DungeonCollider>>,
    dungeon_nav_mesh: Option<ResMut<DungeonNavMesh>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut nav_meshes: ResMut<Assets<NavMesh2d>>,
    rubble_material: Option<Res<RubbleMaterial>>,
) {
    let dt = time.delta_secs();
    let (Some(dungeon_state), Some(dungeon_visuals), Some(dungeon_collider), Some(dungeon_nav_mesh), Some(rubble_material)) =
        (dungeon_state, dungeon_visuals, dungeon_collider, dungeon_nav_mesh, rubble_material)
    else {
        return;
    };
    let mut terrain_res = Some((dungeon_state, dungeon_visuals, dungeon_collider, dungeon_nav_mesh));

    for (entity, mut transform, mut shot) in shots.iter_mut() {
        let step = shot.velocity * dt;
        transform.translation += step.extend(0.0);
        shot.distance_traveled += step.length();
        let pos = transform.translation.truncate();

        let wall_hit = !spatial_query
            .shape_intersections(
                &Collider::circle(SHOT_RADIUS),
                pos,
                0.0,
                &SpatialQueryFilter::from_mask(GameLayer::Wall),
            )
            .is_empty();

        if wall_hit && let Some((dungeon_state, dungeon_visuals, dungeon_collider, dungeon_nav_mesh)) = terrain_res.take() {
            let poly = create_circle_polygon(pos, DIG_RADIUS, DIG_CIRCLE_POINTS);
            subtract_polygon_from_terrain(
                &mut commands,
                &SafeMultiPolygon::from(poly),
                dungeon_state,
                dungeon_visuals,
                dungeon_collider,
                dungeon_nav_mesh,
                &mut meshes,
                &mut nav_meshes,
                &rubble_material.0,
                Some((pos, 120.0)),
            );
            commands.entity(entity).despawn();
            continue;
        } else if wall_hit {
            // Another shot already crumbled terrain this frame; just wait for next frame.
            continue;
        }

        let monster_hit = spatial_query
            .shape_intersections(
                &Collider::circle(SHOT_RADIUS),
                pos,
                0.0,
                &SpatialQueryFilter::from_mask(GameLayer::Dynamic),
            )
            .into_iter()
            .find(|e| monster_query.contains(*e));

        if let Some(monster) = monster_hit {
            if let Ok(mut vel) = velocity_query.get_mut(monster) {
                let dir = shot.velocity.normalize_or(Vec2::Y);
                vel.0 = dir * KNOCKBACK_SPEED;
            }
            if let Ok(mut effects) = status_query.get_mut(monster) {
                effects.add(StatusEffect::Slowed(SLOW_FACTOR), SLOW_DURATION);
            }
            message_log.push("The shot knocks a monster back and slows it.");
            commands.entity(entity).despawn();
            continue;
        }

        if shot.distance_traveled >= SHOT_MAX_DISTANCE {
            commands.entity(entity).despawn();
        }
    }
}
