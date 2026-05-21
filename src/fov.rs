// Raycasting FOV and cumulative exploration tracking.
// `fov_arc` collects angles to obstacle endpoints (with ±0.01° offsets), adds 64 circle-boundary
// angles, and casts a ray in each direction stopping at the nearest obstacle edge.
// `ExplorationState` accumulates the ever-seen area as a MultiPolygon unioned with the current
// FOV each frame. `Opaque`/`OpaqueVertices` mark sight-blocking entities; doors use local-space
// polygon vertices so they cast correct shadows as they swing.
use bevy::prelude::*;
use geo::{BooleanOps, Buffer, LineString, MultiPolygon, Polygon, Simplify};
use std::ops::Range;

use crate::{
    GameState, WorldBounds,
    dungeon::terrain::{self, DungeonState},
    player::Player,
};

/// Other than solid rock (a special case), this marks things that block line-of-sight
#[derive(Component)]
pub struct Opaque;

/// Local-space polygon vertices for a mobile opaque object.
/// Add this alongside `Opaque` so the FOV system knows the shape to cast shadows from.
#[derive(Component)]
pub struct OpaqueVertices(pub Vec<Vec2>);

/// Calculate a field of view polygon from a start point, optionally restricted to an arc.
pub fn fov_arc(
    origin: Vec2,
    radius: f32,
    angle_range: Option<Range<f32>>,
    obstacles: &geo::MultiPolygon<f32>,
) -> Polygon<f32> {
    let segments: Vec<_> = obstacles
        .iter()
        .flat_map(|poly| std::iter::once(poly.exterior()).chain(poly.interiors()))
        .flat_map(|line_string| line_string.lines())
        .map(|line| (Vec2::new(line.start.x, line.start.y), Vec2::new(line.end.x, line.end.y)))
        .collect();

    let deg_0_01 = 0.01_f32.to_radians();

    // Collect angles to all points from segments
    let mut angles_to_cast: Vec<f32> = segments
        .iter()
        .flat_map(|&(a, b)| [a, b])
        .filter(|&pt| (pt - origin).length_squared() <= (radius + 0.1).powi(2))
        .flat_map(|pt| {
            let diff = pt - origin;
            let angle = diff.y.atan2(diff.x);
            [angle, angle - deg_0_01, angle + deg_0_01]
        })
        .collect();

    // Add fixed angles to make the circle smooth where it hits the radius
    let steps = 64;
    angles_to_cast.extend(
        (0..steps)
            .map(|i| (i as f32 / steps as f32) * std::f32::consts::TAU - std::f32::consts::PI),
    );

    // If angle_range is Some, also push the boundary angles
    if let Some(range) = &angle_range {
        angles_to_cast.push(range.start);
        angles_to_cast.push(range.end);
    }

    // Filter angles based on angle_range and normalize
    let mut sorted_angles: Vec<f32> = if let Some(range) = &angle_range {
        angles_to_cast
            .into_iter()
            .map(|a| (a - range.start).rem_euclid(std::f32::consts::TAU) + range.start)
            .filter(|&a| a <= range.end)
            .collect()
    } else {
        angles_to_cast
            .into_iter()
            .map(|a| {
                (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
            })
            .collect()
    };

    sorted_angles.sort_by(|a, b| a.partial_cmp(b).unwrap());
    sorted_angles.dedup_by(|a, b| (*a - *b).abs() < 1e-5);

    let mut polygon_points = Vec::new();

    // If it's a wedge, start at the origin
    if angle_range.is_some() {
        polygon_points.push((origin.x, origin.y));
    }

    for angle in sorted_angles {
        let dir = Vec2::new(angle.cos(), angle.sin());

        let min_t = segments
            .iter()
            .filter_map(|&(p1, p2)| {
                let s = p2 - p1;
                let r_cross_s = dir.x * s.y - dir.y * s.x;

                if r_cross_s.abs() <= 1e-6 {
                    return None;
                }

                let q_minus_p = p1 - origin;
                let t = (q_minus_p.x * s.y - q_minus_p.y * s.x) / r_cross_s;
                let u = (q_minus_p.x * dir.y - q_minus_p.y * dir.x) / r_cross_s;

                // Use a small epsilon to avoid self-intersection on edges
                if t > 1e-4 && (0.0..=1.0).contains(&u) { Some(t) } else { None }
            })
            .fold(radius, f32::min);

        let hit_point = origin + dir * min_t;
        polygon_points.push((hit_point.x, hit_point.y));
    }

    if !polygon_points.is_empty() {
        let first = polygon_points[0];
        if polygon_points.last() != Some(&first) {
            polygon_points.push(first);
        }
    }

    Polygon::new(LineString::from(polygon_points), vec![])
}

const NEVER_EXPLORED_Z: f32 = 50.0;
pub const TERRAIN_Z: f32 = 40.0;
const NOT_IN_FOV_Z: f32 = 30.0;
pub const MOVABLE_Z: f32 = 20.0;
pub const ON_FLOOR_Z: f32 = 10.0;

#[derive(Component)]
pub struct FovMeshMarker;

#[derive(Component)]
pub struct NeverExploredMeshMarker;

#[derive(Resource)]
pub struct ExplorationState(pub geo::MultiPolygon<f32>);

pub fn spawn_fov_meshes(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    window_width: f32,
    window_height: f32,
) {
    let fov_material = materials.add(ColorMaterial::from(Color::srgb(0.7, 0.7, 0.7)));
    commands.spawn((
        DespawnOnExit(GameState::InLevel),
        FovMeshMarker,
        Mesh2d(
            meshes.add(Mesh::new(bevy_mesh::PrimitiveTopology::TriangleList, Default::default())),
        ),
        MeshMaterial2d(fov_material),
        Transform::from_translation(Vec3::new(0.0, 0.0, NOT_IN_FOV_Z)),
    ));

    let never_explored_material = materials.add(ColorMaterial::from(Color::srgb(0.5, 0.5, 0.5)));
    let w = window_width;
    let h = window_height;
    let bg_rect =
        geo::Rect::new((-w / 2.0 - 200.0, -h / 2.0 - 200.0), (w / 2.0 + 200.0, h / 2.0 + 200.0));
    let bg_poly = MultiPolygon::new(vec![bg_rect.to_polygon()]);
    commands.insert_resource(ExplorationState(bg_poly.clone()));

    commands.spawn((
        DespawnOnExit(GameState::InLevel),
        NeverExploredMeshMarker,
        Mesh2d(meshes.add(terrain::geometry_to_mesh(&bg_poly))),
        MeshMaterial2d(never_explored_material),
        Transform::from_translation(Vec3::new(0.0, 0.0, NEVER_EXPLORED_Z)),
    ));
}

pub fn update_fov(
    player_query: Query<Ref<Transform>, (With<Player>, With<Transform>)>,
    fov_mesh_query: Query<&Mesh2d, With<FovMeshMarker>>,
    never_explored_query: Query<&Mesh2d, (With<NeverExploredMeshMarker>, Without<FovMeshMarker>)>,
    opaque_query: Query<(&GlobalTransform, &OpaqueVertices)>,
    mut meshes: ResMut<Assets<Mesh>>,
    dungeon_state: Res<DungeonState>,
    bounds: Res<WorldBounds>,
    mut exploration_state: ResMut<ExplorationState>,
) {
    let Ok(player_transform) = player_query.single() else { return };
    let fov_mesh_handle = fov_mesh_query.single().unwrap();
    let never_explored_mesh_handle = never_explored_query.single().unwrap();

    // TODO: Why doesn't
    // `player_query: Query<Transform, (With<Player>, Changed<Transform>)>`
    // fire on the first frame? Then we'd be able to avoid this `if`.
    // TODO: `is_changed()` always seems to be true, even when time isn't passing and physics should be quiescent
    // TODO: ...but we should recalculate this if terrain changes, too.

    if !player_transform.is_changed() && fov_mesh_handle.0.path().is_some() {
        return;
    }

    let origin = player_transform.translation.truncate();
    let radius = 600.0;

    // Append mobile obstacle polygons to solid_rock without unioning (fov_arc only needs segments).
    let mut obstacle_polys: Vec<geo::Polygon<f32>> = dungeon_state.solid_rock.0.clone();
    for (gtransform, opaque_verts) in &opaque_query {
        if opaque_verts.0.len() < 3 {
            continue;
        }
        let mut pts: Vec<(f32, f32)> = opaque_verts
            .0
            .iter()
            .map(|&v| {
                let w = gtransform.transform_point(v.extend(0.0));
                (w.x, w.y)
            })
            .collect();
        pts.push(pts[0]);
        obstacle_polys.push(geo::Polygon::new(geo::LineString::from(pts), vec![]));
    }
    let obstacles = geo::MultiPolygon::new(obstacle_polys);

    let (new_exp, new_fov, new_ne) =
        update_fov_from_pov(origin, radius, &obstacles, &bounds, &exploration_state.0);

    exploration_state.0 = new_exp;
    *meshes.get_mut(&fov_mesh_handle.0).unwrap() = new_fov;
    *meshes.get_mut(&never_explored_mesh_handle.0).unwrap() = new_ne;
}

fn update_fov_from_pov(
    origin: Vec2,
    radius: f32,
    solid_rock: &MultiPolygon<f32>,
    bounds: &WorldBounds,
    exploration: &MultiPolygon<f32>,
) -> (MultiPolygon<f32>, Mesh, Mesh) {
    let fov_poly = fov_arc(origin, radius, None, solid_rock);
    let fov_multi = MultiPolygon::new(vec![fov_poly]);

    let w = bounds.width;
    let h = bounds.height;
    // To be safe, make it larger than bounds
    let bg_rect =
        geo::Rect::new((-w / 2.0 - 200.0, -h / 2.0 - 200.0), (w / 2.0 + 200.0, h / 2.0 + 200.0));
    let bg_poly = MultiPolygon::new(vec![bg_rect.to_polygon()]);

    // The negative buffer is a bit of a hack to remove little erroneous rays that sometimes sneak through the terrain.
    // The positive buffer allows looking at the walls.
    let dark_area = bg_poly.difference(&fov_multi.buffer(-1.0).buffer(5.0));

    (
        exploration.intersection(&dark_area).simplify(1e-1),
        terrain::geometry_to_mesh(&dark_area),
        terrain::geometry_to_mesh(&exploration),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dungeon::{
        bsp::Partition,
        level_generation::{PartitionRole, TerrainGeometry},
    };
    use geo::CoordsIter;
    use rand::{SeedableRng, rngs::StdRng};

    #[test]
    fn test_fov_point_explosion() {
        let mut rng = StdRng::seed_from_u64(1234);

        let left = Partition {
            x: (10.0, 300.0),
            y: (10.0, 300.0),
            horz_conn: (vec![300.0], vec![]),
            vert_conn: (vec![], vec![]),
        };
        let mid = Partition {
            x: (300.0, 500.0),
            y: (10.0, 300.0),
            horz_conn: (vec![500.0], vec![300.0]),
            vert_conn: (vec![], vec![]),
        };
        let right = Partition {
            x: (500.0, 790.0),
            y: (10.0, 300.0),
            horz_conn: (vec![], vec![500.0]),
            vert_conn: (vec![], vec![]),
        };

        let allocated_partitions = vec![
            (left, PartitionRole::Room),
            (mid, PartitionRole::Corridor { double_width: false }),
            (right, PartitionRole::Room),
        ];

        let terrain_geometry = TerrainGeometry::from_partitions_and_roles(
            800.0,
            310.0,
            allocated_partitions,
            &mut rng,
        );

        let world_bounds = WorldBounds { width: 1200.0, height: 800.0 };
        let w = world_bounds.width;
        let h = world_bounds.height;
        let bg_rect = geo::Rect::new(
            (-w / 2.0 - 200.0, -h / 2.0 - 200.0),
            (w / 2.0 + 200.0, h / 2.0 + 200.0),
        );
        let mut exploration_state = ExplorationState(MultiPolygon::new(vec![bg_rect.to_polygon()]));

        let mut points = vec![];
        for i in 0..=20 {
            let t = i as f32 / 20.0;
            let x = 150.0 * (1.0 - t) + 650.0 * t;
            let y = 150.0;
            points.push(Vec2::new(x, y));
        }

        for pt in &points {
            let (new_exp, _, _) = update_fov_from_pov(
                *pt,
                600.0,
                &terrain_geometry.solid_rock,
                &world_bounds,
                &exploration_state.0,
            );
            exploration_state.0 = new_exp;
        }

        let pass_1_points: usize = exploration_state.0.coords_count();

        for extra_pass in 0..100 {
            for pt in &points {
                let (new_exp, _, _) = update_fov_from_pov(
                    Vec2::new(pt.x + 0.01 * (extra_pass as f32), pt.y + 0.01 * (extra_pass as f32)),
                    600.0,
                    &terrain_geometry.solid_rock,
                    &world_bounds,
                    &exploration_state.0,
                );
                exploration_state.0 = new_exp;
            }
        }

        let pass_2_points: usize = exploration_state.0.coords_count();
        assert!(pass_1_points < 500, "{}", pass_1_points);
        let extra_points = pass_2_points - pass_1_points;
        assert!(extra_points < 5, "{}", extra_points);
    }
}
