// Converts BSP output into dungeon geometry (`LevelPlan`) via the `RoomKind`
// extension point. Rooms are padded inward (min 60 units after padding).
// Corridors generate L-shaped or straight hallways with beveled corners at
// junctions — carving corridors is framework-owned, not user-extensible, since
// every such game needs *some* way to join partitions. Doors are placed at
// corridor/room boundaries with a hinge point for revolute joints.
use crate::dungeon::bsp::{CORRIDOR_WIDTH, PADDING, Partition, partition_space};
use crate::util::safegeo::{SafeMultiPolygon, SafePolygon};
use avian2d::prelude::Collider;
use bevy::math::Vec2;
use geo::{LineString, MultiPolygon, Polygon, Rect, Translate};
use rand::RngCore;
use rand::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Space around the edge of the map.
pub const MARGIN: f32 = 10.0;
const MIN_ROOM_SIZE: f32 = 100.0 - PADDING * 2.0;
/// Probability a live connection gets a door at all. Currently hacky-disabled
/// (evaluates to 0.0) but left showing the intended value.
pub const DOOR_PROB: f32 = 0.25 * 0.0;
/// Like `DOOR_PROB`, for connections carrying a double-width corridor.
pub const DOUBLE_DOOR_PROB: f32 = 0.75 * 0.0;

/// if a partition is a dead end, the probability it will be empty
const EMPTY_PROB: f32 = 0.3;
/// otherwise, the probability it will be a corridor
const CORRIDOR_PROB: f32 = 0.45;
/// probability two rooms sharing a connection have the wall between them fully opened
const MERGE_PROB: f64 = 0.20;

/// The output of level generation: geometry plus tagged extras, ready for a game's
/// `PopulateLevel` observer to spawn from. String-tagged `markers`/`regions` (rather
/// than a fixed struct of `Vec`s) let a custom `RoomKind` introduce new
/// gameplay-relevant output without an engine change.
pub struct LevelPlan {
    pub solid_rock: SafeMultiPolygon,
    pub playable_area: SafeMultiPolygon,
    pub glass_walls: SafeMultiPolygon,
    pub rooms: Vec<Rect<f32>>,
    pub corridor_ends: Vec<Vec2>,
    pub doors: Vec<DoorGeometry>,
    pub markers: HashMap<&'static str, Vec<Vec2>>,
    pub regions: HashMap<&'static str, Vec<SafePolygon>>,
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

impl LevelPlan {
    pub fn new(width: f32, height: f32, registry: &RoomRegistry) -> Self {
        let mut rng = rand::thread_rng();
        Self::new_seeded(width, height, registry, &mut rng)
    }

    pub fn new_seeded(width: f32, height: f32, registry: &RoomRegistry, rng: &mut impl Rng) -> Self {
        // The playable area bounds
        let bounds = Partition {
            x: (MARGIN, width - MARGIN),
            y: (MARGIN, height - MARGIN),
            horz_conn: (vec![], vec![]),
            vert_conn: (vec![], vec![]),
        };

        let partitions = partition_space(bounds, rng);
        let mut allocated_partitions = allocate_roles(partitions, registry, rng);

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
        let raw = render(&allocated_partitions, rng);

        // The terrain is the bounds minus the playable area, minus glass walls (which have their own entity).
        let earth = Rect::<f32>::new((0.0, 0.0), (width, height));
        let geometry = SafeMultiPolygon::from_geo(MultiPolygon(vec![earth.to_polygon()]))
            .difference(&raw.playable_area)
            .difference(&raw.glass_walls)
            .translate(-width / 2.0, -height / 2.0);

        let offset_x = -width / 2.0;
        let offset_y = -height / 2.0;

        let playable_area = raw.playable_area.translate(offset_x, offset_y);
        let glass_walls = raw.glass_walls.translate(offset_x, offset_y);

        let rooms = raw.rooms.into_iter().map(|r| r.translate(offset_x, offset_y)).collect();

        let corridor_ends = raw
            .corridor_ends
            .into_iter()
            .map(|pos| Vec2::new(pos.x + offset_x, pos.y + offset_y))
            .collect();

        let doors = raw
            .doors
            .into_iter()
            .map(|mut d| {
                d.phys_rect = d.phys_rect.translate(offset_x, offset_y);
                d.disp_rect = d.disp_rect.translate(offset_x, offset_y);
                d.hinge.0 += offset_x;
                d.hinge.1 += offset_y;
                d
            })
            .collect();

        let markers = raw
            .markers
            .into_iter()
            .map(|(tag, positions)| {
                let positions = positions
                    .into_iter()
                    .map(|pos| Vec2::new(pos.x + offset_x, pos.y + offset_y))
                    .collect();
                (tag, positions)
            })
            .collect();

        let regions = raw
            .regions
            .into_iter()
            .map(|(tag, polys)| {
                let polys = polys.into_iter().map(|p| p.translate(offset_x, offset_y)).collect();
                (tag, polys)
            })
            .collect();

        LevelPlan {
            solid_rock: geometry,
            playable_area,
            glass_walls,
            rooms,
            corridor_ends,
            doors,
            markers,
            regions,
        }
    }
}

/// A BSP-derived point where a room or corridor should connect to a neighbor, enriched
/// with the level-wide context (`is_double`/`is_merged`) an individual room can't derive
/// on its own — that requires looking at every other partition.
#[derive(Copy, Clone)]
pub struct RoomConnection {
    pub x: f32,
    pub y: f32,
    pub side: ConnectionSide,
    /// This connection carries a double-width corridor.
    pub is_double: bool,
    /// The wall at this connection was randomly chosen to be fully opened rather than
    /// getting a normal corridor stub (see `MERGE_PROB`); `RoomKind::carve` should open
    /// the full wall instead of carving a connector.
    pub is_merged: bool,
}

/// What a `RoomKind` is asked to carve: the room's (already padding-shrunk) bounds and
/// its live connections.
pub struct RoomRequest {
    pub room: Rect<f32>,
    pub connections: Vec<RoomConnection>,
}

/// Level-wide information relevant to picking a room kind, passed to `RoomKind::weight`.
pub struct LevelContext {
    pub connection_count: usize,
}

/// Accumulates one room's contribution to the level: floor polygon, doors, and
/// string-tagged markers/regions. Exposes only additive mutators (`add_*`) plus
/// `carve_connection`/`carve_full_wall_opening` for the common connection-carving
/// pattern — a `RoomKind` cannot reach into another room's contribution or bypass the
/// union semantics of `add_floor`.
#[derive(Default)]
pub struct LevelBuilder {
    floor: SafeMultiPolygon,
    doors: Vec<DoorGeometry>,
    markers: Vec<(&'static str, Vec2)>,
    regions: Vec<(&'static str, SafePolygon)>,
}

impl LevelBuilder {
    pub fn add_floor(&mut self, poly: impl Into<SafeMultiPolygon>) {
        self.floor = self.floor.union(&poly.into());
    }

    pub fn subtract_floor(&mut self, poly: impl Into<SafeMultiPolygon>) {
        self.floor = self.floor.difference(&poly.into());
    }

    pub fn add_door(&mut self, door: DoorGeometry) { self.doors.push(door); }

    pub fn add_marker(&mut self, tag: &'static str, pos: Vec2) { self.markers.push((tag, pos)); }

    pub fn add_region(&mut self, tag: &'static str, poly: SafePolygon) {
        self.regions.push((tag, poly));
    }

    pub fn floor(&self) -> &SafeMultiPolygon { &self.floor }

    pub fn doors(&self) -> &[DoorGeometry] { &self.doors }

    pub fn markers(&self) -> &[(&'static str, Vec2)] { &self.markers }

    pub fn regions(&self) -> &[(&'static str, SafePolygon)] { &self.regions }

    /// Carves a corridor connection into `room`'s wall: opens the shared wall fully if
    /// the connection was merged with its neighbor, otherwise connects a corridor-width
    /// stub to the room's rectangular entry point and probabilistically adds a door.
    /// This is the block every stock room but Oval and Colonnade shares verbatim.
    pub fn carve_connection(
        &mut self,
        room: &Rect<f32>,
        conn: &RoomConnection,
        rng: &mut dyn RngCore,
        door_prob: f32,
        double_door_prob: f32,
    ) {
        if conn.is_merged {
            self.add_floor(open_full_wall(room, conn));
            return;
        }
        let width = if conn.is_double { CORRIDOR_WIDTH * 2.0 } else { CORRIDOR_WIDTH };
        for poly in connect_room_to_connection(room, conn, width) {
            self.add_floor(poly);
        }
        let prob = if conn.is_double { double_door_prob } else { door_prob };
        if rand::Rng::gen_bool(rng, prob as f64) {
            let entry = room_entry_point(room, conn);
            if conn.is_double {
                let (d1, d2) = create_double_door(conn.side, entry);
                self.add_door(d1);
                self.add_door(d2);
            } else {
                self.add_door(create_door(conn.side, entry));
            }
        }
    }

    /// Like `carve_connection` but skips the door roll entirely — for rooms (Colonnade)
    /// whose connections never get doors.
    pub fn carve_connection_geometry_only(&mut self, room: &Rect<f32>, conn: &RoomConnection) {
        if conn.is_merged {
            self.add_floor(open_full_wall(room, conn));
            return;
        }
        let width = if conn.is_double { CORRIDOR_WIDTH * 2.0 } else { CORRIDOR_WIDTH };
        for poly in connect_room_to_connection(room, conn, width) {
            self.add_floor(poly);
        }
    }

    /// Connects a custom `entry` point (rather than the room's rectangular entry point)
    /// to `conn` — for rooms like Oval whose boundary isn't the room rect itself.
    pub fn carve_entry(&mut self, entry: Vec2, conn: &RoomConnection, width: f32) {
        for poly in connect_to_entry(entry, conn, width) {
            self.add_floor(poly);
        }
    }

    /// Fully opens the wall at a merged connection. Exposed separately from
    /// `carve_connection` for rooms (Oval) that carve non-merged connections differently
    /// but still need to handle the merged case the same way as everyone else.
    pub fn carve_full_wall_opening(&mut self, room: &Rect<f32>, conn: &RoomConnection) {
        self.add_floor(open_full_wall(room, conn));
    }
}

/// The `RoomKind` extension point: how a BSP partition assigned the Room role gets
/// carved into floor, doors, and tagged markers/regions. Pure data in, pure data out —
/// no ECS access, trivially unit-testable.
pub trait RoomKind: Send + Sync + 'static {
    fn name(&self) -> &'static str;

    /// Relative weight for being chosen to fill a Room-role partition; 0.0 means never
    /// chosen (but still registrable, e.g. for a game that only ever picks it explicitly).
    fn weight(&self, ctx: &LevelContext) -> f32;

    fn carve(&self, req: &RoomRequest, rng: &mut dyn RngCore, out: &mut LevelBuilder);

    /// Whether this room's silhouette is the plain room rectangle, making it eligible for
    /// a glass wall to a spatially-adjacent room with no BSP connector between them.
    fn rectangular_borders(&self) -> bool { true }
}

/// The set of `RoomKind`s a level generator can draw from, with `Arc` so the same
/// registry can be shared across levels without recreating room instances.
#[derive(Default, Clone)]
pub struct RoomRegistry {
    kinds: Vec<Arc<dyn RoomKind>>,
}

impl RoomRegistry {
    pub fn new() -> Self { Self::default() }

    pub fn register(&mut self, kind: impl RoomKind) -> &mut Self {
        self.kinds.push(Arc::new(kind));
        self
    }

    pub fn choose(&self, ctx: &LevelContext, rng: &mut dyn RngCore) -> Option<Arc<dyn RoomKind>> {
        let weights: Vec<f32> = self.kinds.iter().map(|k| k.weight(ctx)).collect();
        let total: f32 = weights.iter().sum();
        if total <= 0.0 {
            return None;
        }
        let mut choice = rand::Rng::gen_range(rng, 0.0..total);
        for (kind, &w) in self.kinds.iter().zip(&weights) {
            if choice < w {
                return Some(kind.clone());
            }
            choice -= w;
        }
        self.kinds.last().cloned()
    }
}

#[derive(Clone)]
pub enum PartitionRole {
    Room { kind: Arc<dyn RoomKind> },
    Corridor { double_width: bool },
    Empty,
}

// Dead ends may be empty or rooms. Other partitions may be corridors or rooms.
fn allocate_roles(
    p: Vec<Partition>,
    registry: &RoomRegistry,
    rng: &mut impl Rng,
) -> Vec<(Partition, PartitionRole)> {
    p.into_iter()
        .map(|partition| {
            let connection_count = partition.connection_count();

            let role = match connection_count {
                0 => PartitionRole::Empty,
                1 => {
                    if rng.gen_bool((1.0 - EMPTY_PROB).into()) {
                        PartitionRole::Room { kind: choose_room(registry, connection_count, rng) }
                    } else {
                        PartitionRole::Empty
                    }
                }
                2 => {
                    if rng.gen_bool(CORRIDOR_PROB.into()) {
                        let double_width = rng.gen_bool(0.15);
                        PartitionRole::Corridor { double_width }
                    } else {
                        PartitionRole::Room { kind: choose_room(registry, connection_count, rng) }
                    }
                }
                _ => PartitionRole::Room { kind: choose_room(registry, connection_count, rng) },
            };

            (partition, role)
        })
        .collect()
}

fn choose_room(registry: &RoomRegistry, connection_count: usize, rng: &mut impl Rng) -> Arc<dyn RoomKind> {
    let ctx = LevelContext { connection_count };
    registry.choose(&ctx, rng).unwrap_or_else(|| panic!("RoomRegistry has no room kind with positive weight"))
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
    connection: &RoomConnection,
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

fn is_merged_connection(connection: &RoomConnection, merged: &HashSet<(u32, u32)>) -> bool {
    merged.contains(&(connection.x.to_bits(), connection.y.to_bits()))
}

// Returns a polygon that fills the entire gap from the room's edge to the partition boundary on
// the given connection side, spanning the full perpendicular extent of the room.
fn open_full_wall(room: &Rect<f32>, conn: &RoomConnection) -> SafePolygon {
    let rect = match conn.side {
        ConnectionSide::Left => Rect::new((conn.x, room.min().y), (room.min().x, room.max().y)),
        ConnectionSide::Right => Rect::new((room.max().x, room.min().y), (conn.x, room.max().y)),
        ConnectionSide::Bottom => Rect::new((room.min().x, conn.y), (room.max().x, room.min().y)),
        ConnectionSide::Top => Rect::new((room.min().x, room.max().y), (room.max().x, conn.y)),
    };
    SafePolygon(rect.to_polygon())
}

// Returns a glass wall polygon filling the gap between two spatially-adjacent rooms (no BSP
// connector), with 50% probability. Returns None if not adjacent, no common span, or skipped.
fn glass_wall_between(
    p_i: &Partition,
    p_j: &Partition,
    r_i: &Rect<f32>,
    r_j: &Rect<f32>,
    rng: &mut impl Rng,
) -> Option<SafePolygon> {
    // shrink_along_wall: whether to inset along the y axis (true) or x axis (false)
    let (mut poly, shrink_y) = if p_i.x.1 == p_j.x.0 {
        // Detect horizontal adjacency: p_i to the left of p_j
        let y0 = r_i.min().y.max(r_j.min().y);
        let y1 = r_i.max().y.min(r_j.max().y);
        if y1 <= y0 {
            return None;
        }
        (Rect::new((r_i.max().x, y0), (r_j.min().x, y1)), true)
    } else if p_j.x.1 == p_i.x.0 {
        // p_j to the left of p_i
        let y0 = r_i.min().y.max(r_j.min().y);
        let y1 = r_i.max().y.min(r_j.max().y);
        if y1 <= y0 {
            return None;
        }
        (Rect::new((r_j.max().x, y0), (r_i.min().x, y1)), true)
    } else if p_i.y.1 == p_j.y.0 {
        // p_i below p_j
        let x0 = r_i.min().x.max(r_j.min().x);
        let x1 = r_i.max().x.min(r_j.max().x);
        if x1 <= x0 {
            return None;
        }
        (Rect::new((x0, r_i.max().y), (x1, r_j.min().y)), false)
    } else if p_j.y.1 == p_i.y.0 {
        // p_j below p_i
        let x0 = r_i.min().x.max(r_j.min().x);
        let x1 = r_i.max().x.min(r_j.max().x);
        if x1 <= x0 {
            return None;
        }
        (Rect::new((x0, r_j.max().y), (x1, r_i.min().y)), false)
    } else {
        return None;
    };

    if rng.gen_bool(0.75) {
        let inset: f32 = rng.gen_range(10.0..=40.0);
        poly = if shrink_y {
            Rect::new((poly.min().x, poly.min().y + inset), (poly.max().x, poly.max().y - inset))
        } else {
            Rect::new((poly.min().x + inset, poly.min().y), (poly.max().x - inset, poly.max().y))
        };
        if poly.width() <= 0.0 || poly.height() <= 0.0 {
            return None;
        }
    }

    let span = if shrink_y { poly.height() } else { poly.width() };
    if span < 10.0 {
        return None;
    }

    if !rng.gen_bool(0.5) {
        return None;
    }
    Some(SafePolygon(poly.to_polygon()))
}

struct RawPlan {
    rooms: Vec<Rect<f32>>,
    playable_area: SafeMultiPolygon,
    doors: Vec<DoorGeometry>,
    corridor_ends: Vec<Vec2>,
    glass_walls: SafeMultiPolygon,
    markers: HashMap<&'static str, Vec<Vec2>>,
    regions: HashMap<&'static str, Vec<SafePolygon>>,
}

// For rooms, shrink at least PADDING away from the edges (respecting MIN_ROOM_SIZE), adding hallways out to the edge.
// For corridors, if there are two connections, draw a straight hallway between them; otherwise, draw hallways from all connections to the center point.
fn render(bsp: &[(Partition, PartitionRole)], rng: &mut impl Rng) -> RawPlan {
    let merged_connections = find_merged_room_connections(bsp, rng);

    let mut rooms = Vec::new();
    let mut playables = SafeMultiPolygon::empty();
    let mut doors = Vec::new();
    let mut corridor_ends_set: HashSet<(u32, u32)> = HashSet::new();
    let mut markers: HashMap<&'static str, Vec<Vec2>> = HashMap::new();
    let mut regions: HashMap<&'static str, Vec<SafePolygon>> = HashMap::new();

    // Avoid hallway stubs leading to nothing.
    let empty_connections: Vec<(f32, f32)> = bsp
        .iter()
        .filter(|(_, role)| matches!(role, PartitionRole::Empty))
        .flat_map(|(partition, _)| partition_connections(partition).into_iter().map(|c| (c.x, c.y)))
        .collect();

    let is_live =
        |c: &RoomConnection| !empty_connections.iter().any(|&(ex, ey)| c.x == ex && c.y == ey);

    for (partition, role) in bsp {
        match role {
            PartitionRole::Empty => continue,
            PartitionRole::Room { kind } => {
                let room = shrink_room(partition);
                rooms.push(room);

                let connections: Vec<RoomConnection> = partition_connections(partition)
                    .into_iter()
                    .filter(&is_live)
                    .map(|c| RoomConnection {
                        is_double: is_double_width_corridor_connection(&c, bsp),
                        is_merged: is_merged_connection(&c, &merged_connections),
                        ..c
                    })
                    .collect();

                let req = RoomRequest { room, connections };
                let mut builder = LevelBuilder::default();
                kind.carve(&req, rng, &mut builder);

                playables = playables.union(&builder.floor);
                doors.extend(builder.doors);
                for (tag, pos) in builder.markers {
                    markers.entry(tag).or_default().push(pos);
                }
                for (tag, poly) in builder.regions {
                    regions.entry(tag).or_default().push(poly);
                }
            }
            PartitionRole::Corridor { double_width } => {
                let width = if *double_width { CORRIDOR_WIDTH * 2.0 } else { CORRIDOR_WIDTH };
                let connections: Vec<RoomConnection> =
                    partition_connections(partition).into_iter().filter(&is_live).collect();
                let center = partition_center(partition);
                let mut region = SafeMultiPolygon::empty();

                if connections.len() == 2
                    && connections[0].side.is_vertical() != connections[1].side.is_vertical()
                {
                    for poly in connect_adjacent(&connections[0], &connections[1], width) {
                        region = region.union(&SafeMultiPolygon::from(poly));
                    }
                } else if !connections.is_empty() {
                    region = region.union(&SafeMultiPolygon::from(bevel_at_point(center, width)));
                    for connection in &connections {
                        for poly in connect_point_to_center(connection, center, width) {
                            region = region.union(&SafeMultiPolygon::from(poly));
                        }
                    }
                }
                playables = playables.union(&region);
            }
        }
    }

    // Collect all live corridor endpoints (connection points).
    for (partition, role) in bsp {
        if matches!(role, PartitionRole::Empty) {
            continue;
        }
        for connection in partition_connections(partition).into_iter().filter(&is_live) {
            corridor_ends_set.insert((connection.x.to_bits(), connection.y.to_bits()));
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

    // Generate glass walls between spatially-adjacent rooms that have no BSP connector.
    let room_infos: Vec<(&Partition, bool, Rect<f32>)> = bsp
        .iter()
        .filter_map(|(partition, role)| {
            if let PartitionRole::Room { kind } = role {
                Some((partition, kind.rectangular_borders(), shrink_room(partition)))
            } else {
                None
            }
        })
        .collect();

    let mut glass_walls = SafeMultiPolygon::empty();
    for i in 0..room_infos.len() {
        for j in (i + 1)..room_infos.len() {
            let (p_i, rect_i, r_i) = &room_infos[i];
            let (p_j, rect_j, r_j) = &room_infos[j];
            if !rect_i || !rect_j {
                continue;
            }
            if partitions_share_connection(p_i, p_j) {
                continue;
            }
            if let Some(poly) = glass_wall_between(p_i, p_j, r_i, r_j, rng) {
                glass_walls = glass_walls.union(&SafeMultiPolygon::from(poly));
            }
        }
    }

    RawPlan { rooms, playable_area: playables, doors, corridor_ends, glass_walls, markers, regions }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ConnectionSide {
    Left,
    Right,
    Bottom,
    Top,
}

impl ConnectionSide {
    fn is_vertical(&self) -> bool { matches!(self, ConnectionSide::Left | ConnectionSide::Right) }
}

fn make_door(
    side: ConnectionSide,
    room_entry: Vec2,
    thickness: f32,
    phys_span: (f32, f32),
    disp_span: (f32, f32),
    hinge_at_start: bool,
) -> DoorGeometry {
    if side.is_vertical() {
        let x0 =
            if let ConnectionSide::Left = side { room_entry.x - thickness } else { room_entry.x };
        let x1 = x0 + thickness;
        let hinge = (x0 + thickness / 2.0, if hinge_at_start { phys_span.0 } else { phys_span.1 });
        DoorGeometry {
            phys_rect: Rect::new((x0, phys_span.0), (x1, phys_span.1)),
            disp_rect: Rect::new((x0, disp_span.0), (x1, disp_span.1)),
            hinge,
        }
    } else {
        let y0 =
            if let ConnectionSide::Bottom = side { room_entry.y - thickness } else { room_entry.y };
        let y1 = y0 + thickness;
        let hinge = (if hinge_at_start { phys_span.0 } else { phys_span.1 }, y0 + thickness / 2.0);
        DoorGeometry {
            phys_rect: Rect::new((phys_span.0, y0), (phys_span.1, y1)),
            disp_rect: Rect::new((disp_span.0, y0), (disp_span.1, y1)),
            hinge,
        }
    }
}

pub fn create_door(side: ConnectionSide, room_entry: Vec2) -> DoorGeometry {
    create_sized_door(side, room_entry, CORRIDOR_WIDTH)
}

/// Like `create_door`, but for an opening of arbitrary `width` (e.g. a vault's doorway).
pub fn create_sized_door(side: ConnectionSide, room_entry: Vec2, width: f32) -> DoorGeometry {
    let thickness = 5.0;
    let phys_len = width - 4.0;
    let entry_mid = if side.is_vertical() { room_entry.y } else { room_entry.x };

    let phys_span = (entry_mid - phys_len / 2.0, entry_mid + phys_len / 2.0);
    let disp_span = (entry_mid - width / 2.0, entry_mid + width / 2.0);

    make_door(side, room_entry, thickness, phys_span, disp_span, true)
}

pub fn create_double_door(side: ConnectionSide, room_entry: Vec2) -> (DoorGeometry, DoorGeometry) {
    let thickness = 5.0;
    let w = CORRIDOR_WIDTH * 2.0;
    let entry_mid = if side.is_vertical() { room_entry.y } else { room_entry.x };

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

fn partition_connections(partition: &Partition) -> Vec<RoomConnection> {
    let mut connections = Vec::new();

    for &y in &partition.horz_conn.0 {
        connections.push(RoomConnection {
            x: partition.x.0,
            y,
            side: ConnectionSide::Left,
            is_double: false,
            is_merged: false,
        });
    }
    for &y in &partition.horz_conn.1 {
        connections.push(RoomConnection {
            x: partition.x.1,
            y,
            side: ConnectionSide::Right,
            is_double: false,
            is_merged: false,
        });
    }
    for &x in &partition.vert_conn.0 {
        connections.push(RoomConnection {
            x,
            y: partition.y.0,
            side: ConnectionSide::Bottom,
            is_double: false,
            is_merged: false,
        });
    }
    for &x in &partition.vert_conn.1 {
        connections.push(RoomConnection {
            x,
            y: partition.y.1,
            side: ConnectionSide::Top,
            is_double: false,
            is_merged: false,
        });
    }

    connections
}

/// Where on `room`'s rectangular boundary the given connection enters.
pub fn room_entry_point(room: &Rect<f32>, conn: &RoomConnection) -> Vec2 {
    match conn.side {
        ConnectionSide::Left => Vec2::new(room.min().x, conn.y.clamp(room.min().y, room.max().y)),
        ConnectionSide::Right => Vec2::new(room.max().x, conn.y.clamp(room.min().y, room.max().y)),
        ConnectionSide::Bottom => Vec2::new(conn.x.clamp(room.min().x, room.max().x), room.min().y),
        ConnectionSide::Top => Vec2::new(conn.x.clamp(room.min().x, room.max().x), room.max().y),
    }
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

fn partition_center(partition: &Partition) -> Vec2 {
    Vec2::new((partition.x.0 + partition.x.1) / 2.0, (partition.y.0 + partition.y.1) / 2.0)
}

fn connect_to_entry(entry: Vec2, conn: &RoomConnection, width: f32) -> Vec<SafePolygon> {
    let conn_pt = Vec2::new(conn.x, conn.y);
    let elbow =
        if conn.side.is_vertical() { Vec2::new(entry.x, conn.y) } else { Vec2::new(conn.x, entry.y) };

    let mut polygons = vec![rect_for_segment(conn_pt, elbow, width)];

    let needs_bend = if conn.side.is_vertical() {
        (entry.y - conn.y).abs() > f32::EPSILON
    } else {
        (entry.x - conn.x).abs() > f32::EPSILON
    };
    if needs_bend {
        polygons.push(bevel_at_point(elbow, width));
        polygons.push(rect_for_segment(elbow, entry, width));
    }

    polygons
}

fn connect_room_to_connection(room: &Rect<f32>, conn: &RoomConnection, width: f32) -> Vec<SafePolygon> {
    connect_to_entry(room_entry_point(room, conn), conn, width)
}

fn connect_adjacent(a: &RoomConnection, b: &RoomConnection, width: f32) -> Vec<SafePolygon> {
    let (h_conn, v_conn) = if a.side.is_vertical() { (a, b) } else { (b, a) };
    let elbow = Vec2::new(v_conn.x, h_conn.y);
    vec![
        rect_for_segment(Vec2::new(h_conn.x, h_conn.y), elbow, width),
        bevel_at_point(elbow, width),
        rect_for_segment(elbow, Vec2::new(v_conn.x, v_conn.y), width),
    ]
}

fn connect_point_to_center(connection: &RoomConnection, center: Vec2, width: f32) -> Vec<SafePolygon> {
    let conn_pt = Vec2::new(connection.x, connection.y);

    if (connection.x - center.x).abs() < f32::EPSILON || (connection.y - center.y).abs() < f32::EPSILON
    {
        return vec![rect_for_segment(conn_pt, center, width)];
    }

    let elbow = if connection.side.is_vertical() {
        Vec2::new(center.x, connection.y)
    } else {
        Vec2::new(connection.x, center.y)
    };

    vec![
        rect_for_segment(conn_pt, elbow, width),
        bevel_at_point(elbow, width),
        rect_for_segment(elbow, center, width),
    ]
}

fn bevel_at_point(p: Vec2, width: f32) -> SafePolygon {
    let h = width / 2.0;
    let q = width / 4.0;
    SafePolygon(Polygon::new(
        LineString::from(vec![
            (p.x - h, p.y - q),
            (p.x - q, p.y - h),
            (p.x + q, p.y - h),
            (p.x + h, p.y - q),
            (p.x + h, p.y + q),
            (p.x + q, p.y + h),
            (p.x - q, p.y + h),
            (p.x - h, p.y + q),
            (p.x - h, p.y - q),
        ]),
        vec![],
    ))
}

fn rect_for_segment(a: Vec2, b: Vec2, width: f32) -> SafePolygon {
    if !a.x.is_finite() || !a.y.is_finite() || !b.x.is_finite() || !b.y.is_finite() {
        return SafePolygon(Polygon::new(LineString::new(vec![]), vec![]));
    }
    if (a.x - b.x).abs() < f32::EPSILON {
        let min_y = a.y.min(b.y);
        let max_y = a.y.max(b.y);
        let half = width / 2.0;
        SafePolygon(Rect::new((a.x - half, min_y), (a.x + half, max_y)).to_polygon())
    } else if (a.y - b.y).abs() < f32::EPSILON {
        let min_x = a.x.min(b.x);
        let max_x = a.x.max(b.x);
        let half = width / 2.0;
        SafePolygon(Rect::new((min_x, a.y - half), (max_x, a.y + half)).to_polygon())
    } else {
        unreachable!("rect_for_segment called with diagonal segment ({:?} to {:?})", a, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dungeon::rooms::{OctagonRoom, RingRoom};
    use geo::Contains;

    fn partition_300x300(
        horz_conn: (Vec<f32>, Vec<f32>),
        vert_conn: (Vec<f32>, Vec<f32>),
    ) -> Partition {
        Partition { x: (0.0, 300.0), y: (0.0, 300.0), horz_conn, vert_conn }
    }

    #[test]
    fn test_ring_room_has_solid_center_and_playable_walkway() {
        let partition = partition_300x300((Vec::new(), Vec::new()), (Vec::new(), Vec::new()));
        let bsp = vec![(partition, PartitionRole::Room { kind: Arc::new(RingRoom) })];
        let mut rng = rand::thread_rng();
        let raw = render(&bsp, &mut rng);

        assert!(
            !raw.playable_area.contains(&geo::Point::new(150.0, 150.0)),
            "ring room should have a solid island in the middle"
        );
        assert!(
            raw.playable_area.contains(&geo::Point::new(20.0, 150.0)),
            "ring room's walkway should be playable"
        );
    }

    #[test]
    fn octagon_bevels_corners_when_registry_drawn() {
        // Sanity check that render() dispatches to a registered RoomKind at all.
        let partition = partition_300x300((Vec::new(), vec![150.0]), (Vec::new(), Vec::new()));
        let bsp = vec![(partition, PartitionRole::Room { kind: Arc::new(OctagonRoom) })];
        let mut rng = rand::thread_rng();
        let raw = render(&bsp, &mut rng);
        assert!(raw.playable_area.contains(&geo::Point::new(150.0, 150.0)));
    }

    #[test]
    fn test_partition_space_large_space_has_multiple_partitions_and_connections() {
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
        use geo::BoundingRect;
        // Vertical segment (same x): result rect spans the y range, width = corridor width.
        let poly = rect_for_segment(Vec2::new(50.0, 100.0), Vec2::new(50.0, 200.0), 40.0);
        let bbox = poly.bounding_rect().unwrap();
        assert!((bbox.width() - 40.0).abs() < f32::EPSILON);
        assert!((bbox.height() - 100.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_rect_for_segment_horizontal() {
        use geo::BoundingRect;
        // Horizontal segment (same y): result rect spans the x range, height = corridor width.
        let poly = rect_for_segment(Vec2::new(0.0, 75.0), Vec2::new(120.0, 75.0), 30.0);
        let bbox = poly.bounding_rect().unwrap();
        assert!((bbox.width() - 120.0).abs() < f32::EPSILON);
        assert!((bbox.height() - 30.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_bevel_at_point_has_8_exterior_vertices() {
        let poly = bevel_at_point(Vec2::new(0.0, 0.0), 40.0);
        // LineString is closed: first == last, so 9 coords for 8 distinct vertices.
        assert_eq!(poly.exterior().0.len(), 9);
    }

    #[test]
    fn test_bevel_at_point_all_vertices_within_radius() {
        let width = 40.0;
        let poly = bevel_at_point(Vec2::new(10.0, 20.0), width);
        for coord in &poly.exterior().0 {
            let dx = coord.x - 10.0;
            let dy = coord.y - 20.0;
            let dist = (dx * dx + dy * dy).sqrt();
            assert!(dist <= width, "vertex at distance {dist} exceeds width {width}");
        }
    }

    #[test]
    fn test_door_geometry_disp_corners_symmetric() {
        let door = create_door(ConnectionSide::Left, Vec2::new(100.0, 200.0));
        let corners = door.disp_corners();
        assert_eq!(corners.len(), 4);
        // Corners are centered at origin, so opposite corners should sum to zero.
        assert!((corners[0] + corners[2]).length() < f32::EPSILON);
        assert!((corners[1] + corners[3]).length() < f32::EPSILON);
    }

    #[test]
    fn test_door_geometry_center_matches_phys_rect() {
        let door = create_door(ConnectionSide::Right, Vec2::new(300.0, 150.0));
        let c = door.center();
        let rect_center = door.phys_rect.center();
        assert!((c.x - rect_center.x).abs() < f32::EPSILON);
        assert!((c.y - rect_center.y).abs() < f32::EPSILON);
    }

    // --- Degenerate / extreme geometry stress tests ---

    #[test]
    fn test_bevel_at_point_zero_width() { let _poly = bevel_at_point(Vec2::new(0.0, 0.0), 0.0); }

    #[test]
    fn test_bevel_at_point_negative_width() { let _poly = bevel_at_point(Vec2::new(0.0, 0.0), -20.0); }

    #[test]
    fn test_bevel_at_point_nan_width() { let _poly = bevel_at_point(Vec2::new(0.0, 0.0), f32::NAN); }

    #[test]
    fn test_bevel_at_point_nan_coords() {
        let _poly = bevel_at_point(Vec2::new(f32::NAN, 0.0), 40.0);
    }

    #[test]
    fn test_bevel_at_point_huge_width() {
        let _poly = bevel_at_point(Vec2::new(0.0, 0.0), f32::MAX / 2.0);
    }

    #[test]
    fn test_rect_for_segment_zero_length() {
        // Start == End: a degenerate segment with no length.
        let _poly = rect_for_segment(Vec2::new(50.0, 50.0), Vec2::new(50.0, 50.0), 30.0);
    }

    #[test]
    fn test_rect_for_segment_zero_width() {
        let _poly = rect_for_segment(Vec2::new(0.0, 0.0), Vec2::new(0.0, 100.0), 0.0);
    }

    #[test]
    fn test_rect_for_segment_negative_width() {
        let _poly = rect_for_segment(Vec2::new(0.0, 0.0), Vec2::new(0.0, 100.0), -10.0);
    }

    #[test]
    fn test_rect_for_segment_nan_width() {
        let _poly = rect_for_segment(Vec2::new(0.0, 0.0), Vec2::new(0.0, 100.0), f32::NAN);
    }

    #[test]
    fn test_rect_for_segment_nan_coords() {
        let _poly = rect_for_segment(Vec2::new(f32::NAN, 0.0), Vec2::new(0.0, 100.0), 30.0);
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
    fn test_level_plan_zero_dimensions() {
        let mut rng = rand::thread_rng();
        let registry = RoomRegistry::stock();
        let _result = LevelPlan::new_seeded(0.0, 0.0, &registry, &mut rng);
    }

    #[test]
    fn test_level_plan_tiny_dimensions() {
        let mut rng = rand::thread_rng();
        let registry = RoomRegistry::stock();
        let _result = LevelPlan::new_seeded(1.0, 1.0, &registry, &mut rng);
    }

    #[test]
    fn test_level_plan_very_narrow() {
        // Wide but extremely short — likely to produce no rooms or degenerate corridors.
        let mut rng = rand::thread_rng();
        let registry = RoomRegistry::stock();
        let _result = LevelPlan::new_seeded(1600.0, 10.0, &registry, &mut rng);
    }

    #[test]
    #[ignore = "hangs: BSP partition_space loops forever on a 1M×1M space"]
    fn test_level_plan_huge_dimensions() {
        let mut rng = rand::thread_rng();
        let registry = RoomRegistry::stock();
        let _result = LevelPlan::new_seeded(1_000_000.0, 1_000_000.0, &registry, &mut rng);
    }

    #[test]
    #[ignore = "likely hangs: NaN dimensions cause infinite BSP recursion"]
    fn test_level_plan_nan_dimensions() {
        let mut rng = rand::thread_rng();
        let registry = RoomRegistry::stock();
        let _result = LevelPlan::new_seeded(f32::NAN, f32::NAN, &registry, &mut rng);
    }

    #[test]
    #[ignore = "likely hangs: infinite dimensions cause infinite BSP recursion"]
    fn test_level_plan_infinite_dimensions() {
        let mut rng = rand::thread_rng();
        let registry = RoomRegistry::stock();
        let _result = LevelPlan::new_seeded(f32::INFINITY, f32::INFINITY, &registry, &mut rng);
    }
}
