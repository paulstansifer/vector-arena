use geo::Rect;
use rand::RngCore;

use crate::dungeon::bsp::CORRIDOR_WIDTH;
use crate::dungeon::level_generation::{
    DOOR_PROB, DOUBLE_DOOR_PROB, LevelBuilder, LevelContext, RoomKind, RoomRequest,
};

/// A rectangular room whose interior (inset by one corridor-width from every wall) is
/// tagged as a `"slow"` region — the game maps that to whatever status effect it wants
/// (e.g. this game's Torpor) rather than the engine knowing about slowness at all.
pub struct SlowZoneRoom;

impl RoomKind for SlowZoneRoom {
    fn name(&self) -> &'static str { "slow_zone" }

    fn weight(&self, _ctx: &LevelContext) -> f32 { 8.0 / 6.0 }

    fn carve(&self, req: &RoomRequest, rng: &mut dyn RngCore, out: &mut LevelBuilder) {
        let room = &req.room;
        out.add_floor(room.to_polygon());
        for conn in &req.connections {
            out.carve_connection(room, conn, rng, DOOR_PROB, DOUBLE_DOOR_PROB);
        }

        let inner = Rect::new(
            (room.min().x + CORRIDOR_WIDTH, room.min().y + CORRIDOR_WIDTH),
            (room.max().x - CORRIDOR_WIDTH, room.max().y - CORRIDOR_WIDTH),
        );
        if inner.width() > 0.0 && inner.height() > 0.0 {
            out.add_region("slow", crate::util::safegeo::SafePolygon(inner.to_polygon()));
        }
    }
}
