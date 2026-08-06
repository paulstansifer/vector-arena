use geo::Rect;
use rand::RngCore;

use crate::dungeon::bsp::CORRIDOR_WIDTH;
use crate::dungeon::level_generation::{
    DOOR_PROB, DOUBLE_DOOR_PROB, LevelBuilder, LevelContext, RoomKind, RoomRequest,
};
use crate::util::safegeo::SafeMultiPolygon;

/// Thickness of the walkway (and the solid island it encloses).
const RING_THICKNESS: f32 = CORRIDOR_WIDTH;

/// A rectangular ring of walkway around a solid island.
pub struct RingRoom;

impl RoomKind for RingRoom {
    fn name(&self) -> &'static str { "ring" }

    fn weight(&self, _ctx: &LevelContext) -> f32 { 8.0 / 6.0 }

    fn carve(&self, req: &RoomRequest, rng: &mut dyn RngCore, out: &mut LevelBuilder) {
        let room = &req.room;
        let inner = Rect::new(
            (room.min().x + RING_THICKNESS, room.min().y + RING_THICKNESS),
            (room.max().x - RING_THICKNESS, room.max().y - RING_THICKNESS),
        );
        let outer_poly = SafeMultiPolygon::from(room.to_polygon());
        let ring = if inner.width() > 0.0 && inner.height() > 0.0 {
            outer_poly.difference(&SafeMultiPolygon::from(inner.to_polygon()))
        } else {
            outer_poly
        };
        out.add_floor(ring);

        for conn in &req.connections {
            out.carve_connection(room, conn, rng, DOOR_PROB, DOUBLE_DOOR_PROB);
        }
    }
}
