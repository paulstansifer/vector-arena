use crate::bsp::{Partition, partition_space};
use bevy::prelude::*;
use bevy_mesh::{Indices, PrimitiveTopology};
use bevy_rapier2d::prelude::*;
use geo::algorithm::triangulate_delaunay::{DelaunayTriangulationConfig, TriangulateDelaunay};
use geo::geometry::MultiPolygon;
use geo::{BooleanOps, Rect, Translate};
use rand::prelude::*;

pub const MARGIN: f32 = 10.0;
pub const PADDING: f32 = 30.0;

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

const EMPTY_PROB: f32 = 0.3; // if a partition is a dead end, the probability it will be empty
const CORRIDOR_PROB: f32 = 0.3; // otherwise, the probability it will be a corridor

enum PartitionRole {
    Room,
    Corridor,
    Empty,
}

// Dead ends may be empty or rooms. Other partitions may be corridors or rooms.
fn allocate_roles(p: Vec<Partition>, rng: &mut ThreadRng) -> Vec<(Partition, PartitionRole)> {
    p.into_iter()
        .map(|partition| {
            let horz_count = partition.horz_conn.0.len() + partition.horz_conn.1.len();
            let vert_count = partition.vert_conn.0.len() + partition.vert_conn.1.len();
            let connection_count = horz_count + vert_count;

            let role = match connection_count {
                0 => panic!("Shouldn't be possible to generate an unconnected partition"),
                1 => {
                    if rng.gen_bool((1.0 - EMPTY_PROB).into()) {
                        PartitionRole::Room
                    } else {
                        PartitionRole::Empty
                    }
                }
                2 => {
                    if rng.gen_bool(CORRIDOR_PROB.into()) {
                        PartitionRole::Corridor
                    } else {
                        PartitionRole::Room
                    }
                }
                _ => PartitionRole::Room,
            };

            (partition, role)
        })
        .collect()
}

const MIN_ROOM_SIZE: f32 = 100.0 - PADDING * 2.0;
const CORRIDOR_WIDTH: f32 = 50.0;

// For rooms, shrink at least PADDING away from the edges (respecting MIN_ROOM_SIZE), adding hallways out to the edge.
// For corridors, if there are two connections, draw a straight hallway between them; otherwise, draw hallways from all connections to the center point.
// Returns rooms and a multipolygon representing passable space.
fn render(
    bsp: &[(Partition, PartitionRole)],
    _rng: &mut ThreadRng,
) -> (Vec<Rect<f32>>, MultiPolygon<f32>) {
    let mut rooms = Vec::new();
    let mut playables = MultiPolygon::new(vec![]);

    for (partition, role) in bsp {
        let mut region = MultiPolygon::new(vec![]);

        match role {
            PartitionRole::Empty => {
                continue;
            }
            PartitionRole::Room => {
                let room = shrink_room(partition);
                let room_poly = room.to_polygon();
                region = region.union(&room_poly);
                rooms.push(room);

                for connection in partition_connections(partition) {
                    let hallway = connect_room_to_connection(&room, connection);
                    for poly in hallway {
                        region = region.union(&poly);
                    }
                }
            }
            PartitionRole::Corridor => {
                let connections = partition_connections(partition);
                if connections.len() == 2 {
                    let corridor = connect_two_points(connections[0], connections[1]);
                    for poly in corridor {
                        region = region.union(&poly);
                    }
                } else {
                    let center = partition_center(partition);
                    for connection in connections {
                        let corridor = connect_point_to_center(connection, center);
                        for poly in corridor {
                            region = region.union(&poly);
                        }
                    }
                }
            }
        }

        playables = playables.union(&region);
    }

    (rooms, playables)
}

#[derive(Copy, Clone)]
enum ConnectionSide {
    Left,
    Right,
    Bottom,
    Top,
}

#[derive(Copy, Clone)]
struct ConnectionPoint {
    x: f32,
    y: f32,
    side: ConnectionSide,
}

fn partition_connections(partition: &Partition) -> Vec<ConnectionPoint> {
    let mut connections = Vec::new();

    for &y in &partition.horz_conn.0 {
        connections.push(ConnectionPoint {
            x: partition.x.0,
            y,
            side: ConnectionSide::Left,
        });
    }
    for &y in &partition.horz_conn.1 {
        connections.push(ConnectionPoint {
            x: partition.x.1,
            y,
            side: ConnectionSide::Right,
        });
    }
    for &x in &partition.vert_conn.0 {
        connections.push(ConnectionPoint {
            x,
            y: partition.y.0,
            side: ConnectionSide::Bottom,
        });
    }
    for &x in &partition.vert_conn.1 {
        connections.push(ConnectionPoint {
            x,
            y: partition.y.1,
            side: ConnectionSide::Top,
        });
    }

    connections
}

fn shrink_room(partition: &Partition) -> Rect<f32> {
    let width = partition.x.1 - partition.x.0;
    let height = partition.y.1 - partition.y.0;

    let inner_width = (width - PADDING * 2.0).max(MIN_ROOM_SIZE);
    let inner_height = (height - PADDING * 2.0).max(MIN_ROOM_SIZE);

    let x0 = partition.x.0 + (width - inner_width) / 2.0;
    let y0 = partition.y.0 + (height - inner_height) / 2.0;
    let x1 = x0 + inner_width;
    let y1 = y0 + inner_height;

    Rect::new((x0, y0), (x1, y1))
}

fn partition_center(partition: &Partition) -> (f32, f32) {
    (
        (partition.x.0 + partition.x.1) / 2.0,
        (partition.y.0 + partition.y.1) / 2.0,
    )
}

fn connect_room_to_connection(
    room: &Rect<f32>,
    connection: ConnectionPoint,
) -> Vec<geo::Polygon<f32>> {
    let room_entry = match connection.side {
        ConnectionSide::Left => (room.min().x, connection.y.clamp(room.min().y, room.max().y)),
        ConnectionSide::Right => (room.max().x, connection.y.clamp(room.min().y, room.max().y)),
        ConnectionSide::Bottom => (connection.x.clamp(room.min().x, room.max().x), room.min().y),
        ConnectionSide::Top => (connection.x.clamp(room.min().x, room.max().x), room.max().y),
    };

    let corridor_width = CORRIDOR_WIDTH;
    let mut polygons = Vec::new();

    match connection.side {
        ConnectionSide::Left | ConnectionSide::Right => {
            polygons.push(rect_for_segment(
                (connection.x, connection.y),
                (room_entry.0, connection.y),
                corridor_width,
            ));
            if (room_entry.1 - connection.y).abs() > 0.0 {
                polygons.push(rect_for_segment(
                    (room_entry.0, connection.y),
                    room_entry,
                    corridor_width,
                ));
            }
        }
        ConnectionSide::Bottom | ConnectionSide::Top => {
            polygons.push(rect_for_segment(
                (connection.x, connection.y),
                (connection.x, room_entry.1),
                corridor_width,
            ));
            if (room_entry.0 - connection.x).abs() > 0.0 {
                polygons.push(rect_for_segment(
                    (connection.x, room_entry.1),
                    room_entry,
                    corridor_width,
                ));
            }
        }
    }

    polygons
}

fn connect_two_points(a: ConnectionPoint, b: ConnectionPoint) -> Vec<geo::Polygon<f32>> {
    if (a.x - b.x).abs() < f32::EPSILON {
        return vec![rect_for_segment((a.x, a.y), (b.x, b.y), CORRIDOR_WIDTH)];
    }
    if (a.y - b.y).abs() < f32::EPSILON {
        return vec![rect_for_segment((a.x, a.y), (b.x, b.y), CORRIDOR_WIDTH)];
    }

    let horizontal_first = (a.x - b.x).abs() >= (a.y - b.y).abs();
    if horizontal_first {
        vec![
            rect_for_segment((a.x, a.y), (b.x, a.y), CORRIDOR_WIDTH),
            rect_for_segment((b.x, a.y), (b.x, b.y), CORRIDOR_WIDTH),
        ]
    } else {
        vec![
            rect_for_segment((a.x, a.y), (a.x, b.y), CORRIDOR_WIDTH),
            rect_for_segment((a.x, b.y), (b.x, b.y), CORRIDOR_WIDTH),
        ]
    }
}

fn connect_point_to_center(
    connection: ConnectionPoint,
    center: (f32, f32),
) -> Vec<geo::Polygon<f32>> {
    if (connection.x - center.0).abs() < f32::EPSILON {
        return vec![rect_for_segment(
            (connection.x, connection.y),
            center,
            CORRIDOR_WIDTH,
        )];
    }
    if (connection.y - center.1).abs() < f32::EPSILON {
        return vec![rect_for_segment(
            (connection.x, connection.y),
            center,
            CORRIDOR_WIDTH,
        )];
    }

    let horizontal_first = (connection.x - center.0).abs() >= (connection.y - center.1).abs();
    if horizontal_first {
        vec![
            rect_for_segment(
                (connection.x, connection.y),
                (center.0, connection.y),
                CORRIDOR_WIDTH,
            ),
            rect_for_segment((center.0, connection.y), center, CORRIDOR_WIDTH),
        ]
    } else {
        vec![
            rect_for_segment(
                (connection.x, connection.y),
                (connection.x, center.1),
                CORRIDOR_WIDTH,
            ),
            rect_for_segment((connection.x, center.1), center, CORRIDOR_WIDTH),
        ]
    }
}

fn rect_for_segment(a: (f32, f32), b: (f32, f32), width: f32) -> geo::Polygon<f32> {
    if (a.0 - b.0).abs() < f32::EPSILON {
        let min_y = a.1.min(b.1);
        let max_y = a.1.max(b.1);
        let half = width / 2.0;
        Rect::new((a.0 - half, min_y), (a.0 + half, max_y)).to_polygon()
    } else if (a.1 - b.1).abs() < f32::EPSILON {
        let min_x = a.0.min(b.0);
        let max_x = a.0.max(b.0);
        let half = width / 2.0;
        Rect::new((min_x, a.1 - half), (max_x, a.1 + half)).to_polygon()
    } else {
        let min_x = a.0.min(b.0);
        let max_x = a.0.max(b.0);
        let min_y = a.1.min(b.1);
        let max_y = a.1.max(b.1);
        Rect::new((min_x, min_y), (max_x, max_y)).to_polygon()
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
