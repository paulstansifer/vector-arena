use crate::bsp::{Partition, partition_space};
use bevy::prelude::*;
use bevy_mesh::{Indices, PrimitiveTopology};
use bevy_rapier2d::prelude::*;
use geo::algorithm::triangulate_delaunay::{DelaunayTriangulationConfig, TriangulateDelaunay};
use geo::geometry::{Coord, MultiPolygon};
use geo::{BooleanOps, Rect, Translate};
use rand::prelude::*;

pub const MARGIN: f32 = 10.0;
pub const PADDING: f32 = 10.0;

#[derive(Component)]
pub struct Terrain;

pub struct TerrainGeometry {
    pub polygon: MultiPolygon<f32>,
    pub rooms: Vec<Rect<f32>>,
}

impl TerrainGeometry {
    pub fn new(width: f32, height: f32) -> Self {
        let mut rng = rand::thread_rng();

        // The playable area bounds
        let bounds = Partition {
            x: (MARGIN, width - MARGIN),
            y: (MARGIN, height - MARGIN),
            horz_conn: (vec![], vec![]),
            vert_conn: (vec![], vec![]),
        };

        let partitions = partition_space(bounds, &mut rng);
        let allocated_patitions = allocate_roles(partitions, &mut rng);

        let (rooms, playable_area) = render(&allocated_patitions, &mut rng);

        // The terrain is the bounds minus the playable area
        let earth = Rect::<f32>::new((0.0, 0.0), (width, height));
        let geometry = earth
            .to_polygon()
            .difference(&playable_area)
            .translate(-width / 2.0, -height / 2.0);

        let offset_x = -width / 2.0;
        let offset_y = -height / 2.0;
        let rooms = rooms
            .into_iter()
            .map(|r| r.translate(offset_x, offset_y))
            .collect();

        TerrainGeometry {
            polygon: geometry,
            rooms,
        }
    }
}

// if a partition is a dead end, the probability it will be empty
const EMPTY_PROB: f32 = 0.3;
const CORRIDOR_PROB: f32 = 0.3;

enum PartitionRole {
    Room,
    Corridor,
    Empty,
}

// Dead ends may be empty or rooms. Other partitions may be corridors or rooms.
fn allocate_roles(p: Vec<Partition>, rng: &mut ThreadRng) -> Vec<(Partition, PartitionRole)> {
    todo!()
}

const MIN_ROOM_SIZE: f32 = 100.0 - PADDING * 2.0;
const CORRIDOR_WIDTH: f32 = 50.0;

// For rooms, shrink at least PADDING away from the edges (respecting MIN_ROOM_SIZE), adding hallways out to the edge.
// For corridors, if there are two connections, draw a straight hallway between them; otherwise, draw hallways from all connections to the center point.
// Returns rooms and a multipolygon representing passable space.
fn render(
    bsp: &[(Partition, PartitionRole)],
    rng: &mut ThreadRng,
) -> (Vec<Rect<f32>>, MultiPolygon<f32>) {
    todo!()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partition_space_large_space_has_multiple_partitions_and_connections() {
        let mut rng = rand::thread_rng();
        let bounds = Partition {
            x: (0.0, 1600.0),
            y: (0.0, 1200.0),
            horz_conn: (Vec::new(), Vec::new()),
            vert_conn: (Vec::new(), Vec::new()),
        };

        let partitions = partition_space(bounds, &mut rng);
        assert!(
            partitions.len() >= 5,
            "expected at least 5 partitions, got {}",
            partitions.len()
        );

        let connection_total: usize = partitions
            .iter()
            .map(|p| {
                p.horz_conn.0.len()
                    + p.horz_conn.1.len()
                    + p.vert_conn.0.len()
                    + p.vert_conn.1.len()
            })
            .sum();

        assert!(
            connection_total >= partitions.len() * 2,
            "expected at least twice as many connections as partitions, got {} connections for {} partitions",
            connection_total,
            partitions.len(),
        );
    }
}
