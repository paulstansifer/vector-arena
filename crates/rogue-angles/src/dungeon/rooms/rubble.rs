use bevy::math::Vec2;
use geo::{LineString, Polygon};
use rand::{Rng, RngCore};

use crate::dungeon::level_generation::{
    DOOR_PROB, DOUBLE_DOOR_PROB, LevelBuilder, LevelContext, RoomKind, RoomRequest,
};
use crate::util::safegeo::SafePolygon;

/// Number of rubble obstacles scattered around a rubble room.
const RUBBLE_COUNT: usize = 20;
/// Size range (diameter) of a single rubble obstacle.
const RUBBLE_SIZE: std::ops::Range<f32> = 15.0..30.0;
/// Margin kept clear of rubble near a room's walls/connections.
const RUBBLE_MARGIN: f32 = 20.0;

/// A room stays fully open (rubble is spawned as movable obstacles, not carved into the
/// terrain) but scattered with `RUBBLE_COUNT` immovable jagged rock shapes, tagged as the
/// `"rubble"` region for the game to spawn as `Dynamic` bodies.
pub struct RubbleRoom;

impl RoomKind for RubbleRoom {
    fn name(&self) -> &'static str { "rubble" }

    fn weight(&self, _ctx: &LevelContext) -> f32 { 8.0 / 6.0 }

    fn carve(&self, req: &RoomRequest, rng: &mut dyn RngCore, out: &mut LevelBuilder) {
        out.add_floor(req.room.to_polygon());
        for conn in &req.connections {
            out.carve_connection(&req.room, conn, rng, DOOR_PROB, DOUBLE_DOOR_PROB);
        }
        for piece in rubble_piece_polygons(&req.room, rng) {
            out.add_region("rubble", piece);
        }
    }
}

/// Generates ~`RUBBLE_COUNT` immovable jagged rock obstacles scattered through `room`'s
/// interior, kept clear of the walls by `RUBBLE_MARGIN` so they never block a connection.
fn rubble_piece_polygons(room: &geo::Rect<f32>, rng: &mut dyn RngCore) -> Vec<SafePolygon> {
    let min_x = room.min().x + RUBBLE_MARGIN;
    let max_x = room.max().x - RUBBLE_MARGIN;
    let min_y = room.min().y + RUBBLE_MARGIN;
    let max_y = room.max().y - RUBBLE_MARGIN;
    if max_x <= min_x || max_y <= min_y {
        return Vec::new();
    }

    (0..RUBBLE_COUNT)
        .map(|_| {
            let size = Rng::gen_range(rng, RUBBLE_SIZE);
            let center =
                Vec2::new(Rng::gen_range(rng, min_x..max_x), Rng::gen_range(rng, min_y..max_y));
            let angle: f32 = Rng::gen_range(rng, 0.0..std::f32::consts::TAU);
            irregular_blob(center, size / 2.0, angle, rng)
        })
        .collect()
}

/// A small irregular polygon (jittered heptagon) used for rubble obstacles.
fn irregular_blob(center: Vec2, radius: f32, base_angle: f32, rng: &mut dyn RngCore) -> SafePolygon {
    const POINTS: usize = 7;
    let mut coords: Vec<(f32, f32)> = (0..POINTS)
        .map(|i| {
            let angle = base_angle + std::f32::consts::TAU * i as f32 / POINTS as f32;
            let r = radius * Rng::gen_range(rng, 0.6..1.0);
            (center.x + r * angle.cos(), center.y + r * angle.sin())
        })
        .collect();
    if let Some(&first) = coords.first() {
        coords.push(first);
    }
    SafePolygon(Polygon::new(LineString::from(coords), vec![]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dungeon::level_generation::LevelBuilder;
    use geo::{Contains, Rect};

    #[test]
    fn stays_open_and_yields_movable_pieces() {
        let room = Rect::new((10.0, 10.0), (290.0, 290.0));
        let req = RoomRequest { room, connections: vec![] };
        let mut builder = LevelBuilder::default();
        let mut rng = rand::thread_rng();
        RubbleRoom.carve(&req, &mut rng, &mut builder);

        // Rubble is spawned as movable obstacles, not carved into the terrain, so the room
        // stays exactly as open as the plain room rectangle.
        assert!(builder.floor().contains(&geo::Point::new(150.0, 150.0)));
        assert!(builder.floor().contains(&geo::Point::new(15.0, 15.0)));
        let rubble_regions =
            builder.regions().iter().filter(|(tag, _)| *tag == "rubble").count();
        assert_eq!(rubble_regions, RUBBLE_COUNT);
    }
}
