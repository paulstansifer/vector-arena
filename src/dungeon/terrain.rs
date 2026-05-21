// Bridges TerrainGeometry into Bevy/Avian2D entities.
// `geometry_to_mesh` triangulates polygons into a Bevy Mesh.
// `geometry_to_collider` turns polygon rings into an Avian2D polyline Collider.
// `sync_dungeon_to_entities` is a system that rebuilds both whenever DungeonState
// changes (e.g. after excavation). Navmesh types and conversion live in `nav`.
use avian2d::prelude::*;
use bevy::prelude::*;
use bevy_mesh::{Indices, PrimitiveTopology};
use geo::{
    BooleanOps, MultiPolygon,
    algorithm::triangulate_delaunay::{DelaunayTriangulationConfig, TriangulateDelaunay},
};

/// Convert the terrain geometry to a Bevy mesh for rendering.
pub fn geometry_to_mesh(geometry: &MultiPolygon<f32>) -> Mesh {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    let mut normals = Vec::new();

    for polygon in geometry.iter() {
        // TODO: This `.expect()` has failed before! Figure out why this can fail!
        let triangulation = polygon
            .constrained_triangulation(DelaunayTriangulationConfig::default())
            .expect("generating visual terrain mesh");
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

#[derive(Component)]
pub struct TerrainMarker;

#[derive(Resource)]
pub struct DungeonState {
    pub solid_rock: MultiPolygon<f32>,
    pub playable_area: MultiPolygon<f32>,
}

#[derive(Resource)]
pub struct DungeonCollider(pub Collider);

#[derive(Resource)]
pub struct DungeonVisuals(pub Handle<Mesh>);

/// Bevy system to sync dungeon visuals and collider from resources to entity components.
pub fn sync_dungeon_to_entities(
    dungeon_visuals: Res<DungeonVisuals>,
    dungeon_collider: Res<DungeonCollider>,
    mut terrain_query: Query<(&mut Mesh2d, &mut Collider), With<TerrainMarker>>,
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
}
