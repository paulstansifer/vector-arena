use bevy::prelude::*;
use bevy_mesh::{Indices, PrimitiveTopology};
use bevy_rapier2d::prelude::*;
use geo::algorithm::triangulate_delaunay::{DelaunayTriangulationConfig, TriangulateDelaunay};
use geo::geometry::{Coord, MultiPolygon};
use geo::{BooleanOps, Rect, Translate};
use rand::prelude::*;

pub const MARGIN: f32 = 10.0;
pub const MIN_ROOM_SIZE: f32 = 120.0;
pub const CORRIDOR_WIDTH: f32 = 50.0;
pub const PADDING: f32 = 5.0;

#[derive(Component)]
pub struct Terrain;

pub struct TerrainGeometry {
    pub polygon: MultiPolygon<f32>,
    pub rooms: Vec<Rect<f32>>,
}

impl TerrainGeometry {
    /// Create terrain as a dungeon with BSP-generated rooms and corridors.
    /// Screen is assumed to be width x height centered at origin.
    pub fn new(width: f32, height: f32) -> Self {
        let mut rng = rand::thread_rng();

        // The playable area bounds
        let bounds = Rect::new(
            Coord {
                x: MARGIN,
                y: MARGIN,
            },
            Coord {
                x: width - MARGIN,
                y: height - MARGIN,
            },
        );

        // Generate rooms using BSP
        let rooms = generate_rooms(&bounds, MIN_ROOM_SIZE, &mut rng);

        // Generate corridors connecting adjacent rooms
        let corridors = generate_corridors(&rooms, CORRIDOR_WIDTH);

        // Combine rooms and corridors into playable area
        let mut playable_polygons = Vec::new();
        for room in &rooms {
            playable_polygons.push(room.to_polygon());
        }
        for corridor in &corridors {
            playable_polygons.push(corridor.to_polygon());
        }

        // Union all playable areas
        let mut playable_area = MultiPolygon::new(vec![]);
        for poly in playable_polygons {
            playable_area = playable_area.union(&poly);
        }

        // The terrain is the bounds minus the playable area
        let earth = Rect::<f32>::new(
            Coord::<f32> { x: 0.0, y: 0.0 },
            Coord::<f32> {
                x: width,
                y: height,
            },
        );
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

/// BSP tree subdivision.
fn partition_level(bounds: &Rect<f32>, min_size: f32, rng: &mut impl Rng) -> Vec<Rect<f32>> {
    let mut partitions = Vec::new();
    let mut stack = vec![*bounds];

    while let Some(rect) = stack.pop() {
        let width = rect.width();
        let height = rect.height();

        if width <= min_size * rng.gen_range(2.0..4.5)
            && height <= min_size * rng.gen_range(2.0..4.5)
        {
            // Make the room smaller than the leaf
            let room_width = rng.gen_range(min_size..width);
            let room_height = rng.gen_range(min_size..height);
            let room_x = rect.min().x + rng.gen_range(0.0..width - room_width);
            let room_y = rect.min().y + rng.gen_range(0.0..height - room_height);
            let room = Rect::new(
                Coord {

                    x: room_x + PADDING,
                    y: room_y + PADDING,
                },
                Coord {
                    x: room_x + room_width - PADDING,
                    y: room_y + room_height - PADDING,
                },
            );
            partitions.push(room);
        } else {
            // Split
            let split_horizontal = rng.gen_bool(0.5);
            if (split_horizontal && width > min_size * 2.0) || height <= min_size * 2.0 {
                // Split vertically (along x)
                let split_x = rect.min().x + rng.gen_range(min_size..width - min_size);
                let left = Rect::new(
                    rect.min(),
                    Coord {
                        x: split_x,
                        y: rect.max().y,
                    },
                );
                let right = Rect::new(
                    Coord {
                        x: split_x,
                        y: rect.min().y,
                    },
                    rect.max(),
                );
                stack.push(left);
                stack.push(right);
            } else {
                // Split horizontally (along y)
                let split_y = rect.min().y + rng.gen_range(min_size..height - min_size);
                let bottom = Rect::new(
                    rect.min(),
                    Coord {
                        x: rect.max().x,
                        y: split_y,
                    },
                );
                let top = Rect::new(
                    Coord {
                        x: rect.min().x,
                        y: split_y,
                    },
                    rect.max(),
                );
                stack.push(bottom);
                stack.push(top);
            }
        }
    }

    partitions
}

/// Generate corridors connecting adjacent rooms.
fn generate_corridors(rooms: &[Rect<f32>], width: f32) -> Vec<Rect<f32>> {
    let mut corridors = Vec::new();

    for i in 0..rooms.len() {
        for j in (i + 1)..rooms.len() {
            let room_a = rooms[i];
            let room_b = rooms[j];

            let a_min = room_a.min();
            let a_max = room_a.max();
            let b_min = room_b.min();
            let b_max = room_b.max();

            let overlap_x = a_min.x < b_max.x && a_max.x > b_min.x;
            let overlap_y = a_min.y < b_max.y && a_max.y > b_min.y;

            if (a_max.x == b_min.x || a_min.x == b_max.x) && overlap_y {
                // Adjacent horizontally, connect with vertical corridor
                let x = if a_max.x == b_min.x { a_max.x } else { a_min.x };
                let bottom = a_min.y.max(b_min.y);
                let top = a_max.y.min(b_max.y);
                let mid_y = (bottom + top) / 2.0;
                let corridor = Rect::new(
                    Coord {
                        x: x - width / 2.0,
                        y: mid_y - width / 2.0,
                    },
                    Coord {
                        x: x + width / 2.0,
                        y: mid_y + width / 2.0,
                    },
                );
                println!("h corr: {:?}", corridor);
                corridors.push(corridor);
            } else if (a_max.y == b_min.y || a_min.y == b_max.y) && overlap_x {
                // Adjacent vertically, connect with horizontal corridor
                let y = if a_max.y == b_min.y { a_max.y } else { a_min.y };
                let left = a_min.x.max(b_min.x);
                let right = a_max.x.min(b_max.x);
                let mid_x = (left + right) / 2.0;
                let corridor = Rect::new(
                    Coord {
                        x: mid_x - width / 2.0,
                        y: y - width / 2.0,
                    },
                    Coord {
                        x: mid_x + width / 2.0,
                        y: y + width / 2.0,
                    },
                );
                println!("v corr: {:?}", corridor);
                corridors.push(corridor);
            }
        }
    }

    corridors
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
