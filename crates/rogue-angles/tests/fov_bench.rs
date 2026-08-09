//! Timing breakdown of one FOV rebuild, for deciding what is worth caching.
//!
//! `#[ignore]`d: it asserts nothing and its numbers depend on the machine, so it is a
//! measurement tool rather than a test. Run it with:
//!
//! ```text
//! cargo test -p rogue-angles --release --test fov_bench -- --ignored --nocapture
//! ```
//!
//! On a typical seed the split is roughly: raycast ~0.1 ms, the buffer pair that produces
//! `VisibleArea` ~0.5 ms, whole update ~2 ms. That is why `update_fov` is gated on its inputs
//! actually changing, and why `VisibleArea` is published for reuse instead of each consumer
//! re-deriving it.

use bevy::math::Vec2;
use rand::{SeedableRng, rngs::StdRng};
use rogue_angles::{
    WorldBounds,
    dungeon::level_generation::{LevelPlan, RoomRegistry},
    fov::{WALL_FOV_DEPTH, fov_arc, update_fov_from_pov},
    util::safegeo::SafeMultiPolygon,
};
use std::time::Instant;

#[test]
#[ignore = "benchmark, not a pass/fail test"]
fn fov_update_cost_breakdown() {
    let mut rng = StdRng::seed_from_u64(42);
    let registry = RoomRegistry::stock();
    let plan = LevelPlan::new_seeded(1280.0, 720.0, &registry, &mut rng);
    let segments: usize = plan
        .solid_rock
        .iter()
        .flat_map(|p| std::iter::once(p.exterior()).chain(p.interiors()))
        .map(|ls| ls.lines().count())
        .sum();
    println!("solid_rock segments: {segments}");

    let bounds = WorldBounds { width: 1280.0, height: 720.0 };
    let bg = geo::Rect::new((-840.0f32, -560.0), (840.0, 560.0));
    let mut exploration = SafeMultiPolygon::from(geo::MultiPolygon::new(vec![bg.to_polygon()]));

    // Walk a little first, so the exploration polygon is in a realistic partly-revealed state
    // rather than the trivial full rectangle.
    for i in 0..20 {
        let p = Vec2::new(-200.0 + i as f32 * 20.0, 0.0);
        exploration =
            update_fov_from_pov(p, 600.0, &plan.solid_rock, &bounds, &exploration).exploration;
    }

    let origin = Vec2::ZERO;
    let n = 20;

    let t = Instant::now();
    for _ in 0..n {
        let _ = fov_arc(origin, 600.0, None, &plan.solid_rock);
    }
    println!("fov_arc (raycast):     {:?}/call", t.elapsed() / n);

    let fov = SafeMultiPolygon::from(fov_arc(origin, 600.0, None, &plan.solid_rock));
    let t = Instant::now();
    for _ in 0..n {
        let _ = fov.buffer(-1.0).buffer(1.0 + WALL_FOV_DEPTH);
    }
    println!("VisibleArea buffers:   {:?}/call", t.elapsed() / n);

    let t = Instant::now();
    for _ in 0..n {
        let _ = update_fov_from_pov(origin, 600.0, &plan.solid_rock, &bounds, &exploration);
    }
    println!("update_fov_from_pov:   {:?}/call", t.elapsed() / n);
}
