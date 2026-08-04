use bevy::math::Vec2;
use geo::Rect;
use rand::{Rng, RngCore};

use crate::dungeon::level_generation::{
    ConnectionSide, DOOR_PROB, DOUBLE_DOOR_PROB, LevelBuilder, LevelContext, RoomKind,
    RoomRequest, create_door,
};
use crate::util::safegeo::SafePolygon;

/// Side length of a vault's inner treasure chamber.
const CHAMBER_SIZE: f32 = 45.0;
/// Thickness of the wall separating the vault from the rest of the room.
const CHAMBER_WALL_THICKNESS: f32 = 5.0;
/// Width of the corridor stubs at the room's own connections (matches `CORRIDOR_WIDTH`,
/// duplicated locally since the vault opening's half-width uses the same constant).
const CORRIDOR_WIDTH: f32 = crate::dungeon::bsp::CORRIDOR_WIDTH;

/// A rectangular room with a small walled-off vault in the middle, holding a monster and
/// treasure behind exactly one door. Tags the vault's center as a `"vault_center"` marker
/// for the game to spawn a monster and treasure there.
pub struct VaultRoom;

impl RoomKind for VaultRoom {
    fn name(&self) -> &'static str { "vault" }

    fn weight(&self, _ctx: &LevelContext) -> f32 { 8.0 / 6.0 }

    fn carve(&self, req: &RoomRequest, rng: &mut dyn RngCore, out: &mut LevelBuilder) {
        let room = &req.room;
        out.add_floor(room.to_polygon());
        for conn in &req.connections {
            out.carve_connection(room, conn, rng, DOOR_PROB, DOUBLE_DOOR_PROB);
        }

        let cx = (room.min().x + room.max().x) / 2.0;
        let cy = (room.min().y + room.max().y) / 2.0;
        let half = CHAMBER_SIZE / 2.0;
        let inner = Rect::new((cx - half, cy - half), (cx + half, cy + half));
        let outer_half = half + CHAMBER_WALL_THICKNESS;
        let outer = Rect::new((cx - outer_half, cy - outer_half), (cx + outer_half, cy + outer_half));

        // Only carve the vault if the room comfortably contains it.
        if room.width() > outer.width() + 10.0 && room.height() > outer.height() + 10.0 {
            out.subtract_floor(outer.to_polygon());
            out.add_floor(inner.to_polygon());

            let side = [
                ConnectionSide::Left,
                ConnectionSide::Right,
                ConnectionSide::Bottom,
                ConnectionSide::Top,
            ][Rng::gen_range(rng, 0..4)];
            let (room_entry, opening) = chamber_opening(inner, outer, side);
            out.add_floor(opening);
            out.add_door(create_door(side, room_entry));
            out.add_marker("vault_center", Vec2::new(cx, cy));
        }
    }
}

/// Returns the point on the vault's inner wall where a door should sit, and the polygon
/// of the wall gap (from outer to inner boundary) that must be carved to let the door through.
fn chamber_opening(inner: Rect<f32>, outer: Rect<f32>, side: ConnectionSide) -> (Vec2, SafePolygon) {
    let hw = CORRIDOR_WIDTH / 2.0;
    let cx = (inner.min().x + inner.max().x) / 2.0;
    let cy = (inner.min().y + inner.max().y) / 2.0;

    match side {
        ConnectionSide::Left => {
            let rect = Rect::new((outer.min().x, cy - hw), (inner.min().x, cy + hw));
            (Vec2::new(inner.min().x, cy), SafePolygon(rect.to_polygon()))
        }
        ConnectionSide::Right => {
            let rect = Rect::new((inner.max().x, cy - hw), (outer.max().x, cy + hw));
            (Vec2::new(inner.max().x, cy), SafePolygon(rect.to_polygon()))
        }
        ConnectionSide::Bottom => {
            let rect = Rect::new((cx - hw, outer.min().y), (cx + hw, inner.min().y));
            (Vec2::new(cx, inner.min().y), SafePolygon(rect.to_polygon()))
        }
        ConnectionSide::Top => {
            let rect = Rect::new((cx - hw, inner.max().y), (cx + hw, outer.max().y));
            (Vec2::new(cx, inner.max().y), SafePolygon(rect.to_polygon()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dungeon::level_generation::LevelBuilder;
    use geo::Contains;

    #[test]
    fn carves_vault_with_single_door_and_records_center() {
        let req = RoomRequest { room: Rect::new((10.0, 10.0), (290.0, 290.0)), connections: vec![] };
        let mut builder = LevelBuilder::default();
        let mut rng = rand::thread_rng();
        VaultRoom.carve(&req, &mut rng, &mut builder);

        let markers = builder.markers();
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].0, "vault_center");
        assert!((markers[0].1 - Vec2::new(150.0, 150.0)).length() < f32::EPSILON);
        assert_eq!(builder.doors().len(), 1, "vault should have exactly one door");
        assert!(
            builder.floor().contains(&geo::Point::new(150.0, 150.0)),
            "vault interior should be playable"
        );

        // Exactly one of the four wall-band probes (one per side) should be open: the door side.
        let probes = [
            geo::Point::new(124.5, 150.0), // left wall band
            geo::Point::new(175.5, 150.0), // right wall band
            geo::Point::new(150.0, 124.5), // bottom wall band
            geo::Point::new(150.0, 175.5), // top wall band
        ];
        let open_count = probes.iter().filter(|p| builder.floor().contains(*p)).count();
        assert_eq!(open_count, 1, "exactly one wall of the vault should have a door opening");
    }
}
