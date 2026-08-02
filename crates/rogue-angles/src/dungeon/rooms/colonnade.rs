use geo::Rect;
use rand::RngCore;

use crate::dungeon::bsp::CORRIDOR_WIDTH;
use crate::dungeon::level_generation::{LevelBuilder, LevelContext, RoomKind, RoomRequest};

/// A rectangular room with a colonnade of square columns down each long wall. No doors —
/// the columns themselves break sightlines.
pub struct ColonnadeRoom;

impl RoomKind for ColonnadeRoom {
    fn name(&self) -> &'static str { "colonnade" }

    fn weight(&self, _ctx: &LevelContext) -> f32 { 8.0 / 6.0 }

    fn carve(&self, req: &RoomRequest, _rng: &mut dyn RngCore, out: &mut LevelBuilder) {
        out.add_floor(req.room.to_polygon());
        for conn in &req.connections {
            out.carve_connection_geometry_only(&req.room, conn);
        }
        for col in colonnade_columns(&req.room) {
            out.subtract_floor(col.to_polygon());
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
