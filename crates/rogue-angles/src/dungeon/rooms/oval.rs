use bevy::math::Vec2;
use geo::{LineString, Polygon, Rect};
use rand::RngCore;

use crate::dungeon::bsp::CORRIDOR_WIDTH;
use crate::dungeon::level_generation::{LevelBuilder, LevelContext, RoomConnection, RoomKind, RoomRequest};
use crate::util::safegeo::SafePolygon;

/// An elliptical room. Weighted at 0 in the stock registry — excluded from random
/// selection pending investigation of a performance issue with its polygon — but usable
/// by a game that selects it explicitly (e.g. via a custom `RoomKind::weight`, or by
/// registering a second copy with nonzero weight).
pub struct OvalRoom;

impl RoomKind for OvalRoom {
    fn name(&self) -> &'static str { "oval" }

    fn weight(&self, _ctx: &LevelContext) -> f32 { 0.0 }

    fn rectangular_borders(&self) -> bool { false }

    fn carve(&self, req: &RoomRequest, _rng: &mut dyn RngCore, out: &mut LevelBuilder) {
        let room = &req.room;
        out.add_floor(oval_polygon(room));
        for conn in &req.connections {
            if conn.is_merged {
                out.carve_full_wall_opening(room, conn);
            } else {
                let width = if conn.is_double { CORRIDOR_WIDTH * 2.0 } else { CORRIDOR_WIDTH };
                let entry = oval_entry_point(room, conn, width);
                out.carve_entry(entry, conn, width);
            }
        }
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
fn oval_entry_point(room: &Rect<f32>, conn: &RoomConnection, corridor_width: f32) -> Vec2 {
    use crate::dungeon::level_generation::ConnectionSide;

    let cx = (room.min().x + room.max().x) / 2.0;
    let cy = (room.min().y + room.max().y) / 2.0;
    let a = room.width() / 2.0;
    let b = room.height() / 2.0;
    let hw = corridor_width / 2.0;

    match conn.side {
        ConnectionSide::Left | ConnectionSide::Right => {
            // Corridor spans y' in [conn.y - hw, conn.y + hw].
            // oval left boundary at y': cx - a*sqrt(1 - ((y'-cy)/b)^2)
            // The boundary is ∪-shaped, so max (deepest into room) is at the endpoints.
            // Equivalently: the half-width term x_at(y') = a*sqrt(...) is ∩-shaped,
            // so min(x_at) is at the endpoints of the span.
            let y_lo = (conn.y - hw).clamp(cy - b, cy + b);
            let y_hi = (conn.y + hw).clamp(cy - b, cy + b);
            let x_at = |y: f32| {
                let t = (y - cy) / b;
                a * (1.0 - t * t).max(0.0).sqrt()
            };
            let min_extent = x_at(y_lo).min(x_at(y_hi)) - 2.0;
            let x = if matches!(conn.side, ConnectionSide::Left) { cx - min_extent } else { cx + min_extent };
            Vec2::new(x, conn.y.clamp(cy - b, cy + b))
        }
        ConnectionSide::Bottom | ConnectionSide::Top => {
            let x_lo = (conn.x - hw).clamp(cx - a, cx + a);
            let x_hi = (conn.x + hw).clamp(cx - a, cx + a);
            let y_at = |x: f32| {
                let t = (x - cx) / a;
                b * (1.0 - t * t).max(0.0).sqrt()
            };
            let min_extent = y_at(x_lo).min(y_at(x_hi)) - 2.0;
            let y = if matches!(conn.side, ConnectionSide::Bottom) { cy - min_extent } else { cy + min_extent };
            Vec2::new(conn.x.clamp(cx - a, cx + a), y)
        }
    }
}
