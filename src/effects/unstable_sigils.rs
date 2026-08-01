// Unstable sigil: a hazardous object that explodes on contact.
// Displayed as a churning real-time spline tangle: 8 control points
// random-walking around a circle of radius 30, connected by cubic Hermite
// curves with per-point fixed tangents, rendered as a 2 px-wide black ribbon.
// Explodes when touched by a player, monster, or magic missile:
//   – carves terrain in radius 75 (with rubble)
//   – deals 10 hp flat damage to everything in radius 75
//   – pushes everything in radius 100 radially outward
use std::f32::consts::TAU;

use crate::util::safegeo::SafeMultiPolygon;
use avian2d::prelude::*;
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};
use bevy_landmass::prelude::NavMesh2d;
use geo::{Coord, Intersects, Rect};
use rand::Rng;

use crate::{
    GameLayer, LevelEntity,
    command_palette::LetterMap,
    dungeon::terrain::{DungeonCollider, DungeonState, DungeonVisuals},
    effects::crumble_terrain::{
        Fragile, RubbleMaterial, create_circle_polygon, subtract_polygon_from_terrain,
    },
    fov,
    item::{Item, item_name},
    monster::{Monster, MonsterDrop, Stats},
    nav::DungeonNavMesh,
    player::Player,
    sprite::{SvgSprite, sprite_spec},
    ui::{MessageLog, WorldTooltip},
};

const SIGIL_POINT_RADIUS: f32 = 12.0;
const SIGIL_COLLIDER_RADIUS: f32 = 20.0;
const SIGIL_DAMAGE_RADIUS: f32 = 75.0;
const SIGIL_PUSHBACK_RADIUS: f32 = 100.0;
const SIGIL_DAMAGE: f32 = 10.0;
const EXPLOSION_PUSHBACK_SPEED: f32 = 800.0;
const RUBBLE_EXPLOSION_SPEED: f32 = 300.0;
// Scale applied to fixed tangents; controls curviness of each spline segment.
const TENSION: f32 = 40.0;
const SAMPLES_PER_SEG: usize = 10;
const NUM_PTS: usize = 8;
const TOTAL_SAMPLES: usize = NUM_PTS * SAMPLES_PER_SEG;
const EXPLOSION_ANIM_DURATION: f32 = 0.3;
const EXPLOSION_ANIM_RADIUS: f32 = SIGIL_PUSHBACK_RADIUS;
// Opacity starts fading at 70% through the animation (last 30% ramps to invisible).
const EXPLOSION_FADE_START: f32 = 0.7;

#[derive(Component)]
pub struct UnstableSignil {
    angles: [f32; NUM_PTS],
    angle_vels: [f32; NUM_PTS],
    tangents: [Vec2; NUM_PTS],
    mesh_handle: Handle<Mesh>,
    triggered: bool,
}

/// Short-lived entity spawned at explosion time: plays the outward-expansion
/// animation and then despawns when `elapsed >= EXPLOSION_ANIM_DURATION`.
#[derive(Component)]
pub struct SigilExplosionAnim {
    start_angles: [f32; NUM_PTS],
    tangents: [Vec2; NUM_PTS],
    elapsed: f32,
    mesh_handle: Handle<Mesh>,
    mat_handle: Handle<ColorMaterial>,
}

pub fn spawn_unstable_sigil(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    position: Vec2,
    rng: &mut impl rand::Rng,
) {
    let mut angles = [0.0f32; NUM_PTS];
    let mut angle_vels = [0.0f32; NUM_PTS];
    let mut tangents = [Vec2::ZERO; NUM_PTS];

    for i in 0..NUM_PTS {
        angles[i] = (i as f32) * TAU / (NUM_PTS as f32);
        angle_vels[i] = rng.gen_range(-0.3f32..0.3);
        let a: f32 = rng.gen_range(0.0..TAU);
        tangents[i] = Vec2::new(a.cos(), a.sin());
    }

    let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    let mesh_handle = meshes.add(mesh);

    commands.spawn((
        LevelEntity,
        UnstableSignil {
            angles,
            angle_vels,
            tangents,
            mesh_handle: mesh_handle.clone(),
            triggered: false,
        },
        Mesh2d(mesh_handle),
        MeshMaterial2d(materials.add(ColorMaterial::from(Color::BLACK))),
        Transform::from_translation(position.extend(fov::MOVABLE_Z)),
        RigidBody::Static,
        Collider::circle(SIGIL_COLLIDER_RADIUS),
        CollisionLayers::new(GameLayer::Dynamic, [GameLayer::Wall, GameLayer::Dynamic]),
    ));
}

fn hermite(p0: Vec2, m0: Vec2, p1: Vec2, m1: Vec2, t: f32) -> Vec2 {
    let t2 = t * t;
    let t3 = t2 * t;
    (2.0 * t3 - 3.0 * t2 + 1.0) * p0
        + (t3 - 2.0 * t2 + t) * m0
        + (-2.0 * t3 + 3.0 * t2) * p1
        + (t3 - t2) * m1
}

/// Advances the random walk on each untriggered sigil and rebuilds its spline
/// ribbon mesh from the updated control-point positions.  Uses real time so the
/// animation runs at the same speed regardless of the in-game time scale.
pub fn animate_sigil(
    mut sigil_query: Query<&mut UnstableSignil>,
    mut meshes: ResMut<Assets<Mesh>>,
    time: Res<Time<Real>>,
) {
    let dt = time.delta_secs();
    let mut rng = rand::thread_rng();

    for mut sigil in sigil_query.iter_mut() {
        if sigil.triggered {
            continue;
        }

        for i in 0..NUM_PTS {
            sigil.angle_vels[i] += rng.gen_range(-1.5f32..1.5) * dt;
            sigil.angle_vels[i] = sigil.angle_vels[i].clamp(-2.0, 2.0);
            sigil.angles[i] += sigil.angle_vels[i] * dt;
        }

        let pts: [Vec2; NUM_PTS] = std::array::from_fn(|i| {
            Vec2::new(
                SIGIL_POINT_RADIUS * sigil.angles[i].cos(),
                SIGIL_POINT_RADIUS * sigil.angles[i].sin(),
            )
        });

        // Sample closed cubic Hermite spline: 8 segments × 10 = 80 points.
        let mut samples = Vec::with_capacity(TOTAL_SAMPLES);
        for i in 0..NUM_PTS {
            let j = (i + 1) % NUM_PTS;
            let m0 = sigil.tangents[i] * TENSION;
            let m1 = sigil.tangents[j] * TENSION;
            for s in 0..SAMPLES_PER_SEG {
                let t = s as f32 / SAMPLES_PER_SEG as f32;
                samples.push(hermite(pts[i], m0, pts[j], m1, t));
            }
        }

        // Extrude the sampled path into a 2 px-wide ribbon (±1 px perpendicular).
        let n = TOTAL_SAMPLES;
        let mut positions: Vec<[f32; 3]> = Vec::with_capacity(n * 2);
        let mut indices: Vec<u32> = Vec::with_capacity(n * 6);

        for i in 0..n {
            let fwd = {
                let raw = samples[(i + 1) % n] - samples[i];
                if raw == Vec2::ZERO { Vec2::X } else { raw.normalize() }
            };
            let perp = Vec2::new(-fwd.y, fwd.x);
            let s = samples[i];
            positions.push([s.x + perp.x, s.y + perp.y, 0.0]);
            positions.push([s.x - perp.x, s.y - perp.y, 0.0]);
        }

        for i in 0..n {
            let j = (i + 1) % n;
            let lc = (2 * i) as u32;
            let rc = (2 * i + 1) as u32;
            let ln = (2 * j) as u32;
            let rn = (2 * j + 1) as u32;
            indices.extend_from_slice(&[lc, rc, ln, rc, rn, ln]);
        }

        if let Some(mesh) = meshes.get_mut(&sigil.mesh_handle) {
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
            mesh.insert_indices(Indices::U32(indices));
        }
    }
}

/// Marks untriggered sigils for explosion when a player, monster, or magic
/// missile enters their collision radius.  Uses manual spatial queries rather
/// than Avian collision events to avoid adding a new collision layer.
pub fn detect_sigil_contact(
    mut sigil_query: Query<(Entity, &Transform, &mut UnstableSignil)>,
    spatial_query: SpatialQuery,
) {
    for (sigil_entity, transform, mut sigil) in sigil_query.iter_mut() {
        if sigil.triggered {
            continue;
        }
        let pos = transform.translation.truncate();

        // Physical contact: player or monster overlaps the sigil's body.
        let dynamic_hits = spatial_query.shape_intersections(
            &Collider::circle(SIGIL_COLLIDER_RADIUS),
            pos,
            0.0,
            &SpatialQueryFilter::from_mask(GameLayer::Dynamic),
        );
        if dynamic_hits.iter().any(|&e| e != sigil_entity) {
            sigil.triggered = true;
            continue;
        }

        // Missile contact: a magic missile overlaps a slightly expanded radius.
        let missile_hits = spatial_query.shape_intersections(
            &Collider::circle(SIGIL_COLLIDER_RADIUS + 2.0),
            pos,
            0.0,
            &SpatialQueryFilter::from_mask(GameLayer::Missile),
        );
        if !missile_hits.is_empty() {
            sigil.triggered = true;
        }
    }
}

/// Processes all triggered sigils in one frame: carves terrain once via a
/// combined polygon, then applies per-sigil damage and pushback, and despawns.
#[allow(clippy::too_many_arguments)]
pub fn explode_sigil(
    mut commands: Commands,
    sigil_query: Query<(Entity, &Transform, &UnstableSignil)>,
    spatial_query: SpatialQuery,
    mut dynamic_query: Query<(
        &Transform,
        Option<&mut Stats>,
        Option<&mut LinearVelocity>,
        Has<Player>,
        Has<Monster>,
        Option<&MonsterDrop>,
    )>,
    dungeon_state: Option<ResMut<DungeonState>>,
    dungeon_visuals: Option<ResMut<DungeonVisuals>>,
    dungeon_collider: Option<ResMut<DungeonCollider>>,
    dungeon_nav_mesh: Option<ResMut<DungeonNavMesh>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut nav_meshes: ResMut<Assets<NavMesh2d>>,
    rubble_material: Option<Res<RubbleMaterial>>,
    fragile_query: Query<(Entity, &ColliderAabb), With<Fragile>>,
    mut message_log: ResMut<MessageLog>,
    mut monster_letters: ResMut<LetterMap>,
    mut boredom: Option<ResMut<crate::player::Boredom>>,
) {
    let triggered: Vec<(Entity, Vec2, [f32; NUM_PTS], [Vec2; NUM_PTS])> = sigil_query
        .iter()
        .filter(|(_, _, s)| s.triggered)
        .map(|(e, tf, s)| (e, tf.translation.truncate(), s.angles, s.tangents))
        .collect();

    if triggered.is_empty() {
        return;
    }

    if let Some(ref mut b) = boredom {
        b.reduce(5.0);
        message_log.push("The unstable sigil detonates! (Cool! -5 boredom)");
    } else {
        message_log.push("The unstable sigil detonates!");
    }

    // --- Terrain crumble (one combined call for all triggered sigils) ---
    if let (Some(ds), Some(dv), Some(dc), Some(dn), Some(rm)) =
        (dungeon_state, dungeon_visuals, dungeon_collider, dungeon_nav_mesh, rubble_material)
    {
        let polys: Vec<_> = triggered
            .iter()
            .map(|(_, pos, _, _)| create_circle_polygon(*pos, SIGIL_DAMAGE_RADIUS, 16))
            .collect();
        let combined = SafeMultiPolygon::from_polygons(polys);

        for (fragile_entity, aabb) in &fragile_query {
            let rect_poly = Rect::new(Coord { x: aabb.min.x, y: aabb.min.y }, Coord {
                x: aabb.max.x,
                y: aabb.max.y,
            })
            .to_polygon();
            if combined.intersects(&rect_poly) {
                commands.entity(fragile_entity).despawn();
            }
        }

        let blast_center =
            triggered.iter().map(|(_, pos, _, _)| *pos).fold(Vec2::ZERO, |acc, p| acc + p)
                / triggered.len() as f32;
        subtract_polygon_from_terrain(
            &mut commands,
            &combined,
            ds,
            dv,
            dc,
            dn,
            &mut meshes,
            &mut nav_meshes,
            &rm.0,
            Some((blast_center, RUBBLE_EXPLOSION_SPEED)),
        );
    }

    // --- Damage and pushback (per-sigil so direction is correct) ---
    let mut player_damaged = false;

    for (sigil_entity, pos, _, _) in &triggered {
        let (sigil_entity, pos) = (*sigil_entity, *pos);

        let pushback_entities = spatial_query.shape_intersections(
            &Collider::circle(SIGIL_PUSHBACK_RADIUS),
            pos,
            0.0,
            &SpatialQueryFilter::from_mask(GameLayer::Dynamic),
        );

        for entity in pushback_entities {
            if entity == sigil_entity {
                continue;
            }

            // Read position with an immutable borrow, then drop before get_mut.
            let ent_pos = match dynamic_query.get(entity) {
                Ok((tf, ..)) => tf.translation.truncate(),
                Err(_) => continue,
            };
            let dist = ent_pos.distance(pos);
            let dir = (ent_pos - pos).normalize_or_zero();

            let Ok((_, stats_opt, vel_opt, is_player, is_monster, drop_opt)) =
                dynamic_query.get_mut(entity)
            else {
                continue;
            };

            if let Some(mut vel) = vel_opt {
                vel.0 += dir * EXPLOSION_PUSHBACK_SPEED;
            }

            if dist <= SIGIL_DAMAGE_RADIUS
                && let Some(mut stats) = stats_opt
            {
                let was_alive = stats.hp > 0.0;

                if is_player {
                    player_damaged = true;
                    crate::player::deal_damage_to_player(&mut stats, SIGIL_DAMAGE);
                } else {
                    stats.hp -= SIGIL_DAMAGE;
                }

                if is_monster && was_alive && stats.hp <= 0.0 {
                    if let Some(drop) = drop_opt {
                        let kind = drop.0;
                        let (svg_path, param) = sprite_spec(kind);
                        commands.spawn((
                            LevelEntity,
                            Item(kind),
                            WorldTooltip(item_name(kind, 1).to_string()),
                            SvgSprite { svg_path: svg_path.into(), param: Some(param) },
                            Transform::from_translation(ent_pos.extend(fov::ON_FLOOR_Z))
                                .with_scale(Vec3::splat(0.4)),
                        ));
                    }
                    monster_letters.release_monster(entity);
                    commands.entity(entity).despawn();
                }
            }
        }
    }

    if player_damaged {
        message_log.push("You are caught in the explosion!");
    }

    for (sigil_entity, pos, start_angles, tangents) in &triggered {
        // Spawn the outward-expansion visual before despawning the sigil.
        let exp_mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        let exp_mesh_handle = meshes.add(exp_mesh);
        let exp_mat_handle = materials.add(ColorMaterial::from(Color::BLACK));
        commands.spawn((
            LevelEntity,
            SigilExplosionAnim {
                start_angles: *start_angles,
                tangents: *tangents,
                elapsed: 0.0,
                mesh_handle: exp_mesh_handle.clone(),
                mat_handle: exp_mat_handle.clone(),
            },
            Mesh2d(exp_mesh_handle),
            MeshMaterial2d(exp_mat_handle),
            Transform::from_translation(pos.extend(fov::MOVABLE_Z + 0.1)),
        ));

        commands.entity(*sigil_entity).despawn();
    }
}

/// Ticks each explosion animation: lerps the 8 control points outward to an
/// evenly-spaced circle of radius 100, fades the ribbon to invisible over the
/// last 30% of the 0.3 s duration, then despawns.
pub fn tick_sigil_explosions(
    mut commands: Commands,
    mut query: Query<(Entity, &mut SigilExplosionAnim)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    time: Res<Time<Real>>,
) {
    let dt = time.delta_secs();

    for (entity, mut anim) in query.iter_mut() {
        anim.elapsed += dt;
        let t = (anim.elapsed / EXPLOSION_ANIM_DURATION).min(1.0);

        // Lerp each control point from its snapshot position to the evenly-spaced
        // target on the pushback circle.
        let pts: [Vec2; NUM_PTS] = std::array::from_fn(|i| {
            let start = Vec2::new(
                SIGIL_POINT_RADIUS * anim.start_angles[i].cos(),
                SIGIL_POINT_RADIUS * anim.start_angles[i].sin(),
            );
            let target_angle = (i as f32) * TAU / (NUM_PTS as f32);
            let end = Vec2::new(
                EXPLOSION_ANIM_RADIUS * target_angle.cos(),
                EXPLOSION_ANIM_RADIUS * target_angle.sin(),
            );
            start.lerp(end, t)
        });

        // Rebuild the spline ribbon at the lerped positions.
        let mut samples = Vec::with_capacity(TOTAL_SAMPLES);
        for i in 0..NUM_PTS {
            let j = (i + 1) % NUM_PTS;
            let m0 = anim.tangents[i] * TENSION;
            let m1 = anim.tangents[j] * TENSION;
            for s in 0..SAMPLES_PER_SEG {
                let t_s = s as f32 / SAMPLES_PER_SEG as f32;
                samples.push(hermite(pts[i], m0, pts[j], m1, t_s));
            }
        }

        let n = TOTAL_SAMPLES;
        let mut positions: Vec<[f32; 3]> = Vec::with_capacity(n * 2);
        let mut indices: Vec<u32> = Vec::with_capacity(n * 6);

        for i in 0..n {
            let fwd = {
                let raw = samples[(i + 1) % n] - samples[i];
                if raw == Vec2::ZERO { Vec2::X } else { raw.normalize() }
            };
            let perp = Vec2::new(-fwd.y, fwd.x);
            let s = samples[i];
            positions.push([s.x + perp.x, s.y + perp.y, 0.0]);
            positions.push([s.x - perp.x, s.y - perp.y, 0.0]);
        }

        for i in 0..n {
            let j = (i + 1) % n;
            let lc = (2 * i) as u32;
            let rc = (2 * i + 1) as u32;
            let ln = (2 * j) as u32;
            let rn = (2 * j + 1) as u32;
            indices.extend_from_slice(&[lc, rc, ln, rc, rn, ln]);
        }

        if let Some(mesh) = meshes.get_mut(&anim.mesh_handle) {
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
            mesh.insert_indices(Indices::U32(indices));
        }

        // Fade out over the last 30%.
        let alpha = if t <= EXPLOSION_FADE_START {
            1.0f32
        } else {
            1.0 - (t - EXPLOSION_FADE_START) / (1.0 - EXPLOSION_FADE_START)
        };
        if let Some(mat) = materials.get_mut(&anim.mat_handle) {
            mat.color = Color::srgba(0.0, 0.0, 0.0, alpha);
        }

        if anim.elapsed >= EXPLOSION_ANIM_DURATION {
            commands.entity(entity).despawn();
        }
    }
}
