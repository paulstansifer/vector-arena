// The staircase fog-of-war copy trick is specific to this game's multi-level
// descent mechanic (Staircase/StaircaseFogCopy), so it stays here rather than
// in the engine's FOV module — the engine only knows about generic FOV/
// exploration state, not about staircases.
use bevy::prelude::*;
use rogue_angles::{fov::VisibleArea, util::safegeo::SafeMultiPolygon};

use crate::{Staircase, StaircaseFogCopy};

/// Computes the staircase's bounding square minus the currently-lit area and writes the result
/// into the fog copy's mesh. When fully visible the result is empty (nothing renders); when
/// fully in fog it's the full square; at the boundary it's exact.
///
/// Reads the engine's `VisibleArea` rather than re-deriving it from `CurrentFovState`. This
/// used to recompute `current_fov.0.buffer(-1.0).buffer(1.0 + WALL_FOV_DEPTH)` itself — bit-for-
/// bit the same buffer pair `update_fov_from_pov` had already done that frame, and by far the
/// most expensive part of the whole FOV update (~0.5 ms on a normal level) — to shade a 22×22
/// unit square. Worse, it ran unconditionally every frame, including the (common) frames where
/// the FOV didn't change at all; the `run_if` below now skips those too.
pub fn update_staircase_fog_copy(
    staircase_query: Query<&Transform, With<Staircase>>,
    fog_copy_query: Query<&Mesh2d, With<StaircaseFogCopy>>,
    mut meshes: ResMut<Assets<Mesh>>,
    visible_area: Res<VisibleArea>,
) {
    let Ok(stair_tf) = staircase_query.single() else { return };
    let Ok(fog_mesh_2d) = fog_copy_query.single() else { return };

    let pos = stair_tf.translation.truncate();
    let half = 11.0_f32;
    let staircase_sq = SafeMultiPolygon::from(
        geo::Rect::new((pos.x - half, pos.y - half), (pos.x + half, pos.y + half)).to_polygon(),
    );
    let fog_area = staircase_sq.difference(&visible_area.0);

    if let Some(fog_mesh) = meshes.get_mut(&fog_mesh_2d.0) {
        *fog_mesh = rogue_angles::dungeon::terrain::geometry_to_mesh(&fog_area);
    }
}

/// The fog copy only needs redrawing when the lit area moved or the staircase itself did (it
/// can be displaced by knockback). Both are cheap change-detection checks against work that
/// would otherwise triangulate a fresh mesh every frame.
pub fn staircase_fog_copy_is_stale(
    visible_area: Res<VisibleArea>,
    staircase_query: Query<(), (With<Staircase>, Changed<Transform>)>,
) -> bool {
    visible_area.is_changed() || !staircase_query.is_empty()
}
