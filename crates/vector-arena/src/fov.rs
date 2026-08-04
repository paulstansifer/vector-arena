// The staircase fog-of-war copy trick is specific to this game's multi-level
// descent mechanic (Staircase/StaircaseFogCopy), so it stays here rather than
// in the engine's FOV module — the engine only knows about generic FOV/
// exploration state, not about staircases.
use bevy::prelude::*;
use rogue_angles::{
    fov::{CurrentFovState, WALL_FOV_DEPTH},
    util::safegeo::SafeMultiPolygon,
};

use crate::{Staircase, StaircaseFogCopy};

/// Each frame, computes the staircase's bounding square minus the current FOV polygon
/// and writes the result into the fog copy's mesh. When fully in FOV the result is empty
/// (nothing renders); when fully in fog it's the full square; at the boundary it's exact.
pub fn update_staircase_fog_copy(
    staircase_query: Query<&Transform, With<Staircase>>,
    fog_copy_query: Query<&Mesh2d, With<StaircaseFogCopy>>,
    mut meshes: ResMut<Assets<Mesh>>,
    current_fov: Res<CurrentFovState>,
) {
    let Ok(stair_tf) = staircase_query.single() else { return };
    let Ok(fog_mesh_2d) = fog_copy_query.single() else { return };

    let pos = stair_tf.translation.truncate();
    let half = 11.0_f32;
    let staircase_sq = SafeMultiPolygon::from(
        geo::Rect::new((pos.x - half, pos.y - half), (pos.x + half, pos.y + half)).to_polygon(),
    );
    // Awkwardly duplicate the hack we do to actually display the FOV:
    let fog_area =
        staircase_sq.difference(&current_fov.0.buffer(-1.0).buffer(1.0 + WALL_FOV_DEPTH));

    if let Some(fog_mesh) = meshes.get_mut(&fog_mesh_2d.0) {
        *fog_mesh = rogue_angles::dungeon::terrain::geometry_to_mesh(&fog_area);
    }
}
