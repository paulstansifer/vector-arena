// Field of view calculation

use bevy::prelude::*;
use geo::{LineString, Polygon};
use std::ops::Range;

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
        (0..steps).map(|i| (i as f32 / steps as f32) * std::f32::consts::TAU - std::f32::consts::PI),
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
            .map(|a| (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI)
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
                if t > 1e-4 && (0.0..=1.0).contains(&u) {
                    Some(t)
                } else {
                    None
                }
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
