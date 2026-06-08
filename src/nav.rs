// Bridge between Landmass pathfinding and Avian2D physics.
// Owns the navmesh types (DungeonNavMesh, NavMeshIslandMarker) and the
// function that converts playable geometry into a Landmass nav mesh.
// Also applies landmass-computed desired velocity to Avian2D LinearVelocity
// for monsters each frame, and syncs the DungeonNavMesh resource to the island entity.
use crate::{dungeon::terrain::{TORPOR_FACTOR, TorporMultiplier}, player};
use avian2d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::prelude::*;
use geo::{
    BooleanOps, MultiPolygon, Polygon,
    algorithm::triangulate_delaunay::{DelaunayTriangulationConfig, TriangulateDelaunay},
};
use std::{collections::HashMap, sync::Arc};

const AGENT_STEERING_GAIN: f32 = 60.0;

#[derive(Component)]
pub struct NavMeshIslandMarker;

#[derive(Resource)]
pub struct DungeonNavMesh(pub Handle<NavMesh2d>);

/// Convert the playable area into a landmass NavigationMesh2d.
/// Erodes by AGENT_RADIUS, triangulates, and deduplicates vertices for pathfinding.
/// Torpor zones are triangulated as a separate pass (type index 1) so that zone
/// boundaries align exactly with triangle edges — no triangle straddles the boundary.
pub fn playable_area_to_nav_mesh(
    playable_area: &MultiPolygon<f32>,
    torpor_zones: &[Polygon<f32>],
) -> Arc<ValidNavigationMesh2d> {
    // bevy_landmass::nav_mesh::bevy_mesh_to_landmass_nav_mesh might simplify this somewhat, but it doesn't seem respect agent radius, so I guess we still need to handle that ourselves.
    use geo::{
        Buffer,
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

    let mut add_region = |region: &MultiPolygon<f32>, type_idx: usize| {
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
        let torpor_mp =
            MultiPolygon::new(torpor_zones.to_vec()).buffer_with_style(expand_style);
        // Two-pass triangulation keeps zone boundaries as exact triangle edges,
        // so no triangle straddles the torpor/non-torpor boundary.
        add_region(&eroded.difference(&torpor_mp), 0);
        add_region(&eroded.intersection(&torpor_mp), 1);
    }

    let nav_mesh = NavigationMesh2d { vertices, polygons, polygon_type_indices, height_mesh: None };

    // TODO: validate() sometimes fails (try destroying a lot of terrain)
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
    if dungeon_nav_mesh.is_changed() {
        if let Ok(mut nav_mesh_handle) = island_query.single_mut() {
            nav_mesh_handle.0 = dungeon_nav_mesh.0.clone();
        }
    }
}

/// Apply landmass's desired velocity as actual movement on agents.
pub fn apply_agent_velocity(
    mut agents: Query<
        (Forces, &AgentDesiredVelocity2d, Option<&TorporMultiplier>),
        Without<player::Player>,
    >,
) {
    for (mut forces, desired_velocity, torpor) in agents.iter_mut() {
        let torpor_mult = torpor.map(|t| t.get()).unwrap_or(1.0);
        let desired = desired_velocity.velocity() * torpor_mult;
        let correction = desired - forces.linear_velocity();
        forces.reset_accumulated_linear_acceleration();
        forces.apply_linear_acceleration(correction * AGENT_STEERING_GAIN);
    }
}
