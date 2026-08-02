// Sanitizing wrappers around geo's core polygon types.
//
// Any geometry entering through `SafeLineString`, `SafePolygon`, or `SafeMultiPolygon`
// is guaranteed to contain only finite coordinates. Non-finite coords are dropped at
// the boundary (with an `eprintln!`), and rings that become degenerate after filtering
// (fewer than 4 coords — 3 unique + 1 closing) are discarded entirely.
//
// Every method that produces new geometry (`union`, `buffer`, `from_geo`, …) runs the
// same sanitization pass on its output before returning, so invariants are maintained
// across the full operation chain.
//
// All three types `Deref` to their underlying `geo` equivalents so that read-only geo
// traits (`Intersects`, `Contains`, `BoundingRect`, iteration, …) work without any
// changes at call sites.

use geo::{
    BooleanOps, Buffer, Coord, LineString, MultiPolygon, Polygon, Simplify, Translate,
    algorithm::buffer::BufferStyle,
};
use std::ops::Deref;

// ---------------------------------------------------------------------------
// Internal sanitization helpers
// ---------------------------------------------------------------------------

fn drop_non_finite(coords: Vec<Coord<f32>>) -> Vec<Coord<f32>> {
    coords
        .into_iter()
        .filter(|c| {
            if c.x.is_finite() && c.y.is_finite() {
                true
            } else {
                eprintln!("safegeo: dropped non-finite coord ({}, {})", c.x, c.y);
                false
            }
        })
        .collect()
}

// A valid closed ring needs at least 4 coords (3 distinct vertices + 1 closing repeat).
fn sanitize_ring(ls: LineString<f32>) -> Option<LineString<f32>> {
    let coords = drop_non_finite(ls.0);
    if coords.len() < 4 {
        if !coords.is_empty() {
            eprintln!(
                "safegeo: dropped degenerate ring ({} coords remain after filtering)",
                coords.len()
            );
        }
        None
    } else {
        Some(LineString(coords))
    }
}

fn sanitize_polygon(poly: Polygon<f32>) -> Option<Polygon<f32>> {
    let (exterior, interiors) = poly.into_inner();
    let exterior = sanitize_ring(exterior)?;
    let holes = interiors.into_iter().filter_map(sanitize_ring).collect();
    Some(Polygon::new(exterior, holes))
}

fn sanitize_mp(mp: MultiPolygon<f32>) -> MultiPolygon<f32> {
    MultiPolygon(mp.0.into_iter().filter_map(sanitize_polygon).collect())
}

// ---------------------------------------------------------------------------
// SafeLineString
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct SafeLineString(pub LineString<f32>);

impl Default for SafeLineString {
    fn default() -> Self { Self(LineString(vec![])) }
}

impl SafeLineString {
    /// Build from `(x, y)` tuples, dropping any with non-finite values.
    pub fn from_tuples(coords: impl IntoIterator<Item = (f32, f32)>) -> Self {
        let clean: Vec<Coord<f32>> = coords
            .into_iter()
            .filter_map(|(x, y)| {
                if x.is_finite() && y.is_finite() {
                    Some(Coord { x, y })
                } else {
                    eprintln!("safegeo: dropped non-finite coord ({x}, {y})");
                    None
                }
            })
            .collect();
        Self(LineString(clean))
    }

    pub fn into_inner(self) -> LineString<f32> { self.0 }
}

impl Deref for SafeLineString {
    type Target = LineString<f32>;
    fn deref(&self) -> &Self::Target { &self.0 }
}

/// Sanitizes on conversion: non-finite coords are dropped.
impl From<LineString<f32>> for SafeLineString {
    fn from(ls: LineString<f32>) -> Self { Self(LineString(drop_non_finite(ls.0))) }
}

// ---------------------------------------------------------------------------
// SafePolygon
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct SafePolygon(pub Polygon<f32>);

impl SafePolygon {
    /// Returns `None` if the exterior ring has fewer than 4 finite coords after sanitization.
    pub fn new(exterior: SafeLineString, holes: Vec<SafeLineString>) -> Option<Self> {
        let exterior = sanitize_ring(exterior.0)?;
        let holes = holes.into_iter().filter_map(|h| sanitize_ring(h.0)).collect();
        Some(Self(Polygon::new(exterior, holes)))
    }

    /// Buffer this polygon with a custom style; the output is sanitized.
    pub fn buffer_with_style(&self, style: BufferStyle<f32>) -> SafeMultiPolygon {
        SafeMultiPolygon::from_geo(self.0.buffer_with_style(style))
    }

    /// Intersection of two polygons; output is sanitized.
    pub fn intersection(&self, other: &SafePolygon) -> SafeMultiPolygon {
        SafeMultiPolygon::from_geo(self.0.intersection(&other.0))
    }

    /// Difference of two polygons; output is sanitized.
    pub fn difference(&self, other: &SafePolygon) -> SafeMultiPolygon {
        SafeMultiPolygon::from_geo(self.0.difference(&other.0))
    }

    /// Translate this polygon by (x, y); the result is always valid since translation preserves topology.
    pub fn translate(&self, x: f32, y: f32) -> Self { SafePolygon(self.0.translate(x, y)) }
}

impl Deref for SafePolygon {
    type Target = Polygon<f32>;
    fn deref(&self) -> &Self::Target { &self.0 }
}

impl TryFrom<Polygon<f32>> for SafePolygon {
    type Error = ();
    fn try_from(p: Polygon<f32>) -> Result<Self, ()> {
        sanitize_polygon(p).map(SafePolygon).ok_or(())
    }
}

impl From<SafePolygon> for SafeMultiPolygon {
    fn from(p: SafePolygon) -> Self { SafeMultiPolygon(MultiPolygon(vec![p.0])) }
}

/// Sanitizes and wraps a raw polygon as a single-polygon `SafeMultiPolygon`.
/// Returns `SafeMultiPolygon::empty()` if the polygon is degenerate after sanitization.
impl From<Polygon<f32>> for SafeMultiPolygon {
    fn from(p: Polygon<f32>) -> Self {
        match sanitize_polygon(p) {
            Some(clean) => SafeMultiPolygon(MultiPolygon(vec![clean])),
            None => SafeMultiPolygon::empty(),
        }
    }
}

// ---------------------------------------------------------------------------
// SafeMultiPolygon
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct SafeMultiPolygon(pub MultiPolygon<f32>);

impl Default for SafeMultiPolygon {
    fn default() -> Self { Self(MultiPolygon(vec![])) }
}

impl SafeMultiPolygon {
    pub fn empty() -> Self { Self::default() }

    pub fn from_polygons(polys: impl IntoIterator<Item = SafePolygon>) -> Self {
        Self(MultiPolygon(polys.into_iter().map(|p| p.0).collect()))
    }

    /// Import raw geo geometry, sanitizing it in the process.
    pub fn from_geo(mp: MultiPolygon<f32>) -> Self { Self(sanitize_mp(mp)) }

    pub fn union(&self, other: &SafeMultiPolygon) -> Self { Self::from_geo(self.0.union(&other.0)) }

    /// Union with a single polygon.
    pub fn union_poly(&self, poly: &SafePolygon) -> Self { Self::from_geo(self.0.union(&poly.0)) }

    pub fn intersection(&self, other: &SafeMultiPolygon) -> Self {
        Self::from_geo(self.0.intersection(&other.0))
    }

    pub fn difference(&self, other: &SafeMultiPolygon) -> Self {
        Self::from_geo(self.0.difference(&other.0))
    }

    pub fn buffer(&self, distance: f32) -> Self { Self::from_geo(self.0.buffer(distance)) }

    pub fn buffer_with_style(&self, style: BufferStyle<f32>) -> Self {
        Self::from_geo(self.0.buffer_with_style(style))
    }

    pub fn simplify(&self, epsilon: f32) -> Self { Self::from_geo(self.0.simplify(epsilon)) }

    pub fn translate(&self, x: f32, y: f32) -> Self { Self::from_geo(self.0.translate(x, y)) }
}

impl Deref for SafeMultiPolygon {
    type Target = MultiPolygon<f32>;
    fn deref(&self) -> &Self::Target { &self.0 }
}

/// Sanitizes on conversion.
impl From<MultiPolygon<f32>> for SafeMultiPolygon {
    fn from(mp: MultiPolygon<f32>) -> Self { Self::from_geo(mp) }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use geo::LineString;

    fn make_square(x0: f32, y0: f32, x1: f32, y1: f32) -> SafeMultiPolygon {
        SafeMultiPolygon::from_geo(MultiPolygon(vec![Polygon::new(
            LineString::from(vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]),
            vec![],
        )]))
    }

    #[test]
    fn test_nan_coord_dropped_from_line_string() {
        let ls = SafeLineString::from(LineString::from(vec![
            (0.0f32, 0.0),
            (f32::NAN, 5.0),
            (10.0, 0.0),
            (0.0, 0.0),
        ]));
        assert_eq!(ls.coords().count(), 3, "NaN coord should be dropped");
    }

    #[test]
    fn test_inf_coord_dropped_from_line_string() {
        let ls = SafeLineString::from(LineString::from(vec![
            (0.0f32, 0.0),
            (f32::INFINITY, 5.0),
            (10.0, 0.0),
            (0.0, 0.0),
        ]));
        assert_eq!(ls.coords().count(), 3, "Inf coord should be dropped");
    }

    #[test]
    fn test_degenerate_polygon_dropped_from_mp() {
        // Three-coord ring is valid for a LineString but not a closed polygon.
        let mp = MultiPolygon(vec![Polygon::new(
            LineString::from(vec![(0.0f32, 0.0), (5.0, 0.0), (0.0, 0.0)]),
            vec![],
        )]);
        let safe = SafeMultiPolygon::from_geo(mp);
        assert!(safe.0.0.is_empty(), "degenerate polygon should be dropped");
    }

    #[test]
    fn test_nan_in_boolean_op_does_not_panic() {
        let a = make_square(0.0, 0.0, 10.0, 10.0);
        let b_raw = MultiPolygon(vec![Polygon::new(
            LineString::from(vec![
                (f32::NAN, 0.0),
                (20.0, 0.0),
                (20.0, 20.0),
                (0.0, 20.0),
                (f32::NAN, 0.0),
            ]),
            vec![],
        )]);
        let b = SafeMultiPolygon::from_geo(b_raw);
        // b's polygon is dropped (NaN ring < 4 finite coords), so union == a.
        let result = a.union(&b);
        assert!(!result.0.0.is_empty());
    }

    #[test]
    fn test_union_two_squares() {
        let a = make_square(0.0, 0.0, 10.0, 10.0);
        let b = make_square(5.0, 0.0, 15.0, 10.0);
        let result = a.union(&b);
        assert!(!result.0.0.is_empty());
    }

    #[test]
    fn test_buffer_output_is_sanitized() {
        let a = make_square(0.0, 0.0, 10.0, 10.0);
        let result = a.buffer(2.0);
        for poly in result.0.iter() {
            for coord in poly.exterior().coords() {
                assert!(coord.x.is_finite() && coord.y.is_finite());
            }
        }
    }

    // ---------------------------------------------------------------------------
    // Garbage-geometry stress tests (ported from crumble_terrain)
    // ---------------------------------------------------------------------------

    fn bowtie() -> SafeMultiPolygon {
        SafeMultiPolygon::from_geo(MultiPolygon(vec![Polygon::new(
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

    fn zero_area() -> SafeMultiPolygon {
        SafeMultiPolygon::from_geo(MultiPolygon(vec![Polygon::new(
            LineString::from(vec![(0.0f32, 0.0), (50.0, 0.0), (100.0, 0.0), (0.0, 0.0)]),
            vec![],
        )]))
    }

    fn nan_coords() -> SafeMultiPolygon {
        SafeMultiPolygon::from_geo(MultiPolygon(vec![Polygon::new(
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

    fn sliver() -> SafeMultiPolygon {
        SafeMultiPolygon::from_geo(MultiPolygon(vec![Polygon::new(
            LineString::from(vec![
                (0.0f32, 0.0),
                (200.0, 0.0),
                (200.0, 0.3),
                (0.0, 0.3),
                (0.0, 0.0),
            ]),
            vec![],
        )]))
    }

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
        SafeMultiPolygon::from_geo(MultiPolygon(vec![Polygon::new(exterior, vec![hole])]))
    }

    fn terrain_with_hole() -> SafeMultiPolygon {
        SafeMultiPolygon::from_geo(MultiPolygon(vec![Polygon::new(
            LineString::from(vec![
                (-200.0f32, -200.0),
                (200.0, -200.0),
                (200.0, 200.0),
                (-200.0, 200.0),
                (-200.0, -200.0),
            ]),
            vec![LineString::from(vec![
                (-100.0f32, -100.0),
                (-100.0, 100.0),
                (100.0, 100.0),
                (100.0, -100.0),
                (-100.0, -100.0),
            ])],
        )]))
    }

    #[test]
    fn test_intersection_bowtie_with_terrain() {
        let _r = terrain_with_hole().intersection(&bowtie());
    }

    #[test]
    fn test_difference_bowtie_from_terrain() { let _r = terrain_with_hole().difference(&bowtie()); }

    #[test]
    fn test_union_bowtie_with_terrain() { let _r = terrain_with_hole().union(&bowtie()); }

    #[test]
    fn test_intersection_zero_area_with_terrain() {
        let _r = terrain_with_hole().intersection(&zero_area());
    }

    #[test]
    fn test_difference_zero_area_from_terrain() {
        let _r = terrain_with_hole().difference(&zero_area());
    }

    #[test]
    fn test_intersection_sliver_with_terrain() {
        let _r = terrain_with_hole().intersection(&sliver());
    }

    #[test]
    fn test_difference_sliver_from_terrain() { let _r = terrain_with_hole().difference(&sliver()); }

    #[test]
    fn test_intersection_nan_with_terrain() {
        let _r = terrain_with_hole().intersection(&nan_coords());
    }

    #[test]
    fn test_difference_nan_from_terrain() {
        let _r = terrain_with_hole().difference(&nan_coords());
    }

    #[test]
    fn test_union_nan_with_terrain() { let _r = terrain_with_hole().union(&nan_coords()); }

    #[test]
    fn test_intersection_inverted_hole_with_terrain() {
        let _r = terrain_with_hole().intersection(&inverted_hole());
    }

    #[test]
    fn test_difference_inverted_hole_from_terrain() {
        let _r = terrain_with_hole().difference(&inverted_hole());
    }

    #[test]
    fn test_buffer_bowtie_negative() { let _r = bowtie().buffer(-4.0); }

    #[test]
    fn test_buffer_zero_area_negative() { let _r = zero_area().buffer(-4.0); }

    #[test]
    fn test_buffer_sliver_negative() { let _r = sliver().buffer(-4.0); }

    #[test]
    fn test_buffer_nan_coords() { let _r = nan_coords().buffer(-4.0); }

    #[test]
    fn test_buffer_inverted_hole() { let _r = inverted_hole().buffer(-4.0); }
}
