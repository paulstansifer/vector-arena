// Raycasting FOV and cumulative exploration tracking.
// `fov_arc` collects angles to obstacle endpoints (with ±0.01° offsets), adds 64 circle-boundary
// angles, and casts a ray in each direction stopping at the nearest obstacle edge.
// `ExplorationState` accumulates the ever-seen area as a MultiPolygon unioned with the current
// FOV each frame. `Opaque`/`OpaqueVertices` mark sight-blocking entities; doors use local-space
// polygon vertices so they cast correct shadows as they swing.
use bevy::prelude::*;
use geo::{Contains, Intersects, Line as GeoLine, LineString, MultiPolygon, Polygon};
use std::ops::Range;

use crate::{
    LevelEntity, WorldBounds,
    dungeon::terrain::{self, DungeonState},
    movement::{MovementModifiers, Viewer},
    util::safegeo::{SafeMultiPolygon, SafePolygon},
};

pub const WALL_FOV_DEPTH: f32 = 4.0;

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
    exploration: &SafeMultiPolygon,
    known_blockers: &SafeMultiPolygon,
    playable_area: &SafeMultiPolygon,
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
    obstacles: &SafeMultiPolygon,
) -> SafePolygon {
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

    SafePolygon(Polygon::new(LineString::from(polygon_points), vec![]))
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
pub struct ExplorationState(pub SafeMultiPolygon);

impl ExplorationState {
    /// Whether `pos` has ever been seen — `self.0` is the *unexplored* (fog) region, so this
    /// is the negation of containment. The one criterion for "has this point been explored,"
    /// shared by anything that needs it (e.g. deciding which points get a location label, or
    /// which of those labels stay selectable) rather than each caller re-deriving it.
    pub fn is_explored(&self, pos: Vec2) -> bool {
        !self.0.contains(&geo::Point::new(pos.x, pos.y))
    }
}

#[derive(Resource)]
pub struct CurrentFovState(pub SafeMultiPolygon);

/// The current FOV polygon *grown to cover the wall faces bounding it* — i.e.
/// `CurrentFovState` after the negative/positive buffer pair that
/// `update_fov_from_pov` applies before subtracting it from the dark overlay (the negative
/// buffer trims stray slivers that leak between wall segments; the positive one extends
/// sight `WALL_FOV_DEPTH` into the wall so its face is lit rather than the floor stopping
/// at it).
///
/// Published as its own resource because that buffer pair is by far the most expensive step
/// of the FOV update (~0.5 ms on a normal level, vs ~0.1 ms for the raycast itself), and
/// game-side systems want the same polygon: anything drawing "what is lit right now" —
/// vector-arena's staircase fog copy, for one — was recomputing it identically, and every
/// frame rather than only on the frames the FOV actually changed. Read this instead of
/// re-buffering `CurrentFovState`.
#[derive(Resource)]
pub struct VisibleArea(pub SafeMultiPolygon);

impl CurrentFovState {
    /// Whether `pos` is inside the player's *current* field of view — unlike
    /// `ExplorationState::is_explored`, this is momentary: it says nothing about whether the
    /// point has ever been seen before or since. The one containment check shared by anything
    /// that needs to know "is this world point currently visible" (world tooltips, on-screen
    /// entity markers, the palette's target list) rather than each caller re-deriving it.
    pub fn is_visible(&self, pos: Vec2) -> bool { self.0.contains(&geo::Point::new(pos.x, pos.y)) }
}

/// `current_fov` absent (e.g. FOV not wired up, headless/test contexts) is treated as "nothing
/// is hidden" — permissive by default, matching `is_in_fov`/`is_explored`'s existing fallback.
pub fn is_currently_visible(current_fov: Option<&CurrentFovState>, pos: Vec2) -> bool {
    current_fov.is_none_or(|fov| fov.is_visible(pos))
}

pub fn spawn_fov_meshes(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
    window_width: f32,
    window_height: f32,
) {
    let fov_material = materials.add(ColorMaterial::from(Color::srgb(0.7, 0.7, 0.7)));
    commands.spawn((
        LevelEntity,
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
    let bg_poly = SafeMultiPolygon::from(MultiPolygon::new(vec![bg_rect.to_polygon()]));
    commands.insert_resource(ExplorationState(bg_poly.clone()));
    commands.insert_resource(CurrentFovState(SafeMultiPolygon::empty()));
    commands.insert_resource(VisibleArea(SafeMultiPolygon::empty()));

    commands.spawn((
        LevelEntity,
        NeverExploredMeshMarker,
        Mesh2d(meshes.add(terrain::geometry_to_mesh(&bg_poly))),
        MeshMaterial2d(never_explored_material),
        Transform::from_translation(Vec3::new(0.0, 0.0, NEVER_EXPLORED_Z)),
    ));
}

/// How far the viewer must move (world units) before the FOV is recomputed. Well under the
/// `simplify(1e-1)` tolerance the resulting geometry is reduced by anyway, and two orders of
/// magnitude under a single frame of player movement (~5 units at 320 u/s), so this only ever
/// suppresses recomputation for a viewer that is genuinely standing still. Compared against
/// the origin of the *last computed* FOV rather than the previous frame's, so sub-threshold
/// drift accumulates and eventually triggers instead of being lost.
const FOV_RECOMPUTE_EPSILON: f32 = 0.25;

/// The inputs the FOV is a pure function of, cached to skip recomputation when none changed.
/// Terrain and mobile-blocker movement aren't part of the key — they're detected via Bevy
/// change detection at the call site instead. Public only because it appears in
/// `update_fov`'s `Local<_>` parameter; it is an implementation detail of that system.
pub struct FovKey {
    origin: Vec2,
    radius: f32,
}

pub fn update_fov(
    viewer_query: Query<
        (Ref<Transform>, Option<&MovementModifiers>),
        (With<Viewer>, With<Transform>),
    >,
    fov_mesh_query: Query<&Mesh2d, With<FovMeshMarker>>,
    never_explored_query: Query<&Mesh2d, (With<NeverExploredMeshMarker>, Without<FovMeshMarker>)>,
    opaque_query: Query<(&GlobalTransform, &OpaqueVertices)>,
    moved_opaque_query: Query<(), (With<OpaqueVertices>, Changed<GlobalTransform>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    dungeon_state: Res<DungeonState>,
    bounds: Res<WorldBounds>,
    mut exploration_state: ResMut<ExplorationState>,
    mut current_fov: ResMut<CurrentFovState>,
    mut visible_area: ResMut<VisibleArea>,
    mut cached: Local<Option<FovKey>>,
) {
    let Ok((viewer_transform, viewer_modifiers)) = viewer_query.single() else { return };
    let fov_mesh_handle = fov_mesh_query.single().unwrap();
    let never_explored_mesh_handle = never_explored_query.single().unwrap();

    let origin = viewer_transform.translation.truncate();
    let vision_multiplier = viewer_modifiers.map(|m| m.vision_multiplier).unwrap_or(1.0);
    let radius = 50.0_f32 + (600.0_f32 - 50.0_f32) * vision_multiplier;

    // Recompute only when something the FOV actually depends on changed. This used to be
    // gated on `!viewer_transform.is_changed() && ... && fov_mesh_handle.0.path().is_some()`,
    // which never skipped anything at all: `path()` is `Some` only for handles loaded from an
    // asset path, and these meshes come from `meshes.add()`, so it is unconditionally `None`
    // and the whole guard was dead. The full ~2.6 ms/frame FOV rebuild therefore ran every
    // frame — including while the player stands still, which is most of the time given this
    // game's 0.0x idle time scale. Keying on the values themselves also fixes the "we should
    // recalculate this if terrain changes, too" TODO the old guard carried: excavating a wall
    // while stationary now updates the FOV, where a purely transform-based check would miss it.
    let terrain_changed = dungeon_state.is_changed();
    let blockers_moved = !moved_opaque_query.is_empty();
    let unchanged = cached.as_ref().is_some_and(|c| {
        c.radius == radius && c.origin.distance_squared(origin) <= FOV_RECOMPUTE_EPSILON.powi(2)
    });
    if unchanged && !terrain_changed && !blockers_moved {
        return;
    }
    *cached = Some(FovKey { origin, radius });

    // Append mobile obstacle polygons to solid_rock without unioning (fov_arc only needs segments).
    let mut obstacle_polys: Vec<SafePolygon> =
        dungeon_state.solid_rock.iter().cloned().map(SafePolygon).collect();
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
        obstacle_polys.push(SafePolygon(geo::Polygon::new(geo::LineString::from(pts), vec![])));
    }
    let obstacles = SafeMultiPolygon::from_polygons(obstacle_polys);

    let update = update_fov_from_pov(origin, radius, &obstacles, &bounds, &exploration_state.0);

    exploration_state.0 = update.exploration;
    current_fov.0 = update.fov;
    visible_area.0 = update.visible_area;
    *meshes.get_mut(&fov_mesh_handle.0).unwrap() = update.fov_mesh;
    *meshes.get_mut(&never_explored_mesh_handle.0).unwrap() = update.never_explored_mesh;
}

/// Everything one FOV recomputation produces. A struct rather than the tuple this used to
/// return, so adding `visible_area` (which callers now reuse instead of re-deriving) doesn't
/// leave call sites destructuring five anonymous positions.
pub struct FovUpdate {
    /// The still-unexplored region, i.e. the accumulated fog.
    pub exploration: SafeMultiPolygon,
    /// Overlay mesh for "explored but not currently visible".
    pub fov_mesh: Mesh,
    /// Overlay mesh for "never explored".
    pub never_explored_mesh: Mesh,
    /// The raw FOV polygon, as cast.
    pub fov: SafeMultiPolygon,
    /// `fov` grown to cover the wall faces bounding it — see [`VisibleArea`].
    pub visible_area: SafeMultiPolygon,
}

pub fn update_fov_from_pov(
    origin: Vec2,
    radius: f32,
    solid_rock: &SafeMultiPolygon,
    bounds: &WorldBounds,
    exploration: &SafeMultiPolygon,
) -> FovUpdate {
    let fov_poly = fov_arc(origin, radius, None, solid_rock);
    let fov_multi = SafeMultiPolygon::from(fov_poly);

    let w = bounds.width;
    let h = bounds.height;
    // To be safe, make it larger than bounds
    let bg_rect =
        geo::Rect::new((-w / 2.0 - 200.0, -h / 2.0 - 200.0), (w / 2.0 + 200.0, h / 2.0 + 200.0));
    let bg_poly = SafeMultiPolygon::from(MultiPolygon::new(vec![bg_rect.to_polygon()]));

    // The negative buffer is a bit of a hack to remove little erroneous rays that sometimes
    // sneak through the terrain. The positive buffer allows looking at the walls — hence
    // `WALL_FOV_DEPTH` rather than a bare literal, so game-side code drawing against this same
    // polygon can't drift out of sync with it.
    let visible_area = fov_multi.buffer(-1.0).buffer(1.0 + WALL_FOV_DEPTH);
    let dark_area = bg_poly.difference(&visible_area);

    FovUpdate {
        exploration: exploration.intersection(&dark_area).simplify(1e-1),
        fov_mesh: terrain::geometry_to_mesh(&dark_area),
        never_explored_mesh: terrain::geometry_to_mesh(exploration),
        fov: fov_multi,
        visible_area,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::safegeo::SafeMultiPolygon;

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> SafeMultiPolygon {
        SafeMultiPolygon::from_geo(MultiPolygon::new(vec![
            geo::Rect::new((x0, y0), (x1, y1)).to_polygon(),
        ]))
    }

    // --- Garbage geometry constructors ---

    /// Self-intersecting "bowtie": the edge (0,0)→(100,100) crosses (100,0)→(0,100).
    fn bowtie() -> SafeMultiPolygon {
        SafeMultiPolygon::from_geo(MultiPolygon::new(vec![Polygon::new(
            LineString::from(vec![
                (0.0f32, 0.0),
                (100.0, 100.0),
                (100.0, 0.0),
                (0.0, 100.0),
                (0.0, 0.0),
            ]),
            vec![],
        )]))
    }

    /// All vertices collinear — zero area.
    fn zero_area() -> SafeMultiPolygon {
        SafeMultiPolygon::from_geo(MultiPolygon::new(vec![Polygon::new(
            LineString::from(vec![(0.0f32, 0.0), (50.0, 0.0), (100.0, 0.0), (0.0, 0.0)]),
            vec![],
        )]))
    }

    /// Polygon whose exterior has only two distinct points (degenerate "edge").
    fn two_point_degenerate() -> SafeMultiPolygon {
        SafeMultiPolygon::from_geo(MultiPolygon::new(vec![Polygon::new(
            LineString::from(vec![(0.0f32, 0.0), (100.0, 50.0), (0.0, 0.0)]),
            vec![],
        )]))
    }

    /// Exterior ring is a single repeated point.
    fn point_polygon() -> SafeMultiPolygon {
        SafeMultiPolygon::from_geo(MultiPolygon::new(vec![Polygon::new(
            LineString::from(vec![(50.0f32, 50.0), (50.0, 50.0), (50.0, 50.0), (50.0, 50.0)]),
            vec![],
        )]))
    }

    /// Coordinates at NaN (sanitized to empty on construction).
    fn nan_coords() -> SafeMultiPolygon {
        SafeMultiPolygon::from_geo(MultiPolygon::new(vec![Polygon::new(
            LineString::from(vec![
                (f32::NAN, 0.0),
                (100.0, 0.0),
                (100.0, 100.0),
                (0.0, 100.0),
                (f32::NAN, 0.0),
            ]),
            vec![],
        )]))
    }

    /// Coordinates at ±Infinity (sanitized to empty on construction).
    fn inf_coords() -> SafeMultiPolygon {
        SafeMultiPolygon::from_geo(MultiPolygon::new(vec![Polygon::new(
            LineString::from(vec![
                (f32::NEG_INFINITY, f32::NEG_INFINITY),
                (f32::INFINITY, f32::NEG_INFINITY),
                (f32::INFINITY, f32::INFINITY),
                (f32::NEG_INFINITY, f32::INFINITY),
                (f32::NEG_INFINITY, f32::NEG_INFINITY),
            ]),
            vec![],
        )]))
    }

    /// Coordinates near f32::MAX.
    fn huge_coords() -> SafeMultiPolygon {
        let h = f32::MAX / 4.0;
        SafeMultiPolygon::from_geo(MultiPolygon::new(vec![Polygon::new(
            LineString::from(vec![(-h, -h), (h, -h), (h, h), (-h, h), (-h, -h)]),
            vec![],
        )]))
    }

    /// Exterior ring with many consecutive duplicate vertices.
    fn duplicate_vertices() -> SafeMultiPolygon {
        SafeMultiPolygon::from_geo(MultiPolygon::new(vec![Polygon::new(
            LineString::from(vec![
                (0.0f32, 0.0),
                (0.0, 0.0),
                (100.0, 0.0),
                (100.0, 0.0),
                (100.0, 100.0),
                (100.0, 100.0),
                (0.0, 100.0),
                (0.0, 0.0),
            ]),
            vec![],
        )]))
    }

    /// Tiny 5×5 square — near-degenerate for most buffer/triangulation operations.
    fn tiny_square() -> SafeMultiPolygon { rect(-2.5, -2.5, 2.5, 2.5) }

    /// Hole larger than its exterior — geometrically invalid.
    fn inverted_hole() -> SafeMultiPolygon {
        let exterior = LineString::from(vec![
            (0.0f32, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (0.0, 10.0),
            (0.0, 0.0),
        ]);
        let hole = LineString::from(vec![
            (-1000.0f32, -1000.0),
            (1000.0, -1000.0),
            (1000.0, 1000.0),
            (-1000.0, 1000.0),
            (-1000.0, -1000.0),
        ]);
        SafeMultiPolygon::from_geo(MultiPolygon::new(vec![Polygon::new(exterior, vec![hole])]))
    }

    /// An extremely thin sliver (1-unit wide, 200-unit long).
    fn sliver() -> SafeMultiPolygon { rect(0.0, 0.0, 200.0, 0.5) }

    // --- fov_arc stress tests ---

    #[test]
    fn test_fov_arc_empty_obstacles() {
        let result = fov_arc(Vec2::ZERO, 100.0, None, &SafeMultiPolygon::empty());
        assert!(result.exterior().coords().count() > 0);
    }

    #[test]
    fn test_fov_arc_zero_radius() {
        let _result = fov_arc(Vec2::ZERO, 0.0, None, &SafeMultiPolygon::empty());
    }

    #[test]
    fn test_fov_arc_negative_radius() {
        let _result = fov_arc(Vec2::ZERO, -10.0, None, &SafeMultiPolygon::empty());
    }

    #[test]
    fn test_fov_arc_nan_radius() {
        let _result = fov_arc(Vec2::ZERO, f32::NAN, None, &SafeMultiPolygon::empty());
    }

    #[test]
    fn test_fov_arc_nan_origin() {
        let _result = fov_arc(Vec2::new(f32::NAN, 0.0), 100.0, None, &SafeMultiPolygon::empty());
    }

    #[test]
    fn test_fov_arc_infinite_origin() {
        let _result =
            fov_arc(Vec2::new(f32::INFINITY, 0.0), 100.0, None, &SafeMultiPolygon::empty());
    }

    #[test]
    fn test_fov_arc_self_intersecting_obstacle() {
        let _result = fov_arc(Vec2::ZERO, 200.0, None, &bowtie());
    }

    #[test]
    fn test_fov_arc_zero_area_obstacle() {
        let _result = fov_arc(Vec2::ZERO, 200.0, None, &zero_area());
    }

    #[test]
    fn test_fov_arc_two_point_degenerate_obstacle() {
        let _result = fov_arc(Vec2::ZERO, 200.0, None, &two_point_degenerate());
    }

    #[test]
    fn test_fov_arc_point_polygon_obstacle() {
        let _result = fov_arc(Vec2::ZERO, 200.0, None, &point_polygon());
    }

    #[test]
    fn test_fov_arc_nan_obstacle_coords() {
        let _result = fov_arc(Vec2::ZERO, 200.0, None, &nan_coords());
    }

    #[test]
    fn test_fov_arc_inf_obstacle_coords() {
        let _result = fov_arc(Vec2::ZERO, 200.0, None, &inf_coords());
    }

    #[test]
    fn test_fov_arc_huge_obstacle_coords() {
        let _result = fov_arc(Vec2::ZERO, 200.0, None, &huge_coords());
    }

    #[test]
    fn test_fov_arc_duplicate_vertices_obstacle() {
        let _result = fov_arc(Vec2::ZERO, 200.0, None, &duplicate_vertices());
    }

    #[test]
    fn test_fov_arc_inverted_hole_obstacle() {
        let _result = fov_arc(Vec2::ZERO, 200.0, None, &inverted_hole());
    }

    #[test]
    fn test_fov_arc_sliver_obstacle() { let _result = fov_arc(Vec2::ZERO, 200.0, None, &sliver()); }

    #[test]
    fn test_fov_arc_origin_inside_obstacle() {
        // Origin is fully enclosed inside the obstacle polygon.
        let enclosing = SafeMultiPolygon::from_geo(MultiPolygon::new(vec![Polygon::new(
            LineString::from(vec![
                (-500.0f32, -500.0),
                (500.0, -500.0),
                (500.0, 500.0),
                (-500.0, 500.0),
                (-500.0, -500.0),
            ]),
            vec![],
        )]));
        let _result = fov_arc(Vec2::ZERO, 300.0, None, &enclosing);
    }

    // --- find_exploration_waypoint stress tests ---

    #[test]
    fn test_waypoint_empty_exploration() {
        let result = find_exploration_waypoint(
            Vec2::new(100.0, 0.0),
            &SafeMultiPolygon::empty(),
            &SafeMultiPolygon::empty(),
            &rect(-100.0, -50.0, 0.0, 50.0),
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_waypoint_nan_target() {
        let _result = find_exploration_waypoint(
            Vec2::new(f32::NAN, 0.0),
            &rect(0.0, -50.0, 200.0, 50.0),
            &SafeMultiPolygon::empty(),
            &rect(-200.0, -50.0, 0.0, 50.0),
        );
    }

    #[test]
    fn test_waypoint_self_intersecting_exploration() {
        let _result = find_exploration_waypoint(
            Vec2::new(100.0, 0.0),
            &bowtie(),
            &SafeMultiPolygon::empty(),
            &rect(-200.0, -200.0, 200.0, 200.0),
        );
    }

    #[test]
    fn test_waypoint_nan_in_exploration() {
        let _result = find_exploration_waypoint(
            Vec2::new(100.0, 0.0),
            &nan_coords(),
            &SafeMultiPolygon::empty(),
            &rect(-200.0, -200.0, 200.0, 200.0),
        );
    }

    #[test]
    fn test_waypoint_nan_in_blockers() {
        let _result = find_exploration_waypoint(
            Vec2::new(100.0, 0.0),
            &rect(0.0, -50.0, 200.0, 50.0),
            &nan_coords(),
            &rect(-200.0, -50.0, 0.0, 50.0),
        );
    }

    #[test]
    fn test_waypoint_inf_in_exploration() {
        let _result = find_exploration_waypoint(
            Vec2::new(100.0, 0.0),
            &inf_coords(),
            &SafeMultiPolygon::empty(),
            &rect(-200.0, -200.0, 200.0, 200.0),
        );
    }

    #[test]
    fn test_waypoint_inverted_hole_exploration() {
        let _result = find_exploration_waypoint(
            Vec2::new(100.0, 0.0),
            &inverted_hole(),
            &SafeMultiPolygon::empty(),
            &rect(-200.0, -200.0, 200.0, 200.0),
        );
    }

    #[test]
    fn test_waypoint_zero_area_exploration() {
        let _result = find_exploration_waypoint(
            Vec2::new(100.0, 0.0),
            &zero_area(),
            &SafeMultiPolygon::empty(),
            &rect(-200.0, -200.0, 200.0, 200.0),
        );
    }

    // --- update_fov_from_pov stress tests ---

    fn standard_bounds() -> crate::WorldBounds {
        crate::WorldBounds { width: 800.0, height: 600.0 }
    }

    #[test]
    fn test_update_fov_from_pov_empty_obstacles() {
        let exploration = rect(-400.0, -300.0, 400.0, 300.0);
        let _result = update_fov_from_pov(
            Vec2::ZERO,
            300.0,
            &SafeMultiPolygon::empty(),
            &standard_bounds(),
            &exploration,
        );
    }

    #[test]
    fn test_update_fov_from_pov_self_intersecting_obstacles() {
        let exploration = rect(-400.0, -300.0, 400.0, 300.0);
        let _result =
            update_fov_from_pov(Vec2::ZERO, 300.0, &bowtie(), &standard_bounds(), &exploration);
    }

    #[test]
    fn test_update_fov_from_pov_zero_area_obstacles() {
        let exploration = rect(-400.0, -300.0, 400.0, 300.0);
        let _result =
            update_fov_from_pov(Vec2::ZERO, 300.0, &zero_area(), &standard_bounds(), &exploration);
    }

    #[test]
    fn test_update_fov_from_pov_nan_obstacles() {
        let exploration = rect(-400.0, -300.0, 400.0, 300.0);
        let _result =
            update_fov_from_pov(Vec2::ZERO, 300.0, &nan_coords(), &standard_bounds(), &exploration);
    }

    #[test]
    fn test_update_fov_from_pov_inf_obstacles() {
        let exploration = rect(-400.0, -300.0, 400.0, 300.0);
        let _result =
            update_fov_from_pov(Vec2::ZERO, 300.0, &inf_coords(), &standard_bounds(), &exploration);
    }

    #[test]
    fn test_update_fov_from_pov_sliver_obstacles() {
        let exploration = rect(-400.0, -300.0, 400.0, 300.0);
        let _result =
            update_fov_from_pov(Vec2::ZERO, 300.0, &sliver(), &standard_bounds(), &exploration);
    }

    #[test]
    fn test_update_fov_from_pov_tiny_square_obstacles() {
        let exploration = rect(-400.0, -300.0, 400.0, 300.0);
        let _result = update_fov_from_pov(
            Vec2::ZERO,
            300.0,
            &tiny_square(),
            &standard_bounds(),
            &exploration,
        );
    }

    #[test]
    fn test_update_fov_from_pov_bowtie_exploration() {
        let _result = update_fov_from_pov(
            Vec2::ZERO,
            300.0,
            &SafeMultiPolygon::empty(),
            &standard_bounds(),
            &bowtie(),
        );
    }

    #[test]
    fn test_update_fov_from_pov_empty_exploration() {
        let _result = update_fov_from_pov(
            Vec2::ZERO,
            300.0,
            &SafeMultiPolygon::empty(),
            &standard_bounds(),
            &SafeMultiPolygon::empty(),
        );
    }

    #[test]
    fn test_update_fov_from_pov_zero_radius() {
        let exploration = rect(-400.0, -300.0, 400.0, 300.0);
        let _result = update_fov_from_pov(
            Vec2::ZERO,
            0.0,
            &SafeMultiPolygon::empty(),
            &standard_bounds(),
            &exploration,
        );
    }

    #[test]
    fn test_update_fov_from_pov_huge_coords_obstacles() {
        let exploration = rect(-400.0, -300.0, 400.0, 300.0);
        let _result = update_fov_from_pov(
            Vec2::ZERO,
            300.0,
            &huge_coords(),
            &standard_bounds(),
            &exploration,
        );
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
        let known_blockers = SafeMultiPolygon::empty();
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
        let known_blockers = SafeMultiPolygon::empty();
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
        let exploration = SafeMultiPolygon::from_geo(MultiPolygon::new(vec![
            geo::Rect::new((0.0_f32, -50.0), (200.0_f32, -20.0)).to_polygon(),
            geo::Rect::new((0.0_f32, 20.0), (200.0_f32, 50.0)).to_polygon(),
        ]));
        let playable_area = rect(-200.0, -50.0, 0.0, 50.0);
        let known_blockers = SafeMultiPolygon::empty();
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
