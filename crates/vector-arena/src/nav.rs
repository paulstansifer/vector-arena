// Bridge between Landmass pathfinding and Avian2D physics.
// Owns the navmesh types (DungeonNavMesh, NavMeshIslandMarker) and the
// function that converts playable geometry into a Landmass nav mesh.
// Also applies landmass-computed desired velocity to Avian2D LinearVelocity
// for all agents each frame, and syncs the DungeonNavMesh resource to the island entity.
use crate::{
    dungeon::terrain::{TORPOR_FACTOR, TorporMultiplier},
    status_effect::StatusEffects,
    util::safegeo::{SafeMultiPolygon, SafePolygon},
};
use avian2d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::prelude::*;
use geo::algorithm::triangulate_delaunay::{DelaunayTriangulationConfig, TriangulateDelaunay};
use std::{collections::HashMap, sync::Arc};

pub const STEERING_GAIN: f32 = 40.0;
pub const STOP_THRESHOLD: f32 = 8.0;

const NAVMESH_SNAP_RADIUS: f32 = 80.0;

/// Find the nearest point on the navmesh to `point`, within NAVMESH_SNAP_RADIUS.
/// Returns `point` unchanged if no navmesh is reachable (e.g. not yet loaded).
pub fn snap_to_navmesh(point: Vec2, archipelago_query: &Query<&Archipelago2d>) -> Vec2 {
    archipelago_query
        .iter()
        .next()
        .and_then(|a| a.sample_point(point, &NAVMESH_SNAP_RADIUS).ok())
        .map(|s| s.point())
        .unwrap_or(point)
}

#[derive(Component)]
pub struct NavMeshIslandMarker;

#[derive(Resource)]
pub struct DungeonNavMesh(pub Handle<NavMesh2d>);

/// Convert the playable area into a landmass NavigationMesh2d.
/// Erodes by AGENT_RADIUS, triangulates, and deduplicates vertices for pathfinding.
/// Torpor zones are triangulated as a separate pass (type index 1) so that zone
/// boundaries align exactly with triangle edges — no triangle straddles the boundary.
pub fn playable_area_to_nav_mesh(
    playable_area: &SafeMultiPolygon,
    torpor_zones: &[SafePolygon],
) -> Arc<ValidNavigationMesh2d> {
    // bevy_landmass::nav_mesh::bevy_mesh_to_landmass_nav_mesh might simplify this somewhat, but it doesn't seem respect agent radius, so I guess we still need to handle that ourselves.
    use geo::{
        algorithm::buffer::BufferStyle,
        buffer::{LineCap, LineJoin},
    };

    let style =
        BufferStyle::new(-crate::AGENT_RADIUS).line_cap(LineCap::Square).line_join(LineJoin::Bevel);
    let eroded = playable_area.buffer_with_style(style);

    let mut vertices: Vec<Vec2> = Vec::new();
    let mut polygons: Vec<Vec<usize>> = Vec::new();
    let mut polygon_type_indices: Vec<usize> = Vec::new();

    // Map from quantized vertex position to index, for deduplication.
    // Shared across both passes so boundary vertices between regions are merged.
    let mut vertex_map: HashMap<(i64, i64), usize> = HashMap::new();

    // Quantize to ~0.001 precision to merge near-identical vertices.
    let quantize = |x: f32, y: f32| -> (i64, i64) {
        ((x * 1000.0).round() as i64, (y * 1000.0).round() as i64)
    };

    let mut add_region = |region: &SafeMultiPolygon, type_idx: usize| {
        for polygon in region.iter() {
            let Ok(triangulation) =
                polygon.constrained_triangulation(DelaunayTriangulationConfig::default())
            else {
                continue;
            };
            for triangle in &triangulation {
                let coords = [triangle.v1(), triangle.v2(), triangle.v3()];
                let indices: Vec<usize> = coords
                    .iter()
                    .map(|c| {
                        let key = quantize(c.x, c.y);
                        *vertex_map.entry(key).or_insert_with(|| {
                            let idx = vertices.len();
                            vertices.push(Vec2::new(c.x, c.y));
                            idx
                        })
                    })
                    .collect();
                // landmass expects counter-clockwise polygons.
                // geo's constrained_triangulation produces CCW triangles already.
                polygons.push(indices);
                polygon_type_indices.push(type_idx);
            }
        }
    };

    if torpor_zones.is_empty() {
        add_region(&eroded, 0);
    } else {
        // Expand the torpor zones by AGENT_RADIUS so the high-cost navmesh region
        // starts slightly before the visual boundary. This prevents agents from getting
        // hitched on zone edges/corners due to the imprecision of physical movement.
        let expand_style = BufferStyle::new(crate::AGENT_RADIUS)
            .line_cap(LineCap::Square)
            .line_join(LineJoin::Bevel);
        let torpor_mp = SafeMultiPolygon::from_polygons(torpor_zones.iter().cloned())
            .buffer_with_style(expand_style);
        // Two-pass triangulation keeps zone boundaries as exact triangle edges,
        // so no triangle straddles the torpor/non-torpor boundary.
        add_region(&eroded.difference(&torpor_mp), 0);
        add_region(&eroded.intersection(&torpor_mp), 1);
    }

    let nav_mesh = NavigationMesh2d { vertices, polygons, polygon_type_indices, height_mesh: None };

    Arc::new(nav_mesh.validate().expect("playable area nav mesh should be valid"))
}

/// The travel-cost multiplier for navmesh polygon type 1 (torpor zones).
/// Inverse of TORPOR_FACTOR: traversing the zone costs this much more per unit distance.
pub const TORPOR_NAV_COST: f32 = 1.0 / TORPOR_FACTOR;

/// Sync the DungeonNavMesh resource to the island entity when it changes.
pub fn sync_island_nav_mesh(
    dungeon_nav_mesh: Res<DungeonNavMesh>,
    mut island_query: Query<&mut bevy_landmass::NavMeshHandle<TwoD>, With<NavMeshIslandMarker>>,
) {
    if dungeon_nav_mesh.is_changed()
        && let Ok(mut nav_mesh_handle) = island_query.single_mut()
    {
        nav_mesh_handle.0 = dungeon_nav_mesh.0.clone();
    }
}

/// Apply landmass's desired velocity as actual movement for all agents (player and monsters).
/// Uses the direction from AgentDesiredVelocity2d with the speed from AgentSettings.desired_speed,
/// then scales by StatusEffects and TorporMultiplier. Applies a cornering slowdown and stops the
/// agent when it arrives at a Point target. When off-navmesh with an active target, steers toward
/// the nearest navmesh point so pathfinding can resume.
pub fn apply_nav_velocity(
    mut agents: Query<(
        Forces,
        &Transform,
        &AgentDesiredVelocity2d,
        &AgentSettings,
        &AgentTarget2d,
        Option<&TorporMultiplier>,
        Option<&StatusEffects>,
    )>,
    archipelago_query: Query<&Archipelago2d>,
) {
    for (mut forces, transform, desired_velocity, settings, agent_target, torpor, effects) in
        agents.iter_mut()
    {
        let pos = transform.translation.truncate();
        let speed_mult = effects.map(|e| e.speed_multiplier()).unwrap_or(1.0)
            * torpor.map(|t| t.get()).unwrap_or(1.0);

        // Stop when close enough to a Point target.
        if let AgentTarget2d::Point(target) = *agent_target
            && pos.distance(target) <= STOP_THRESHOLD
        {
            let correction = Vec2::ZERO - forces.linear_velocity();
            forces.reset_accumulated_linear_acceleration();
            forces.apply_linear_acceleration(correction * STEERING_GAIN);
            continue;
        }

        let nav_vel = desired_velocity.velocity();
        let desired_dir = if nav_vel != Vec2::ZERO {
            nav_vel.normalize_or_zero()
        } else if !matches!(*agent_target, AgentTarget2d::None) {
            // Off-navmesh: steer toward nearest navmesh point so pathfinding can resume.
            let snapped = snap_to_navmesh(pos, &archipelago_query);
            (snapped - pos).normalize_or_zero()
        } else {
            Vec2::ZERO
        };

        // Reduce speed when cornering sharply; full speed from rest or when going straight.
        let current_vel = forces.linear_velocity();
        let cornering = if current_vel.length_squared() > 0.5 && desired_dir != Vec2::ZERO {
            desired_dir.dot(current_vel.normalize_or_zero()).max(0.0)
        } else {
            1.0
        };

        let desired = desired_dir * settings.desired_speed * speed_mult * cornering;
        let correction = desired - forces.linear_velocity();
        forces.reset_accumulated_linear_acceleration();
        forces.apply_linear_acceleration(correction * STEERING_GAIN);
    }
}
