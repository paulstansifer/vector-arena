// Converts BSP output into dungeon geometry (`TerrainGeometry`).
// Each partition gets a role: Room, Corridor { double_width }, or Empty.
// Rooms are padded inward (min 60 units after padding).  Corridors generate
// L-shaped or straight hallways with beveled corners at junctions.  Doors are
// placed at corridor/room boundaries with a hinge point for revolute joints.
use crate::{
    dungeon::bsp::{Partition, partition_space},
    util::safegeo::{SafeMultiPolygon, SafePolygon},
};
use avian2d::prelude::Collider;
use bevy::math::Vec2;
use geo::{LineString, MultiPolygon, Polygon, Rect, Translate};
use rand::prelude::*;
use std::collections::HashSet;

/// Space around the edge of the map
pub const MARGIN: f32 = 10.0;
/// Minimum thickness around a room
pub const PADDING: f32 = 10.0;
pub const CORRIDOR_WIDTH: f32 = 35.0;
const MIN_ROOM_SIZE: f32 = 100.0 - PADDING * 2.0;
const DOOR_PROB: f32 = 0.25 * 0.0;
const DOUBLE_DOOR_PROB: f32 = 0.75 * 0.0;
/// Probability a room gets a special variant (high for testing)
const SPECIAL_ROOM_PROB: f32 = 0.15;

pub struct TerrainGeometry {
    pub solid_rock: SafeMultiPolygon,
    pub playable_area: SafeMultiPolygon,
    pub rooms: Vec<Rect<f32>>,
    pub corridor_ends: Vec<Vec2>,
    pub doors: Vec<DoorGeometry>,
    pub torpor_zones: Vec<SafePolygon>,
}

#[derive(Clone, Copy)]
pub struct DoorGeometry {
    pub phys_rect: Rect<f32>,
    pub disp_rect: Rect<f32>,
    pub hinge: (f32, f32),
}

impl DoorGeometry {
    pub fn center(&self) -> Vec2 {
        let c = self.phys_rect.center();
        Vec2::new(c.x, c.y)
    }

    pub fn hinge_vec(&self) -> Vec2 { Vec2::new(self.hinge.0, self.hinge.1) }

    pub fn collider(&self) -> Collider {
        Collider::rectangle(self.phys_rect.width(), self.phys_rect.height())
    }

    pub fn disp_size(&self) -> Vec2 { Vec2::new(self.disp_rect.width(), self.disp_rect.height()) }

    /// Display-rect corners in local space (centered at origin), suitable for FOV shadow casting.
    pub fn disp_corners(&self) -> Vec<Vec2> {
        let w = self.disp_rect.width() / 2.0;
        let h = self.disp_rect.height() / 2.0;
        vec![Vec2::new(-w, -h), Vec2::new(w, -h), Vec2::new(w, h), Vec2::new(-w, h)]
    }
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
        let (rooms, playable_area, doors, corridor_ends, torpor_zones) =
            render(&allocated_partitions, rng);

        // The terrain is the bounds minus the playable area
        let earth = Rect::<f32>::new((0.0, 0.0), (width, height));
        let geometry = SafeMultiPolygon::from_geo(MultiPolygon(vec![earth.to_polygon()]))
            .difference(&playable_area)
            .translate(-width / 2.0, -height / 2.0);

        let offset_x = -width / 2.0;
        let offset_y = -height / 2.0;

        let playable_area = playable_area.translate(offset_x, offset_y);

        let rooms = rooms.into_iter().map(|r| r.translate(offset_x, offset_y)).collect();

        let corridor_ends = corridor_ends
            .into_iter()
            .map(|pos| Vec2::new(pos.x + offset_x, pos.y + offset_y))
            .collect();

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

        let torpor_zones =
            torpor_zones.into_iter().map(|p| p.translate(offset_x, offset_y)).collect();
        TerrainGeometry {
            solid_rock: geometry,
            playable_area,
            rooms,
            corridor_ends,
            doors,
            torpor_zones,
        }
    }
}

const EMPTY_PROB: f32 = 0.3; // if a partition is a dead end, the probability it will be empty
const CORRIDOR_PROB: f32 = 0.45; // otherwise, the probability it will be a corridor

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RoomVariant {
    Normal,
    Oval,
    Colonnade,
    Torpor,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PartitionRole {
    Room { variant: RoomVariant },
    Corridor { double_width: bool },
    Empty,
}

// Dead ends may be empty or rooms. Other partitions may be corridors or rooms.
fn allocate_roles(p: Vec<Partition>, rng: &mut impl Rng) -> Vec<(Partition, PartitionRole)> {
    p.into_iter()
        .map(|partition| {
            let connection_count = partition.connection_count();

            let role = match connection_count {
                0 => PartitionRole::Empty,
                1 => {
                    if rng.gen_bool((1.0 - EMPTY_PROB).into()) {
                        PartitionRole::Room { variant: random_room_variant(rng) }
                    } else {
                        PartitionRole::Empty
                    }
                }
                2 => {
                    if rng.gen_bool(CORRIDOR_PROB.into()) {
                        let double_width = rng.gen_bool(0.15);
                        PartitionRole::Corridor { double_width }
                    } else {
                        PartitionRole::Room { variant: random_room_variant(rng) }
                    }
                }
                _ => PartitionRole::Room { variant: random_room_variant(rng) },
            };

            (partition, role)
        })
        .collect()
}

fn union_all(base: &mut SafeMultiPolygon, polys: Vec<SafePolygon>) {
    for poly in polys {
        *base = base.union(&SafeMultiPolygon::from(poly));
    }
}

fn partitions_share_connection(p1: &Partition, p2: &Partition) -> bool {
    let conns1 = partition_connections(p1);
    let conns2 = partition_connections(p2);
    for c1 in &conns1 {
        for c2 in &conns2 {
            if c1.x == c2.x && c1.y == c2.y {
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
                if conn.x == connection.x && conn.y == connection.y {
                    return true;
                }
            }
        }
    }
    false
}

const MERGE_PROB: f64 = 0.20;

// Returns a set of connection-point keys (x_bits, y_bits) that should be fully opened between
// two adjacent rooms, removing the wall between them.
fn find_merged_room_connections(
    bsp: &[(Partition, PartitionRole)],
    rng: &mut impl Rng,
) -> HashSet<(u32, u32)> {
    let mut merged = HashSet::new();
    for i in 0..bsp.len() {
        if !matches!(bsp[i].1, PartitionRole::Room { .. }) {
            continue;
        }
        for j in (i + 1)..bsp.len() {
            if !matches!(bsp[j].1, PartitionRole::Room { .. }) {
                continue;
            }
            let conns_i = partition_connections(&bsp[i].0);
            let conns_j = partition_connections(&bsp[j].0);
            let shared: Vec<_> = conns_i
                .iter()
                .filter(|c1| conns_j.iter().any(|c2| c1.x == c2.x && c1.y == c2.y))
                .collect();
            if !shared.is_empty() && rng.gen_bool(MERGE_PROB) {
                for conn in shared {
                    merged.insert((conn.x.to_bits(), conn.y.to_bits()));
                }
            }
        }
    }
    merged
}

fn is_merged_connection(connection: &ConnectionPoint, merged: &HashSet<(u32, u32)>) -> bool {
    merged.contains(&(connection.x.to_bits(), connection.y.to_bits()))
}

// Returns a polygon that fills the entire gap from the room's edge to the partition boundary on
// the given connection side, spanning the full perpendicular extent of the room.
fn open_full_wall(room: &Rect<f32>, connection: ConnectionPoint) -> SafePolygon {
    let rect = match connection.side {
        ConnectionSide::Left => {
            Rect::new((connection.x, room.min().y), (room.min().x, room.max().y))
        }
        ConnectionSide::Right => {
            Rect::new((room.max().x, room.min().y), (connection.x, room.max().y))
        }
        ConnectionSide::Bottom => {
            Rect::new((room.min().x, connection.y), (room.max().x, room.min().y))
        }
        ConnectionSide::Top => {
            Rect::new((room.min().x, room.max().y), (room.max().x, connection.y))
        }
    };
    SafePolygon(rect.to_polygon())
}

// For rooms, shrink at least PADDING away from the edges (respecting MIN_ROOM_SIZE), adding hallways out to the edge.
// For corridors, if there are two connections, draw a straight hallway between them; otherwise, draw hallways from all connections to the center point.
// Returns rooms, a multipolygon representing passable space, doors, and corridor endpoints.
fn render(
    bsp: &[(Partition, PartitionRole)],
    rng: &mut impl Rng,
) -> (Vec<Rect<f32>>, SafeMultiPolygon, Vec<DoorGeometry>, Vec<Vec2>, Vec<SafePolygon>) {
    let merged_connections = find_merged_room_connections(bsp, rng);

    let mut rooms = Vec::new();
    let mut playables = SafeMultiPolygon::empty();
    let mut doors = Vec::new();
    let mut corridor_ends_set: HashSet<(u32, u32)> = HashSet::new();
    let mut torpor_zones: Vec<SafePolygon> = Vec::new();

    // Avoid hallway stubs leading to nothing.
    let empty_connections: Vec<(f32, f32)> = bsp
        .iter()
        .filter(|(_, role)| matches!(role, PartitionRole::Empty))
        .flat_map(|(partition, _)| partition_connections(partition).into_iter().map(|c| (c.x, c.y)))
        .collect();

    let is_live =
        |c: &ConnectionPoint| !empty_connections.iter().any(|&(ex, ey)| c.x == ex && c.y == ey);

    for (partition, role) in bsp {
        let mut region = SafeMultiPolygon::empty();

        match role {
            PartitionRole::Empty => continue,
            PartitionRole::Room { variant } => {
                let room = shrink_room(partition);
                rooms.push(room);

                match variant {
                    RoomVariant::Normal => {
                        region = region.union(&SafeMultiPolygon::from(room.to_polygon()));
                        for connection in
                            partition_connections(partition).into_iter().filter(is_live)
                        {
                            if is_merged_connection(&connection, &merged_connections) {
                                region = region.union(&SafeMultiPolygon::from(open_full_wall(
                                    &room, connection,
                                )));
                            } else {
                                let is_double =
                                    is_double_width_corridor_connection(&connection, bsp);
                                let width =
                                    if is_double { CORRIDOR_WIDTH * 2.0 } else { CORRIDOR_WIDTH };
                                union_all(
                                    &mut region,
                                    connect_room_to_connection(&room, connection, width),
                                );

                                let door_prob =
                                    if is_double { DOUBLE_DOOR_PROB } else { DOOR_PROB };
                                if rng.gen_bool(door_prob as f64) {
                                    let room_entry = room_entry_point(&room, connection);
                                    if is_double {
                                        let (d1, d2) =
                                            create_double_door(connection.side, room_entry);
                                        doors.push(d1);
                                        doors.push(d2);
                                    } else {
                                        doors.push(create_door(connection.side, room_entry));
                                    }
                                }
                            }
                        }
                    }
                    RoomVariant::Oval => {
                        region = region.union(&SafeMultiPolygon::from(oval_polygon(&room)));
                        for connection in
                            partition_connections(partition).into_iter().filter(is_live)
                        {
                            if is_merged_connection(&connection, &merged_connections) {
                                region = region.union(&SafeMultiPolygon::from(open_full_wall(
                                    &room, connection,
                                )));
                            } else {
                                let is_double =
                                    is_double_width_corridor_connection(&connection, bsp);
                                let width =
                                    if is_double { CORRIDOR_WIDTH * 2.0 } else { CORRIDOR_WIDTH };
                                let entry = oval_entry_point(&room, connection, width);
                                union_all(&mut region, connect_to_entry(entry, connection, width));
                            }
                        }
                    }
                    RoomVariant::Colonnade => {
                        region = region.union(&SafeMultiPolygon::from(room.to_polygon()));
                        for connection in
                            partition_connections(partition).into_iter().filter(is_live)
                        {
                            if is_merged_connection(&connection, &merged_connections) {
                                region = region.union(&SafeMultiPolygon::from(open_full_wall(
                                    &room, connection,
                                )));
                            } else {
                                let is_double =
                                    is_double_width_corridor_connection(&connection, bsp);
                                let width =
                                    if is_double { CORRIDOR_WIDTH * 2.0 } else { CORRIDOR_WIDTH };
                                union_all(
                                    &mut region,
                                    connect_room_to_connection(&room, connection, width),
                                );
                            }
                        }
                        for col in colonnade_columns(&room) {
                            region = region.difference(&SafeMultiPolygon::from(col.to_polygon()));
                        }
                    }
                    RoomVariant::Torpor => {
                        region = region.union(&SafeMultiPolygon::from(room.to_polygon()));
                        for connection in
                            partition_connections(partition).into_iter().filter(is_live)
                        {
                            if is_merged_connection(&connection, &merged_connections) {
                                region = region.union(&SafeMultiPolygon::from(open_full_wall(
                                    &room, connection,
                                )));
                            } else {
                                let is_double =
                                    is_double_width_corridor_connection(&connection, bsp);
                                let width =
                                    if is_double { CORRIDOR_WIDTH * 2.0 } else { CORRIDOR_WIDTH };
                                union_all(
                                    &mut region,
                                    connect_room_to_connection(&room, connection, width),
                                );
                                if rng.gen_bool(DOOR_PROB as f64) {
                                    let room_entry = room_entry_point(&room, connection);
                                    doors.push(create_door(connection.side, room_entry));
                                }
                            }
                        }
                        let inner = Rect::new(
                            (room.min().x + CORRIDOR_WIDTH, room.min().y + CORRIDOR_WIDTH),
                            (room.max().x - CORRIDOR_WIDTH, room.max().y - CORRIDOR_WIDTH),
                        );
                        if inner.width() > 0.0 && inner.height() > 0.0 {
                            torpor_zones.push(SafePolygon(inner.to_polygon()));
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
                    region = region.union(&SafeMultiPolygon::from(bevel_at_point(center, width)));
                    for connection in connections {
                        union_all(&mut region, connect_point_to_center(connection, center, width));
                    }
                }
            }
        }

        playables = playables.union(&region);
    }

    // Collect all live corridor endpoints (connection points).
    for (partition, role) in bsp {
        if matches!(role, PartitionRole::Empty) {
            continue;
        }
        for connection in partition_connections(partition).into_iter().filter(&is_live) {
            let bits_x = connection.x.to_bits();
            let bits_y = connection.y.to_bits();
            corridor_ends_set.insert((bits_x, bits_y));
        }
    }

    let corridor_ends: Vec<Vec2> = corridor_ends_set
        .into_iter()
        .map(|(bx, by)| Vec2::new(f32::from_bits(bx), f32::from_bits(by)))
        .collect();

    for door in &doors {
        let h_x = door.hinge.0;
        let h_y = door.hinge.1;
        let hinge_rect = Rect::new((h_x - 2.5, h_y - 2.5), (h_x + 2.5, h_y + 2.5));
        playables = playables.difference(&SafeMultiPolygon::from(hinge_rect.to_polygon()));
    }

    (rooms, playables, doors, corridor_ends, torpor_zones)
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

fn room_entry_point(room: &Rect<f32>, connection: ConnectionPoint) -> (f32, f32) {
    match connection.side {
        ConnectionSide::Left => (room.min().x, connection.y.clamp(room.min().y, room.max().y)),
        ConnectionSide::Right => (room.max().x, connection.y.clamp(room.min().y, room.max().y)),
        ConnectionSide::Bottom => (connection.x.clamp(room.min().x, room.max().x), room.min().y),
        ConnectionSide::Top => (connection.x.clamp(room.min().x, room.max().x), room.max().y),
    }
}

fn random_room_variant(rng: &mut impl Rng) -> RoomVariant {
    if rng.gen_bool(SPECIAL_ROOM_PROB as f64) {
        match rng.gen_range(0..1) {
            0 => RoomVariant::Colonnade,
            2 => RoomVariant::Torpor,
            _ => unreachable!(),
        }
        // TODO: oval rooms cause big performance problems. Investigate.
        // if rng.gen_bool(0.5) { RoomVariant::Colonnade } else { RoomVariant::Torpor }
    } else {
        RoomVariant::Normal
    }
}

fn oval_polygon(room: &Rect<f32>) -> SafePolygon {
    let cx = (room.min().x + room.max().x) / 2.0;
    let cy = (room.min().y + room.max().y) / 2.0;
    let a = room.width() / 2.0;
    let b = room.height() / 2.0;
    let steps = 32usize;
    let coords: Vec<(f32, f32)> = (0..=steps)
        .map(|i| {
            let angle = 2.0 * std::f32::consts::PI * i as f32 / steps as f32;
            (cx + a * angle.cos(), cy + b * angle.sin())
        })
        .collect();
    SafePolygon(Polygon::new(LineString::from(coords), vec![]))
}

// Returns where the corridor (of the given width) should terminate at the oval's boundary,
// pushed deep enough that both lateral edges of the corridor reach the oval surface.
fn oval_entry_point(
    room: &Rect<f32>,
    connection: ConnectionPoint,
    corridor_width: f32,
) -> (f32, f32) {
    let cx = (room.min().x + room.max().x) / 2.0;
    let cy = (room.min().y + room.max().y) / 2.0;
    let a = room.width() / 2.0;
    let b = room.height() / 2.0;
    let hw = corridor_width / 2.0;

    match connection.side {
        ConnectionSide::Left | ConnectionSide::Right => {
            // Corridor spans y' in [connection.y - hw, connection.y + hw].
            // oval left boundary at y': cx - a*sqrt(1 - ((y'-cy)/b)^2)
            // The boundary is ∪-shaped, so max (deepest into room) is at the endpoints.
            // Equivalently: the half-width term x_at(y') = a*sqrt(...) is ∩-shaped,
            // so min(x_at) is at the endpoints of the span.
            let y_lo = (connection.y - hw).clamp(cy - b, cy + b);
            let y_hi = (connection.y + hw).clamp(cy - b, cy + b);
            let x_at = |y: f32| {
                let t = (y - cy) / b;
                a * (1.0 - t * t).max(0.0).sqrt()
            };
            let min_extent = x_at(y_lo).min(x_at(y_hi)) - 2.0;
            let x = if matches!(connection.side, ConnectionSide::Left) {
                cx - min_extent
            } else {
                cx + min_extent
            };
            (x, connection.y.clamp(cy - b, cy + b))
        }
        ConnectionSide::Bottom | ConnectionSide::Top => {
            let x_lo = (connection.x - hw).clamp(cx - a, cx + a);
            let x_hi = (connection.x + hw).clamp(cx - a, cx + a);
            let y_at = |x: f32| {
                let t = (x - cx) / a;
                b * (1.0 - t * t).max(0.0).sqrt()
            };
            let min_extent = y_at(x_lo).min(y_at(x_hi)) - 2.0;
            let y = if matches!(connection.side, ConnectionSide::Bottom) {
                cy - min_extent
            } else {
                cy + min_extent
            };
            (connection.x.clamp(cx - a, cx + a), y)
        }
    }
}

fn colonnade_columns(room: &Rect<f32>) -> Vec<Rect<f32>> {
    const COL_SIZE: f32 = 20.0;
    const COL_HALF: f32 = COL_SIZE / 2.0;
    // Column center sits CORRIDOR_WIDTH inward from each long wall face
    let from_wall = CORRIDOR_WIDTH + COL_HALF;

    let mut columns = Vec::new();
    let long_axis_is_x = room.width() >= room.height();

    // wall_a / wall_b: coordinates of the two column rows (perpendicular to long axis)
    // along_min / along_max: range along the long axis within which columns are placed
    let (wall_a, wall_b, along_min, along_max) = if long_axis_is_x {
        let wa = room.min().y + from_wall;
        let wb = room.max().y - from_wall;
        if wb - wa < CORRIDOR_WIDTH {
            return columns; // room too narrow to fit two rows with a gap between them
        }
        (wa, wb, room.min().x + CORRIDOR_WIDTH, room.max().x - CORRIDOR_WIDTH)
    } else {
        let wa = room.min().x + from_wall;
        let wb = room.max().x - from_wall;
        if wb - wa < CORRIDOR_WIDTH {
            return columns;
        }
        (wa, wb, room.min().y + CORRIDOR_WIDTH, room.max().y - CORRIDOR_WIDTH)
    };

    let span = along_max - along_min;
    if span <= 0.0 {
        return columns;
    }

    let n = ((span + CORRIDOR_WIDTH) / (COL_SIZE + CORRIDOR_WIDTH)).floor() as usize;
    if n == 0 {
        return columns;
    }

    let total = n as f32 * COL_SIZE + (n - 1) as f32 * CORRIDOR_WIDTH;
    let margin = (span - total) / 2.0;
    let step = COL_SIZE + CORRIDOR_WIDTH;

    for i in 0..n {
        let mid = along_min + margin + COL_HALF + i as f32 * step;
        if long_axis_is_x {
            columns.push(Rect::new(
                (mid - COL_HALF, wall_a - COL_HALF),
                (mid + COL_HALF, wall_a + COL_HALF),
            ));
            columns.push(Rect::new(
                (mid - COL_HALF, wall_b - COL_HALF),
                (mid + COL_HALF, wall_b + COL_HALF),
            ));
        } else {
            columns.push(Rect::new(
                (wall_a - COL_HALF, mid - COL_HALF),
                (wall_a + COL_HALF, mid + COL_HALF),
            ));
            columns.push(Rect::new(
                (wall_b - COL_HALF, mid - COL_HALF),
                (wall_b + COL_HALF, mid + COL_HALF),
            ));
        }
    }

    columns
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

fn connect_to_entry(
    entry: (f32, f32),
    connection: ConnectionPoint,
    width: f32,
) -> Vec<SafePolygon> {
    let conn_pt = (connection.x, connection.y);
    let elbow = if connection.side.is_vertical() {
        (entry.0, connection.y)
    } else {
        (connection.x, entry.1)
    };

    let mut polygons = vec![rect_for_segment(conn_pt, elbow, width)];

    let needs_bend = if connection.side.is_vertical() {
        (entry.1 - connection.y).abs() > f32::EPSILON
    } else {
        (entry.0 - connection.x).abs() > f32::EPSILON
    };
    if needs_bend {
        polygons.push(bevel_at_point(elbow, width));
        polygons.push(rect_for_segment(elbow, entry, width));
    }

    polygons
}

fn connect_room_to_connection(
    room: &Rect<f32>,
    connection: ConnectionPoint,
    width: f32,
) -> Vec<SafePolygon> {
    connect_to_entry(room_entry_point(room, connection), connection, width)
}

fn connect_adjacent(a: ConnectionPoint, b: ConnectionPoint, width: f32) -> Vec<SafePolygon> {
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
) -> Vec<SafePolygon> {
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

fn bevel_at_point(p: (f32, f32), width: f32) -> SafePolygon {
    let h = width / 2.0;
    let q = width / 4.0;
    SafePolygon(Polygon::new(
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
    ))
}

fn rect_for_segment(a: (f32, f32), b: (f32, f32), width: f32) -> SafePolygon {
    if !a.0.is_finite() || !a.1.is_finite() || !b.0.is_finite() || !b.1.is_finite() {
        return SafePolygon(Polygon::new(LineString::new(vec![]), vec![]));
    }
    if (a.0 - b.0).abs() < f32::EPSILON {
        let min_y = a.1.min(b.1);
        let max_y = a.1.max(b.1);
        let half = width / 2.0;
        SafePolygon(Rect::new((a.0 - half, min_y), (a.0 + half, max_y)).to_polygon())
    } else if (a.1 - b.1).abs() < f32::EPSILON {
        let min_x = a.0.min(b.0);
        let max_x = a.0.max(b.0);
        let half = width / 2.0;
        SafePolygon(Rect::new((min_x, a.1 - half), (max_x, a.1 + half)).to_polygon())
    } else {
        unreachable!("rect_for_segment called with diagonal segment ({:?} to {:?})", a, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::BoundingRect;

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

    fn large_partition() -> Partition {
        Partition {
            x: (100.0, 500.0),
            y: (100.0, 400.0),
            horz_conn: (vec![], vec![]),
            vert_conn: (vec![], vec![]),
        }
    }

    #[test]
    fn test_shrink_room_large_partition_has_padding() {
        let p = large_partition();
        let room = shrink_room(&p);
        assert!(room.min().x >= p.x.0 + PADDING - f32::EPSILON);
        assert!(room.min().y >= p.y.0 + PADDING - f32::EPSILON);
        assert!(room.max().x <= p.x.1 - PADDING + f32::EPSILON);
        assert!(room.max().y <= p.y.1 - PADDING + f32::EPSILON);
    }

    #[test]
    fn test_shrink_room_minimum_size_on_tiny_partition() {
        // Partition too small to fit padding — room must still meet MIN_ROOM_SIZE.
        let p = Partition {
            x: (0.0, 20.0),
            y: (0.0, 20.0),
            horz_conn: (vec![], vec![]),
            vert_conn: (vec![], vec![]),
        };
        let room = shrink_room(&p);
        assert!(room.width() >= MIN_ROOM_SIZE - f32::EPSILON);
        assert!(room.height() >= MIN_ROOM_SIZE - f32::EPSILON);
    }

    #[test]
    fn test_partition_connections_left_side() {
        let p = Partition {
            x: (100.0, 300.0),
            y: (200.0, 400.0),
            horz_conn: (vec![250.0], vec![]),
            vert_conn: (vec![], vec![]),
        };
        let conns = partition_connections(&p);
        assert_eq!(conns.len(), 1);
        assert_eq!(conns[0].x, 100.0);
        assert_eq!(conns[0].y, 250.0);
        assert!(matches!(conns[0].side, ConnectionSide::Left));
    }

    #[test]
    fn test_partition_connections_all_sides() {
        let p = Partition {
            x: (100.0, 300.0),
            y: (200.0, 400.0),
            horz_conn: (vec![250.0], vec![350.0]),
            vert_conn: (vec![150.0], vec![200.0]),
        };
        assert_eq!(partition_connections(&p).len(), 4);
    }

    #[test]
    fn test_rect_for_segment_vertical() {
        // Vertical segment (same x): result rect spans the y range, width = corridor width.
        let poly = rect_for_segment((50.0, 100.0), (50.0, 200.0), 40.0);
        let bbox = poly.bounding_rect().unwrap();
        assert!((bbox.width() - 40.0).abs() < f32::EPSILON);
        assert!((bbox.height() - 100.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_rect_for_segment_horizontal() {
        // Horizontal segment (same y): result rect spans the x range, height = corridor width.
        let poly = rect_for_segment((0.0, 75.0), (120.0, 75.0), 30.0);
        let bbox = poly.bounding_rect().unwrap();
        assert!((bbox.width() - 120.0).abs() < f32::EPSILON);
        assert!((bbox.height() - 30.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_bevel_at_point_has_8_exterior_vertices() {
        let poly = bevel_at_point((0.0, 0.0), 40.0);
        // LineString is closed: first == last, so 9 coords for 8 distinct vertices.
        assert_eq!(poly.exterior().0.len(), 9);
    }

    #[test]
    fn test_bevel_at_point_all_vertices_within_radius() {
        let width = 40.0;
        let poly = bevel_at_point((10.0, 20.0), width);
        for coord in &poly.exterior().0 {
            let dx = coord.x - 10.0;
            let dy = coord.y - 20.0;
            let dist = (dx * dx + dy * dy).sqrt();
            assert!(dist <= width, "vertex at distance {dist} exceeds width {width}");
        }
    }

    #[test]
    fn test_door_geometry_disp_corners_symmetric() {
        let door = create_door(ConnectionSide::Left, (100.0, 200.0));
        let corners = door.disp_corners();
        assert_eq!(corners.len(), 4);
        // Corners are centered at origin, so opposite corners should sum to zero.
        assert!((corners[0] + corners[2]).length() < f32::EPSILON);
        assert!((corners[1] + corners[3]).length() < f32::EPSILON);
    }

    #[test]
    fn test_door_geometry_center_matches_phys_rect() {
        let door = create_door(ConnectionSide::Right, (300.0, 150.0));
        let c = door.center();
        let rect_center = door.phys_rect.center();
        assert!((c.x - rect_center.x).abs() < f32::EPSILON);
        assert!((c.y - rect_center.y).abs() < f32::EPSILON);
    }

    // --- Degenerate / extreme geometry stress tests ---

    #[test]
    fn test_bevel_at_point_zero_width() { let _poly = bevel_at_point((0.0, 0.0), 0.0); }

    #[test]
    fn test_bevel_at_point_negative_width() { let _poly = bevel_at_point((0.0, 0.0), -20.0); }

    #[test]
    fn test_bevel_at_point_nan_width() { let _poly = bevel_at_point((0.0, 0.0), f32::NAN); }

    #[test]
    fn test_bevel_at_point_nan_coords() { let _poly = bevel_at_point((f32::NAN, 0.0), 40.0); }

    #[test]
    fn test_bevel_at_point_huge_width() { let _poly = bevel_at_point((0.0, 0.0), f32::MAX / 2.0); }

    #[test]
    fn test_rect_for_segment_zero_length() {
        // Start == End: a degenerate segment with no length.
        let _poly = rect_for_segment((50.0, 50.0), (50.0, 50.0), 30.0);
    }

    #[test]
    fn test_rect_for_segment_zero_width() {
        let _poly = rect_for_segment((0.0, 0.0), (0.0, 100.0), 0.0);
    }

    #[test]
    fn test_rect_for_segment_negative_width() {
        let _poly = rect_for_segment((0.0, 0.0), (0.0, 100.0), -10.0);
    }

    #[test]
    fn test_rect_for_segment_nan_width() {
        let _poly = rect_for_segment((0.0, 0.0), (0.0, 100.0), f32::NAN);
    }

    #[test]
    fn test_rect_for_segment_nan_coords() {
        let _poly = rect_for_segment((f32::NAN, 0.0), (0.0, 100.0), 30.0);
    }

    #[test]
    fn test_shrink_room_zero_size_partition() {
        let p = Partition {
            x: (100.0, 100.0),
            y: (200.0, 200.0),
            horz_conn: (vec![], vec![]),
            vert_conn: (vec![], vec![]),
        };
        let _room = shrink_room(&p);
    }

    #[test]
    fn test_shrink_room_inverted_partition() {
        // max < min on both axes
        let p = Partition {
            x: (300.0, 100.0),
            y: (400.0, 200.0),
            horz_conn: (vec![], vec![]),
            vert_conn: (vec![], vec![]),
        };
        let _room = shrink_room(&p);
    }

    #[test]
    fn test_terrain_geometry_zero_dimensions() {
        let mut rng = rand::thread_rng();
        let _result = TerrainGeometry::new_seeded(0.0, 0.0, &mut rng);
    }

    #[test]
    fn test_terrain_geometry_tiny_dimensions() {
        let mut rng = rand::thread_rng();
        let _result = TerrainGeometry::new_seeded(1.0, 1.0, &mut rng);
    }

    #[test]
    fn test_terrain_geometry_very_narrow() {
        // Wide but extremely short — likely to produce no rooms or degenerate corridors.
        let mut rng = rand::thread_rng();
        let _result = TerrainGeometry::new_seeded(1600.0, 10.0, &mut rng);
    }

    #[test]
    #[ignore = "hangs: BSP partition_space loops forever on a 1M×1M space"]
    fn test_terrain_geometry_huge_dimensions() {
        let mut rng = rand::thread_rng();
        let _result = TerrainGeometry::new_seeded(1_000_000.0, 1_000_000.0, &mut rng);
    }

    #[test]
    #[ignore = "likely hangs: NaN dimensions cause infinite BSP recursion"]
    fn test_terrain_geometry_nan_dimensions() {
        let mut rng = rand::thread_rng();
        let _result = TerrainGeometry::new_seeded(f32::NAN, f32::NAN, &mut rng);
    }

    #[test]
    #[ignore = "likely hangs: infinite dimensions cause infinite BSP recursion"]
    fn test_terrain_geometry_infinite_dimensions() {
        let mut rng = rand::thread_rng();
        let _result = TerrainGeometry::new_seeded(f32::INFINITY, f32::INFINITY, &mut rng);
    }
}
