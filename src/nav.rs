// Bridge between Landmass pathfinding and Avian2D physics.
// Owns the navmesh types (DungeonNavMesh, NavMeshIslandMarker) and the
// function that converts playable geometry into a Landmass nav mesh.
// Also applies landmass-computed desired velocity to Avian2D LinearVelocity
// for monsters each frame, and syncs the DungeonNavMesh resource to the island entity.
use crate::player;
use avian2d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::prelude::*;
use geo::{
    MultiPolygon,
    algorithm::triangulate_delaunay::{DelaunayTriangulationConfig, TriangulateDelaunay},
};
use std::sync::Arc;

const AGENT_STEERING_GAIN: f32 = 60.0;

#[derive(Component)]
pub struct NavMeshIslandMarker;

#[derive(Resource)]
pub struct DungeonNavMesh(pub Handle<NavMesh2d>);

/// Convert the playable area into a landmass NavigationMesh2d.
/// Erodes by AGENT_RADIUS, triangulates, and deduplicates vertices for pathfinding.
pub fn playable_area_to_nav_mesh(playable_area: &MultiPolygon<f32>) -> Arc<ValidNavigationMesh2d> {
    // bevy_landmass::nav_mesh::bevy_mesh_to_landmass_nav_mesh might simplify this somewhat, but it doesn't seem respect agent radius, so I guess we still need to handle that ourselves.
    use geo::{
        Buffer,
        algorithm::buffer::BufferStyle,
        buffer::{LineCap, LineJoin},
    };

    let style =
        BufferStyle::new(-crate::AGENT_RADIUS).line_cap(LineCap::Square).line_join(LineJoin::Bevel);

    let eroded_playable_area = playable_area.buffer_with_style(style);

    let mut vertices: Vec<Vec2> = Vec::new();
    let mut polygons: Vec<Vec<usize>> = Vec::new();

    // Map from quantized vertex position to index, for deduplication.
    // This ensures shared edges between triangles are recognized as connected.
    let mut vertex_map: std::collections::HashMap<(i64, i64), usize> =
        std::collections::HashMap::new();

    let quantize = |x: f32, y: f32| -> (i64, i64) {
        // Quantize to ~0.001 precision to merge near-identical vertices
        ((x * 1000.0).round() as i64, (y * 1000.0).round() as i64)
    };

    let mut get_or_insert_vertex = |x: f32, y: f32| -> usize {
        let key = quantize(x, y);
        if let Some(&idx) = vertex_map.get(&key) {
            idx
        } else {
            let idx = vertices.len();
            vertices.push(Vec2::new(x, y));
            vertex_map.insert(key, idx);
            idx
        }
    };

    for polygon in eroded_playable_area.iter() {
        let triangulation =
            polygon.constrained_triangulation(DelaunayTriangulationConfig::default()).unwrap();
        for triangle in &triangulation {
            let v1 = triangle.v1();
            let v2 = triangle.v2();
            let v3 = triangle.v3();

            let i0 = get_or_insert_vertex(v1.x, v1.y);
            let i1 = get_or_insert_vertex(v2.x, v2.y);
            let i2 = get_or_insert_vertex(v3.x, v3.y);

            // landmass expects counter-clockwise polygons.
            // geo's constrained_triangulation produces CCW triangles already.
            polygons.push(vec![i0, i1, i2]);
        }
    }

    let polygon_type_indices = vec![0; polygons.len()];

    let nav_mesh = NavigationMesh2d { vertices, polygons, polygon_type_indices, height_mesh: None };

    // TODO: validate() sometimes fails (try destroying a lot of terrain)
    Arc::new(nav_mesh.validate().expect("playable area nav mesh should be valid"))
}

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
    mut agents: Query<(Forces, &AgentDesiredVelocity2d), Without<player::Player>>,
) {
    for (mut forces, desired_velocity) in agents.iter_mut() {
        let correction = desired_velocity.velocity() - forces.linear_velocity();
        forces.reset_accumulated_linear_acceleration();
        forces.apply_linear_acceleration(correction * AGENT_STEERING_GAIN);
    }
}
