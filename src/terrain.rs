use avian2d::prelude::*;
use bevy::prelude::*;
use bevy_landmass::prelude::*;
use bevy_mesh::{Indices, PrimitiveTopology};
use geo::algorithm::triangulate_delaunay::{DelaunayTriangulationConfig, TriangulateDelaunay};
use geo::{BooleanOps, MultiPolygon};
use std::sync::Arc;

#[path = "level-generation.rs"]
pub mod level_generation;

pub use level_generation::{CORRIDOR_WIDTH, PADDING, TerrainGeometry, PartitionRole};

/// Convert the terrain geometry to a Bevy mesh for rendering.
pub fn geometry_to_mesh(geometry: &MultiPolygon<f32>) -> Mesh {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    let mut normals = Vec::new();

    for polygon in geometry.iter() {
        let triangulation = polygon
            .constrained_triangulation(DelaunayTriangulationConfig::default())
            .unwrap();
        for triangle in &triangulation {
            for coord in &[triangle.v1(), triangle.v2(), triangle.v3()] {
                indices.push(positions.len() as u32);
                positions.push([coord.x, coord.y, 0.0]);
                normals.push([0.0, 0.0, 1.0]);
            }
        }
    }

    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, Default::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Convert the terrain geometry to a Rapier2D polyline collider.
pub fn geometry_to_collider(geometry: &MultiPolygon<f32>) -> Collider {
    let mut vertices: Vec<Vec2> = Vec::new();
    let mut indices = Vec::new();

    for polygon in geometry.iter() {
        for ring in polygon.rings() {
            let start_index = vertices.len() as u32;

            // Add vertices for the exterior ring (excluding the last point which is a duplicate of the first)
            for coord in ring.coords() {
                vertices.push((coord.x, coord.y).into());
            }

            let num_vertices = ring.coords().count() as u32;
            // Create edges for the polyline (closed loop)
            for i in 0..num_vertices {
                let next = (i + 1) % num_vertices;
                indices.push([start_index + i, start_index + next]);
            }
        }
    }

    Collider::polyline(vertices, Some(indices))
}

/// Convert the playable area into a landmass NavigationMesh2d.
/// Triangulates each polygon and collects vertices/polygons for pathfinding.
pub fn playable_area_to_nav_mesh(playable_area: &MultiPolygon<f32>) -> Arc<ValidNavigationMesh2d> {
    // bevy_landmass::nav_mesh::bevy_mesh_to_landmass_nav_mesh might simplify this somewhat, but it doesn't seem respect agent radius, so I guess we still need to handle that ourselves.
    use geo::Buffer;
    use geo::algorithm::buffer::BufferStyle;
    use geo::buffer::{LineCap, LineJoin};

    let style = BufferStyle::new(-crate::AGENT_RADIUS)
        .line_cap(LineCap::Square)
        .line_join(LineJoin::Bevel);

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
        let triangulation = polygon
            .constrained_triangulation(DelaunayTriangulationConfig::default())
            .unwrap();
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

    let nav_mesh = NavigationMesh2d {
        vertices,
        polygons,
        polygon_type_indices,
        height_mesh: None,
    };

    Arc::new(
        nav_mesh
            .validate()
            .expect("playable area nav mesh should be valid"),
    )
}

#[path = "crumble-terrain.rs"]
pub mod crumble_terrain;

pub use crumble_terrain::{
    RubbleMaterial, handle_right_click_excavation, subtract_polygon_from_terrain,
};

#[derive(Component)]
pub struct TerrainMarker;

#[derive(Component)]
pub struct NavMeshIslandMarker;

#[derive(Resource)]
pub struct DungeonState {
    pub solid_rock: MultiPolygon<f32>,
    pub playable_area: MultiPolygon<f32>,
}

#[derive(Resource)]
pub struct DungeonNavMesh(pub Handle<NavMesh2d>);

#[derive(Resource)]
pub struct DungeonCollider(pub Collider);

#[derive(Resource)]
pub struct DungeonVisuals(pub Handle<Mesh>);

/// Bevy system to sync dungeon state changes from resources to entity components.
pub fn sync_dungeon_to_entities(
    dungeon_visuals: Res<DungeonVisuals>,
    dungeon_collider: Res<DungeonCollider>,
    dungeon_nav_mesh: Res<DungeonNavMesh>,
    mut terrain_query: Query<(&mut Mesh2d, &mut Collider), With<TerrainMarker>>,
    mut island_query: Query<&mut bevy_landmass::NavMeshHandle<TwoD>, With<NavMeshIslandMarker>>,
) {
    if dungeon_visuals.is_changed() {
        if let Ok((mut mesh, _)) = terrain_query.single_mut() {
            mesh.0 = dungeon_visuals.0.clone();
        }
    }
    if dungeon_collider.is_changed() {
        if let Ok((_, mut collider)) = terrain_query.single_mut() {
            *collider = dungeon_collider.0.clone();
        }
    }
    if dungeon_nav_mesh.is_changed() {
        if let Ok(mut nav_mesh_handle) = island_query.single_mut() {
            nav_mesh_handle.0 = dungeon_nav_mesh.0.clone();
        }
    }
}
