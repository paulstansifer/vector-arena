use geo::{LineString, Polygon, Rect};
use rand::RngCore;

use crate::dungeon::level_generation::{
    ConnectionSide, DOOR_PROB, DOUBLE_DOOR_PROB, LevelBuilder, LevelContext, RoomConnection,
    RoomKind, RoomRequest, room_entry_point,
};
use crate::util::safegeo::SafePolygon;

/// Length cut off each wall to form an octagon room's diagonal corners.
const OCTAGON_BEVEL: f32 = 30.0;

/// A rectangular room with diagonally beveled corners, cut only where no connection
/// enters nearby (a bevel too close to a doorway would look wrong).
pub struct OctagonRoom;

impl RoomKind for OctagonRoom {
    fn name(&self) -> &'static str { "octagon" }

    fn weight(&self, _ctx: &LevelContext) -> f32 { 8.0 / 6.0 }

    fn rectangular_borders(&self) -> bool { false }

    fn carve(&self, req: &RoomRequest, rng: &mut dyn RngCore, out: &mut LevelBuilder) {
        let room = &req.room;
        let bevel = octagon_bevel_corners(room, &req.connections);
        out.add_floor(octagon_polygon(room, bevel));
        for conn in &req.connections {
            out.carve_connection(room, conn, rng, DOOR_PROB, DOUBLE_DOOR_PROB);
        }
    }
}

/// Which of a room's four corners (BL, BR, TR, TL) are far enough from any connection's
/// entry point to be safely cut into a diagonal bevel.
fn octagon_bevel_corners(room: &Rect<f32>, connections: &[RoomConnection]) -> [bool; 4] {
    let mut blocked = [false; 4];
    for connection in connections {
        let entry = room_entry_point(room, connection);
        match connection.side {
            ConnectionSide::Left => {
                if entry.y - room.min().y < OCTAGON_BEVEL {
                    blocked[0] = true;
                }
                if room.max().y - entry.y < OCTAGON_BEVEL {
                    blocked[3] = true;
                }
            }
            ConnectionSide::Right => {
                if entry.y - room.min().y < OCTAGON_BEVEL {
                    blocked[1] = true;
                }
                if room.max().y - entry.y < OCTAGON_BEVEL {
                    blocked[2] = true;
                }
            }
            ConnectionSide::Bottom => {
                if entry.x - room.min().x < OCTAGON_BEVEL {
                    blocked[0] = true;
                }
                if room.max().x - entry.x < OCTAGON_BEVEL {
                    blocked[1] = true;
                }
            }
            ConnectionSide::Top => {
                if entry.x - room.min().x < OCTAGON_BEVEL {
                    blocked[3] = true;
                }
                if room.max().x - entry.x < OCTAGON_BEVEL {
                    blocked[2] = true;
                }
            }
        }
    }
    [!blocked[0], !blocked[1], !blocked[2], !blocked[3]]
}

/// Builds a room polygon with diagonal bevels cut into the corners marked `true` in `bevel`
/// (indices: bottom-left, bottom-right, top-right, top-left), each cutting off `OCTAGON_BEVEL`
/// units of the two adjoining walls.
fn octagon_polygon(room: &Rect<f32>, bevel: [bool; 4]) -> SafePolygon {
    let (x0, y0) = (room.min().x, room.min().y);
    let (x1, y1) = (room.max().x, room.max().y);
    let b = OCTAGON_BEVEL.min((x1 - x0) / 2.0).min((y1 - y0) / 2.0).max(0.0);

    let mut coords: Vec<(f32, f32)> = Vec::new();
    if bevel[0] {
        coords.push((x0 + b, y0));
    } else {
        coords.push((x0, y0));
    }
    if bevel[1] {
        coords.push((x1 - b, y0));
        coords.push((x1, y0 + b));
    } else {
        coords.push((x1, y0));
    }
    if bevel[2] {
        coords.push((x1, y1 - b));
        coords.push((x1 - b, y1));
    } else {
        coords.push((x1, y1));
    }
    if bevel[3] {
        coords.push((x0 + b, y1));
        coords.push((x0, y1 - b));
    } else {
        coords.push((x0, y1));
    }
    if bevel[0] {
        coords.push((x0, y0 + b));
    }
    if let Some(&first) = coords.first() {
        coords.push(first);
    }
    SafePolygon(Polygon::new(LineString::from(coords), vec![]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dungeon::level_generation::LevelBuilder;
    use geo::Contains;

    fn room_300() -> Rect<f32> { Rect::new((10.0, 10.0), (290.0, 290.0)) }

    #[test]
    fn bevels_corners_with_no_nearby_connection() {
        // A single connection on the right side, far from every corner.
        let req = RoomRequest {
            room: room_300(),
            connections: vec![RoomConnection {
                x: 290.0,
                y: 150.0,
                side: ConnectionSide::Right,
                is_double: false,
                is_merged: false,
            }],
        };
        let mut builder = LevelBuilder::default();
        let mut rng = rand::thread_rng();
        OctagonRoom.carve(&req, &mut rng, &mut builder);

        assert!(!builder.floor().contains(&geo::Point::new(10.5, 10.5)), "BL corner should be cut");
        assert!(
            !builder.floor().contains(&geo::Point::new(289.5, 10.5)),
            "BR corner should be cut"
        );
        assert!(
            !builder.floor().contains(&geo::Point::new(289.5, 289.5)),
            "TR corner should be cut"
        );
        assert!(
            !builder.floor().contains(&geo::Point::new(10.5, 289.5)),
            "TL corner should be cut"
        );
        assert!(
            builder.floor().contains(&geo::Point::new(150.0, 150.0)),
            "center should stay playable"
        );
    }

    #[test]
    fn skips_bevel_near_connection() {
        // A connection entering right next to the bottom-left corner.
        let req = RoomRequest {
            room: room_300(),
            connections: vec![RoomConnection {
                x: 0.0,
                y: 15.0,
                side: ConnectionSide::Left,
                is_double: false,
                is_merged: false,
            }],
        };
        let mut builder = LevelBuilder::default();
        let mut rng = rand::thread_rng();
        OctagonRoom.carve(&req, &mut rng, &mut builder);

        assert!(
            builder.floor().contains(&geo::Point::new(10.5, 10.5)),
            "BL corner should stay sharp near a connection"
        );
    }
}
