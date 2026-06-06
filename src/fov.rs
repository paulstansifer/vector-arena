// Raycasting FOV and cumulative exploration tracking.
// `fov_arc` collects angles to obstacle endpoints (with ±0.01° offsets), adds 64 circle-boundary
// angles, and casts a ray in each direction stopping at the nearest obstacle edge.
// `ExplorationState` accumulates the ever-seen area as a MultiPolygon unioned with the current
// FOV each frame. `Opaque`/`OpaqueVertices` mark sight-blocking entities; doors use local-space
// polygon vertices so they cast correct shadows as they swing.
use bevy::prelude::*;
use bevy::asset::RenderAssetUsages;
use bevy_mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy_svg::prelude::{Svg, Svg2d};
use geo::{
    BooleanOps, Buffer, Contains, Intersects, Line as GeoLine, LineString, MultiPolygon, Polygon,
    Simplify,
};
use std::ops::Range;

use crate::{
    FogCopyNeedsInit, GameState, Staircase, StaircaseFogCopy, WorldBounds,
    dungeon::terrain::{self, DungeonState},
    player::Player,
    status_effect::StatusEffects,
};

/// Other than solid rock (a special case), this marks things that block line-of-sight
#[derive(Component)]
pub struct Opaque;

/// Local-space polygon vertices for a mobile opaque object.
/// Add this alongside `Opaque` so the FOV system knows the shape to cast shadows from.
#[derive(Component)]
pub struct OpaqueVertices(pub Vec<Vec2>);

/// Find a point on the boundary of `exploration` (the never-explored area) that:
/// - lies inside `playable_area` (so the navmesh can route there), and
/// - has an unobstructed line-of-sight to `target` through `known_blockers`.
/// Returns the qualifying candidate closest to `target`, or `None` if none exist.
pub fn find_exploration_waypoint(
    target: Vec2,
    exploration: &MultiPolygon<f32>,
    known_blockers: &MultiPolygon<f32>,
    playable_area: &MultiPolygon<f32>,
) -> Option<Vec2> {
    let step = 15.0_f32;

    exploration
        .iter()
        .flat_map(|poly| std::iter::once(poly.exterior()).chain(poly.interiors()))
        .flat_map(|ring| {
            ring.lines().flat_map(move |line| {
                let a = Vec2::new(line.start.x, line.start.y);
                let b = Vec2::new(line.end.x, line.end.y);
                let dist = a.distance(b);
                let n_steps = ((dist / step).ceil() as usize).max(1);
                (0..=n_steps).map(move |i| a.lerp(b, i as f32 / n_steps as f32))
            })
        })
        // `intersects` rather than `contains` because frontier points lie on the shared
        // boundary between playable_area and the exploration polygon — boundary points
        // are excluded by `contains` but included by `intersects`.
        .filter(|&p| playable_area.intersects(&geo::Point::new(p.x, p.y)))
        .filter(|&p| {
            let seg = GeoLine::new(geo::Coord { x: p.x, y: p.y }, geo::Coord {
                x: target.x,
                y: target.y,
            });
            !known_blockers.intersects(&seg)
        })
        .min_by(|&a, &b| {
            a.distance(target).partial_cmp(&b.distance(target)).unwrap_or(std::cmp::Ordering::Equal)
        })
}

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

#[derive(Resource)]
pub struct CurrentFovState(pub geo::MultiPolygon<f32>);

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
    commands.insert_resource(CurrentFovState(MultiPolygon::new(vec![])));

    commands.spawn((
        DespawnOnExit(GameState::InLevel),
        NeverExploredMeshMarker,
        Mesh2d(meshes.add(terrain::geometry_to_mesh(&bg_poly))),
        MeshMaterial2d(never_explored_material),
        Transform::from_translation(Vec3::new(0.0, 0.0, NEVER_EXPLORED_Z)),
    ));
}

pub fn update_fov(
    player_query: Query<(Ref<Transform>, Option<&StatusEffects>), (With<Player>, With<Transform>)>,
    fov_mesh_query: Query<&Mesh2d, With<FovMeshMarker>>,
    never_explored_query: Query<&Mesh2d, (With<NeverExploredMeshMarker>, Without<FovMeshMarker>)>,
    opaque_query: Query<(&GlobalTransform, &OpaqueVertices)>,
    mut meshes: ResMut<Assets<Mesh>>,
    dungeon_state: Res<DungeonState>,
    bounds: Res<WorldBounds>,
    mut exploration_state: ResMut<ExplorationState>,
    mut current_fov: ResMut<CurrentFovState>,
) {
    let Ok((player_transform, player_effects)) = player_query.single() else { return };
    let fov_mesh_handle = fov_mesh_query.single().unwrap();
    let never_explored_mesh_handle = never_explored_query.single().unwrap();

    // TODO: Why doesn't
    // `player_query: Query<Transform, (With<Player>, Changed<Transform>)>`
    // fire on the first frame? Then we'd be able to avoid this `if`.
    // TODO: `is_changed()` always seems to be true, even when time isn't passing and physics should be quiescent
    // TODO: ...but we should recalculate this if terrain changes, too.

    let effects_active = player_effects.map(|e| !e.is_empty()).unwrap_or(false);
    if !player_transform.is_changed() && !effects_active && fov_mesh_handle.0.path().is_some() {
        return;
    }

    let origin = player_transform.translation.truncate();
    let blind_strength = player_effects.map(|e| e.blind_strength()).unwrap_or(0.0);
    let radius = 600.0_f32 + (50.0_f32 - 600.0_f32) * blind_strength;

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

    let (new_exp, new_fov, new_ne, new_fov_poly) = update_fov_from_pov(
        origin,
        radius,
        &obstacles,
        &bounds,
        &exploration_state.0,
    );

    exploration_state.0 = new_exp;
    current_fov.0 = new_fov_poly;
    *meshes.get_mut(&fov_mesh_handle.0).unwrap() = new_fov;
    *meshes.get_mut(&never_explored_mesh_handle.0).unwrap() = new_ne;
}

/// Once the staircase's Svg2d is available, equip the fog copy with its own Mesh2d
/// (empty initially) and the same SVG as its material. After this runs, the fog copy
/// owns its mesh independently from the staircase.
pub fn init_staircase_fog_copies(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    fog_copy_query: Query<Entity, (With<StaircaseFogCopy>, With<FogCopyNeedsInit>)>,
    staircase_query: Query<&Svg2d, With<Staircase>>,
) {
    let Ok(svg2d) = staircase_query.single() else { return };
    let svg_handle = svg2d.0.clone();

    for entity in &fog_copy_query {
        let fog_mesh_handle = meshes.add(Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        ));
        commands.entity(entity)
            .insert((
                Mesh2d(fog_mesh_handle),
                MeshMaterial2d(svg_handle.clone()),
            ))
            .remove::<FogCopyNeedsInit>();
    }
}

/// Each frame, rewrites the fog copy mesh to contain only those triangles from the
/// staircase SVG whose centroid (in world space) lies outside the current FOV polygon.
/// Triangles inside FOV are dropped, so the fog copy never renders over visible geometry.
pub fn update_staircase_fog_copy(
    staircase_query: Query<(&Transform, &Svg2d), With<Staircase>>,
    fog_copy_query: Query<&Mesh2d, (With<StaircaseFogCopy>, Without<FogCopyNeedsInit>)>,
    svgs: Res<Assets<Svg>>,
    mut meshes: ResMut<Assets<Mesh>>,
    current_fov: Res<CurrentFovState>,
) {
    let Ok((stair_tf, svg2d)) = staircase_query.single() else { return };
    let Ok(fog_mesh_2d) = fog_copy_query.single() else { return };
    let Some(svg) = svgs.get(&svg2d.0) else { return };

    // Clone the triangle data out of the source mesh before taking a mutable borrow.
    let (src_positions, src_colors, src_indices) = {
        let src = match meshes.get(&svg.mesh) {
            Some(m) => m,
            None => return,
        };
        let positions = match src.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(VertexAttributeValues::Float32x3(v)) => v.clone(),
            _ => return,
        };
        let colors = match src.attribute(Mesh::ATTRIBUTE_COLOR) {
            Some(VertexAttributeValues::Float32x4(v)) => v.clone(),
            _ => return,
        };
        let indices = match src.indices() {
            Some(Indices::U32(v)) => v.clone(),
            _ => return,
        };
        (positions, colors, indices)
    };

    let mut new_positions: Vec<[f32; 3]> = Vec::new();
    let mut new_colors: Vec<[f32; 4]> = Vec::new();
    let mut new_indices: Vec<u32> = Vec::new();

    let scale = stair_tf.scale.truncate();
    let origin = stair_tf.translation.truncate();

    for tri in src_indices.chunks(3) {
        let [i, j, k] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
        let p0 = Vec2::new(src_positions[i][0], src_positions[i][1]);
        let p1 = Vec2::new(src_positions[j][0], src_positions[j][1]);
        let p2 = Vec2::new(src_positions[k][0], src_positions[k][1]);

        let centroid_local = (p0 + p1 + p2) / 3.0;
        let centroid_world = origin + centroid_local * scale;

        if !current_fov.0.contains(&geo::Point::new(centroid_world.x, centroid_world.y)) {
            let base = new_positions.len() as u32;
            new_positions.extend([src_positions[i], src_positions[j], src_positions[k]]);
            new_colors.extend([src_colors[i], src_colors[j], src_colors[k]]);
            new_indices.extend([base, base + 1, base + 2]);
        }
    }

    if let Some(fog_mesh) = meshes.get_mut(&fog_mesh_2d.0) {
        fog_mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, new_positions);
        fog_mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, new_colors);
        fog_mesh.insert_indices(Indices::U32(new_indices));
    }
}

pub fn update_fov_from_pov(
    origin: Vec2,
    radius: f32,
    solid_rock: &MultiPolygon<f32>,
    bounds: &WorldBounds,
    exploration: &MultiPolygon<f32>,
) -> (MultiPolygon<f32>, Mesh, Mesh, MultiPolygon<f32>) {
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
        fov_multi,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: a MultiPolygon rectangle.
    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> MultiPolygon<f32> {
        MultiPolygon::new(vec![geo::Rect::new((x0, y0), (x1, y1)).to_polygon()])
    }

    /// The frontier of exploration lies exactly on the boundary of `playable_area` (the shared
    /// edge where explored meets unexplored).  `geo::Contains` excludes boundary points, so it
    /// would return None here.  `geo::Intersects` correctly includes them.
    #[test]
    fn test_waypoint_boundary_frontier() {
        // Explored corridor: x=-200..0, y=-50..50
        // Unexplored block: x=0..200, y=-50..50
        // The frontier (left edge of unexplored at x=0) is the right boundary of playable_area.
        // Every sampled point on that edge is on the boundary of playable_area, not its interior.
        let exploration = rect(0.0, -50.0, 200.0, 50.0);
        let playable_area = rect(-200.0, -50.0, 0.0, 50.0);
        let known_blockers = MultiPolygon::new(vec![]);
        let target = Vec2::new(300.0, 0.0);

        let result =
            find_exploration_waypoint(target, &exploration, &known_blockers, &playable_area);
        assert!(result.is_some(), "should find waypoint at corridor entrance boundary");
        let w = result.unwrap();
        assert!(w.x.abs() < 1.0, "waypoint should be on the frontier at x=0, got {w:?}");
    }

    /// Same setup with a very narrow corridor (14 units wide): the frontier edge is too short
    /// for `step=15` to produce any strictly-interior sample — only the two wall-corner endpoints,
    /// both on the boundary of `playable_area`.
    #[test]
    fn test_waypoint_narrow_frontier() {
        let exploration = rect(0.0, -7.0, 200.0, 7.0);
        let playable_area = rect(-200.0, -7.0, 0.0, 7.0);
        let known_blockers = MultiPolygon::new(vec![]);
        let target = Vec2::new(300.0, 0.0);

        let result =
            find_exploration_waypoint(target, &exploration, &known_blockers, &playable_area);
        assert!(result.is_some(), "should find waypoint even at a 14-unit wide corridor entrance");
        let w = result.unwrap();
        assert!(w.x.abs() < 1.0, "waypoint should be at x=0 frontier, got {w:?}");
        assert!(w.y.abs() <= 7.0, "waypoint should be within corridor height, got {w:?}");
    }

    /// Among multiple frontier openings, the function should prefer the one closest to the target.
    #[test]
    fn test_waypoint_picks_closest_frontier() {
        // Two unexplored strips both fronting x=0 from the playable area.
        // Upper strip (y=20..50) is closer to the target at (300, 30) than lower (y=-50..-20).
        let exploration = MultiPolygon::new(vec![
            geo::Rect::new((0.0_f32, -50.0), (200.0_f32, -20.0)).to_polygon(),
            geo::Rect::new((0.0_f32, 20.0), (200.0_f32, 50.0)).to_polygon(),
        ]);
        let playable_area = rect(-200.0, -50.0, 0.0, 50.0);
        let known_blockers = MultiPolygon::new(vec![]);
        let target = Vec2::new(300.0, 30.0);

        let result =
            find_exploration_waypoint(target, &exploration, &known_blockers, &playable_area);
        assert!(result.is_some(), "should find a frontier waypoint");
        let w = result.unwrap();
        assert!(w.y >= 0.0, "should prefer the upper (y>0) strip closer to target y=30, got {w:?}");
    }

    /// When a known wall blocks LOS from every frontier point to the target, return None.
    #[test]
    fn test_waypoint_all_los_blocked() {
        let exploration = rect(0.0, -50.0, 200.0, 50.0);
        let playable_area = rect(-200.0, -50.0, 0.0, 50.0);
        // A thick wall spanning the full corridor height, known to the player.
        let known_blockers = rect(50.0, -200.0, 100.0, 200.0);
        let target = Vec2::new(300.0, 0.0); // behind the wall

        let result =
            find_exploration_waypoint(target, &exploration, &known_blockers, &playable_area);
        assert!(result.is_none(), "should return None when all LOS paths cross a known wall");
    }
}
