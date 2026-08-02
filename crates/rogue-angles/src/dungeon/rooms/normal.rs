use rand::RngCore;

use crate::dungeon::level_generation::{
    DOOR_PROB, DOUBLE_DOOR_PROB, LevelBuilder, LevelContext, RoomKind, RoomRequest,
};

/// A plain rectangular room. The common case: 92% of generated rooms.
pub struct NormalRoom;

impl RoomKind for NormalRoom {
    fn name(&self) -> &'static str { "normal" }

    fn weight(&self, _ctx: &LevelContext) -> f32 { 92.0 }

    fn carve(&self, req: &RoomRequest, rng: &mut dyn RngCore, out: &mut LevelBuilder) {
        out.add_floor(req.room.to_polygon());
        for conn in &req.connections {
            out.carve_connection(&req.room, conn, rng, DOOR_PROB, DOUBLE_DOOR_PROB);
        }
    }
}
