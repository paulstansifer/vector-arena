use bevy::prelude::*;
use bevy_mesh::{Indices, PrimitiveTopology};
use bevy_rapier2d::prelude::*;
use geo::algorithm::triangulate_delaunay::{DelaunayTriangulationConfig, TriangulateDelaunay};
use geo::geometry::{Coord, MultiPolygon};
use geo::{BooleanOps, Rect, Translate};

pub const MARGIN: f32 = 10.0;

#[derive(Component)]
pub struct Terrain;

pub struct TerrainGeometry {
    pub polygon: MultiPolygon<f32>,
}

impl TerrainGeometry {
    /// Create terrain as a rectangle inset by MARGIN pixels from screen edges.
    /// Screen is assumed to be width x height centered at origin.
    pub fn new(width: f32, height: f32) -> Self {
        let earth = Rect::<f32>::new(
            Coord::<f32> { x: 0.0, y: 0.0 },
            Coord::<f32> {
                x: width,
                y: height,
            },
        );

        // Create a rectangle polygon for the terrain bounds
        let room = Rect::new(
            Coord {
                x: MARGIN,
                y: MARGIN,
            },
            Coord {
                x: width - MARGIN,
                y: height - MARGIN,
            },
        );

        let geometry = earth
            .to_polygon()
            .difference(&room.to_polygon())
            .translate(-width / 2.0, -height / 2.0);

        TerrainGeometry { polygon: geometry }
    }
}

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
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    for polygon in geometry.iter() {
        for ring in polygon.rings() {
            let start_index = vertices.len() as u32;

            // Add vertices for the exterior ring (excluding the last point which is a duplicate of the first)
            for coord in ring.coords() {
                vertices.push(Vect::new(coord.x, coord.y));
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
