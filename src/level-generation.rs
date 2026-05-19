use crate::bsp::{Partition, partition_space};
use geo::{BooleanOps, LineString, MultiPolygon, Polygon, Rect, Translate};
use rand::prelude::*;

/// Space around the edge of the map
pub const MARGIN: f32 = 10.0;
/// Minimum thickness around a room
pub const PADDING: f32 = 20.0;
pub const CORRIDOR_WIDTH: f32 = 35.0;
const MIN_ROOM_SIZE: f32 = 100.0 - PADDING * 2.0;

pub struct TerrainGeometry {
    pub solid_rock: MultiPolygon<f32>,
    pub playable_area: MultiPolygon<f32>,
    pub rooms: Vec<Rect<f32>>,
    pub doors: Vec<DoorGeometry>,
}

#[derive(Clone, Copy)]
pub struct DoorGeometry {
    pub phys_rect: Rect<f32>,
    pub disp_rect: Rect<f32>,
    pub hinge: (f32, f32),
}

impl TerrainGeometry {
    pub fn new(width: f32, height: f32) -> Self {
        let mut rng = rand::thread_rng();
        Self::new_seeded(width, height, &mut rng)
    }

    pub fn new_seeded(width: f32, height: f32, rng: &mut impl Rng) -> Self {
        // The playable area bounds
        let bounds = Partition {
            x: (MARGIN, width - MARGIN),
            y: (MARGIN, height - MARGIN),
            horz_conn: (vec![], vec![]),
            vert_conn: (vec![], vec![]),
        };

        let partitions = partition_space(bounds, rng);
        let mut allocated_partitions = allocate_roles(partitions, rng);

        // Propagate double_width between adjacent corridors
        let mut changed = true;
        while changed {
            changed = false;
            for i in 0..allocated_partitions.len() {
                if let PartitionRole::Corridor { double_width: true } = allocated_partitions[i].1 {
                    for j in 0..allocated_partitions.len() {
                        if i == j {
                            continue;
                        }
                        if let PartitionRole::Corridor { double_width: false } =
                            allocated_partitions[j].1
                            && partitions_share_connection(
                                &allocated_partitions[i].0,
                                &allocated_partitions[j].0,
                            )
                        {
                            allocated_partitions[j].1 =
                                PartitionRole::Corridor { double_width: true };
                            changed = true;
                        }
                    }
                }
            }
        }

        Self::from_partitions_and_roles(width, height, allocated_partitions, rng)
    }

    pub fn from_partitions_and_roles(
        width: f32,
        height: f32,
        allocated_partitions: Vec<(Partition, PartitionRole)>,
        rng: &mut impl Rng,
    ) -> Self {
        let (rooms, playable_area, doors) = render(&allocated_partitions, rng);

        // The terrain is the bounds minus the playable area
        let earth = Rect::<f32>::new((0.0, 0.0), (width, height));
        let geometry =
            earth.to_polygon().difference(&playable_area).translate(-width / 2.0, -height / 2.0);

        let offset_x = -width / 2.0;
        let offset_y = -height / 2.0;

        let playable_area = playable_area.translate(offset_x, offset_y);

        let rooms = rooms.into_iter().map(|r| r.translate(offset_x, offset_y)).collect();

        let doors = doors
            .into_iter()
            .map(|mut d| {
                d.phys_rect = d.phys_rect.translate(offset_x, offset_y);
                d.disp_rect = d.disp_rect.translate(offset_x, offset_y);
                d.hinge.0 += offset_x;
                d.hinge.1 += offset_y;
                d
            })
            .collect();

        TerrainGeometry { solid_rock: geometry, playable_area, rooms, doors }
    }
}

const EMPTY_PROB: f32 = 0.3; // if a partition is a dead end, the probability it will be empty
const CORRIDOR_PROB: f32 = 0.45; // otherwise, the probability it will be a corridor

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PartitionRole {
    Room,
    Corridor { double_width: bool },
    Empty,
}

// Dead ends may be empty or rooms. Other partitions may be corridors or rooms.
fn allocate_roles(p: Vec<Partition>, rng: &mut impl Rng) -> Vec<(Partition, PartitionRole)> {
    p.into_iter()
        .map(|partition| {
            let connection_count = partition.connection_count();

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
                        let double_width = rng.gen_bool(0.15);
                        PartitionRole::Corridor { double_width }
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

fn union_all(base: &mut MultiPolygon<f32>, polys: Vec<Polygon<f32>>) {
    for poly in polys {
        *base = base.union(&poly);
    }
}

fn partitions_share_connection(p1: &Partition, p2: &Partition) -> bool {
    let conns1 = partition_connections(p1);
    let conns2 = partition_connections(p2);
    for c1 in &conns1 {
        for c2 in &conns2 {
            if (c1.x - c2.x).abs() < 0.1 && (c1.y - c2.y).abs() < 0.1 {
                return true;
            }
        }
    }
    false
}

fn is_double_width_corridor_connection(
    connection: &ConnectionPoint,
    bsp: &[(Partition, PartitionRole)],
) -> bool {
    for (partition, role) in bsp {
        if let PartitionRole::Corridor { double_width: true } = role {
            for conn in partition_connections(partition) {
                if (conn.x - connection.x).abs() < 0.1 && (conn.y - connection.y).abs() < 0.1 {
                    return true;
                }
            }
        }
    }
    false
}

// For rooms, shrink at least PADDING away from the edges (respecting MIN_ROOM_SIZE), adding hallways out to the edge.
// For corridors, if there are two connections, draw a straight hallway between them; otherwise, draw hallways from all connections to the center point.
// Returns rooms and a multipolygon representing passable space.
fn render(
    bsp: &[(Partition, PartitionRole)],
    rng: &mut impl Rng,
) -> (Vec<Rect<f32>>, MultiPolygon<f32>, Vec<DoorGeometry>) {
    let mut rooms = Vec::new();
    let mut playables = MultiPolygon::new(vec![]);
    let mut doors = Vec::new();

    // Avoid hallway stubs leading to nothing.
    let empty_connections: Vec<(f32, f32)> = bsp
        .iter()
        .filter(|(_, role)| matches!(role, PartitionRole::Empty))
        .flat_map(|(partition, _)| partition_connections(partition).into_iter().map(|c| (c.x, c.y)))
        .collect();

    let is_live = |c: &ConnectionPoint| {
        !empty_connections
            .iter()
            .any(|&(ex, ey)| (c.x - ex).abs() < f32::EPSILON && (c.y - ey).abs() < f32::EPSILON)
    };

    for (partition, role) in bsp {
        let mut region = MultiPolygon::new(vec![]);

        match role {
            PartitionRole::Empty => continue,
            PartitionRole::Room => {
                let room = shrink_room(partition);
                region = region.union(&room.to_polygon());
                rooms.push(room);

                for connection in partition_connections(partition).into_iter().filter(is_live) {
                    let is_double = is_double_width_corridor_connection(&connection, bsp);
                    let width = if is_double { CORRIDOR_WIDTH * 2.0 } else { CORRIDOR_WIDTH };
                    union_all(&mut region, connect_room_to_connection(&room, connection, width));

                    let door_prob = if is_double { 0.75 } else { 0.25 };
                    if rng.gen_bool(door_prob) {
                        let room_entry = match connection.side {
                            ConnectionSide::Left => {
                                (room.min().x, connection.y.clamp(room.min().y, room.max().y))
                            }
                            ConnectionSide::Right => {
                                (room.max().x, connection.y.clamp(room.min().y, room.max().y))
                            }
                            ConnectionSide::Bottom => {
                                (connection.x.clamp(room.min().x, room.max().x), room.min().y)
                            }
                            ConnectionSide::Top => {
                                (connection.x.clamp(room.min().x, room.max().x), room.max().y)
                            }
                        };

                        if is_double {
                            let (d1, d2) = create_double_door(connection.side, room_entry);
                            doors.push(d1);
                            doors.push(d2);
                        } else {
                            doors.push(create_door(connection.side, room_entry));
                        }
                    }
                }
            }
            PartitionRole::Corridor { double_width } => {
                let width = if *double_width { CORRIDOR_WIDTH * 2.0 } else { CORRIDOR_WIDTH };
                let connections: Vec<_> =
                    partition_connections(partition).into_iter().filter(is_live).collect();
                let center = partition_center(partition);

                if connections.len() == 2
                    && connections[0].side.is_vertical() != connections[1].side.is_vertical()
                {
                    union_all(&mut region, connect_adjacent(connections[0], connections[1], width));
                } else if !connections.is_empty() {
                    region = region.union(&bevel_at_point(center, width));
                    for connection in connections {
                        union_all(&mut region, connect_point_to_center(connection, center, width));
                    }
                }
            }
        }

        playables = playables.union(&region);
    }

    for door in &doors {
        let h_x = door.hinge.0;
        let h_y = door.hinge.1;
        let hinge_rect = Rect::new((h_x - 2.5, h_y - 2.5), (h_x + 2.5, h_y + 2.5));
        playables = playables.difference(&MultiPolygon::new(vec![hinge_rect.to_polygon()]));
    }

    (rooms, playables, doors)
}

#[derive(Copy, Clone)]
enum ConnectionSide {
    Left,
    Right,
    Bottom,
    Top,
}

fn make_door(
    side: ConnectionSide,
    room_entry: (f32, f32),
    thickness: f32,
    phys_span: (f32, f32),
    disp_span: (f32, f32),
    hinge_at_start: bool,
) -> DoorGeometry {
    if side.is_vertical() {
        let x0 =
            if let ConnectionSide::Left = side { room_entry.0 - thickness } else { room_entry.0 };
        let x1 = x0 + thickness;
        let hinge = (x0 + thickness / 2.0, if hinge_at_start { phys_span.0 } else { phys_span.1 });
        DoorGeometry {
            phys_rect: Rect::new((x0, phys_span.0), (x1, phys_span.1)),
            disp_rect: Rect::new((x0, disp_span.0), (x1, disp_span.1)),
            hinge,
        }
    } else {
        let y0 =
            if let ConnectionSide::Bottom = side { room_entry.1 - thickness } else { room_entry.1 };
        let y1 = y0 + thickness;
        let hinge = (if hinge_at_start { phys_span.0 } else { phys_span.1 }, y0 + thickness / 2.0);
        DoorGeometry {
            phys_rect: Rect::new((phys_span.0, y0), (phys_span.1, y1)),
            disp_rect: Rect::new((disp_span.0, y0), (disp_span.1, y1)),
            hinge,
        }
    }
}

fn create_door(side: ConnectionSide, room_entry: (f32, f32)) -> DoorGeometry {
    let thickness = 5.0;
    let phys_len = CORRIDOR_WIDTH - 4.0;
    let entry_mid = if side.is_vertical() { room_entry.1 } else { room_entry.0 };

    let phys_span = (entry_mid - phys_len / 2.0, entry_mid + phys_len / 2.0);
    let disp_span = (entry_mid - CORRIDOR_WIDTH / 2.0, entry_mid + CORRIDOR_WIDTH / 2.0);

    make_door(side, room_entry, thickness, phys_span, disp_span, true)
}

fn create_double_door(
    side: ConnectionSide,
    room_entry: (f32, f32),
) -> (DoorGeometry, DoorGeometry) {
    let thickness = 5.0;
    let w = CORRIDOR_WIDTH * 2.0;
    let entry_mid = if side.is_vertical() { room_entry.1 } else { room_entry.0 };

    // Panel 1: bottom/left panel
    let phys_span_1 = (entry_mid - (w / 2.0 - 2.0), entry_mid - 2.0);
    let disp_span_1 = (entry_mid - w / 2.0, entry_mid);
    let panel_1 = make_door(side, room_entry, thickness, phys_span_1, disp_span_1, true);

    // Panel 2: top/right panel
    let phys_span_2 = (entry_mid + 2.0, entry_mid + (w / 2.0 - 2.0));
    let disp_span_2 = (entry_mid, entry_mid + w / 2.0);
    let panel_2 = make_door(side, room_entry, thickness, phys_span_2, disp_span_2, false);

    (panel_1, panel_2)
}

impl ConnectionSide {
    fn is_vertical(&self) -> bool { matches!(self, ConnectionSide::Left | ConnectionSide::Right) }
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
        connections.push(ConnectionPoint { x: partition.x.0, y, side: ConnectionSide::Left });
    }
    for &y in &partition.horz_conn.1 {
        connections.push(ConnectionPoint { x: partition.x.1, y, side: ConnectionSide::Right });
    }
    for &x in &partition.vert_conn.0 {
        connections.push(ConnectionPoint { x, y: partition.y.0, side: ConnectionSide::Bottom });
    }
    for &x in &partition.vert_conn.1 {
        connections.push(ConnectionPoint { x, y: partition.y.1, side: ConnectionSide::Top });
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
    ((partition.x.0 + partition.x.1) / 2.0, (partition.y.0 + partition.y.1) / 2.0)
}

fn connect_room_to_connection(
    room: &Rect<f32>,
    connection: ConnectionPoint,
    width: f32,
) -> Vec<geo::Polygon<f32>> {
    let room_entry = match connection.side {
        ConnectionSide::Left => (room.min().x, connection.y.clamp(room.min().y, room.max().y)),
        ConnectionSide::Right => (room.max().x, connection.y.clamp(room.min().y, room.max().y)),
        ConnectionSide::Bottom => (connection.x.clamp(room.min().x, room.max().x), room.min().y),
        ConnectionSide::Top => (connection.x.clamp(room.min().x, room.max().x), room.max().y),
    };

    let conn_pt = (connection.x, connection.y);
    let elbow = if connection.side.is_vertical() {
        (room_entry.0, connection.y)
    } else {
        (connection.x, room_entry.1)
    };

    let mut polygons = vec![rect_for_segment(conn_pt, elbow, width)];

    let needs_bend = if connection.side.is_vertical() {
        (room_entry.1 - connection.y).abs() > f32::EPSILON
    } else {
        (room_entry.0 - connection.x).abs() > f32::EPSILON
    };
    if needs_bend {
        polygons.push(bevel_at_point(elbow, width));
        polygons.push(rect_for_segment(elbow, room_entry, width));
    }

    polygons
}

fn connect_adjacent(a: ConnectionPoint, b: ConnectionPoint, width: f32) -> Vec<Polygon<f32>> {
    let (h_conn, v_conn) = if a.side.is_vertical() { (a, b) } else { (b, a) };
    let elbow = (v_conn.x, h_conn.y);
    vec![
        rect_for_segment((h_conn.x, h_conn.y), elbow, width),
        bevel_at_point(elbow, width),
        rect_for_segment(elbow, (v_conn.x, v_conn.y), width),
    ]
}

fn connect_point_to_center(
    connection: ConnectionPoint,
    center: (f32, f32),
    width: f32,
) -> Vec<geo::Polygon<f32>> {
    let conn_pt = (connection.x, connection.y);

    if (connection.x - center.0).abs() < f32::EPSILON
        || (connection.y - center.1).abs() < f32::EPSILON
    {
        return vec![rect_for_segment(conn_pt, center, width)];
    }

    let elbow = if connection.side.is_vertical() {
        (center.0, connection.y)
    } else {
        (connection.x, center.1)
    };

    vec![
        rect_for_segment(conn_pt, elbow, width),
        bevel_at_point(elbow, width),
        rect_for_segment(elbow, center, width),
    ]
}

fn bevel_at_point(p: (f32, f32), width: f32) -> Polygon<f32> {
    let h = width / 2.0;
    let q = width / 4.0;
    Polygon::new(
        LineString::from(vec![
            (p.0 - h, p.1 - q),
            (p.0 - q, p.1 - h),
            (p.0 + q, p.1 - h),
            (p.0 + h, p.1 - q),
            (p.0 + h, p.1 + q),
            (p.0 + q, p.1 + h),
            (p.0 - q, p.1 + h),
            (p.0 - h, p.1 + q),
            (p.0 - h, p.1 - q),
        ]),
        vec![],
    )
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
        assert!(partitions.len() >= 5, "expected at least 5 partitions, got {}", partitions.len());

        let connection_total: usize = partitions.iter().map(|p| p.connection_count()).sum();

        assert!(
            connection_total >= partitions.len() * 2,
            "expected at least twice as many connections as partitions, got {} connections for {} \
             partitions",
            connection_total,
            partitions.len(),
        );
    }
}
