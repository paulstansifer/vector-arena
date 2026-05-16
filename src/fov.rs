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
    let mut segments = Vec::new();

    // Extract line segments from the multipolygon
    for poly in obstacles {
        for line_string in std::iter::once(poly.exterior()).chain(poly.interiors()) {
            let coords: Vec<_> = line_string.coords().collect();
            for i in 0..coords.len() {
                let p1 = coords[i];
                let p2 = coords[(i + 1) % coords.len()];
                segments.push((Vec2::new(p1.x, p1.y), Vec2::new(p2.x, p2.y)));
            }
        }
    }

    let mut angles_to_cast = Vec::new();
    let deg_0_01 = 0.01_f32.to_radians();

    // Collect angles to all points from segments
    for &(a, b) in &segments {
        for pt in [a, b] {
            let diff = pt - origin;
            let dist = diff.length();
            if dist <= radius + 0.1 {
                let angle = diff.y.atan2(diff.x);
                angles_to_cast.push(angle);
                angles_to_cast.push(angle - deg_0_01);
                angles_to_cast.push(angle + deg_0_01);
            }
        }
    }

    // Add fixed angles to make the circle smooth where it hits the radius
    let steps = 64;
    for i in 0..steps {
        let angle = (i as f32 / steps as f32) * std::f32::consts::TAU - std::f32::consts::PI;
        angles_to_cast.push(angle);
    }

    // If angle_range is Some, also push the boundary angles
    if let Some(range) = &angle_range {
        angles_to_cast.push(range.start);
        angles_to_cast.push(range.end);
    }

    // Filter angles based on angle_range
    let angles: Vec<f32> = angles_to_cast
        .into_iter()
        .filter(|&a| {
            if let Some(range) = &angle_range {
                let mut norm_a = a;
                while norm_a < range.start {
                    norm_a += std::f32::consts::TAU;
                }
                while norm_a >= range.start + std::f32::consts::TAU {
                    norm_a -= std::f32::consts::TAU;
                }

                norm_a <= range.end
            } else {
                true
            }
        })
        .collect();

    // Sort angles and deduplicate to avoid casting duplicate rays
    let mut sorted_angles = angles;
    if angle_range.is_none() {
        for a in &mut sorted_angles {
            while *a <= -std::f32::consts::PI {
                *a += std::f32::consts::TAU;
            }
            while *a > std::f32::consts::PI {
                *a -= std::f32::consts::TAU;
            }
        }
    } else {
        let start = angle_range.as_ref().unwrap().start;
        for a in &mut sorted_angles {
            while *a < start {
                *a += std::f32::consts::TAU;
            }
            while *a >= start + std::f32::consts::TAU {
                *a -= std::f32::consts::TAU;
            }
        }
    }

    sorted_angles.sort_by(|a, b| a.partial_cmp(b).unwrap());
    sorted_angles.dedup_by(|a, b| (*a - *b).abs() < 1e-5);

    let mut polygon_points = Vec::new();

    // If it's a wedge, start at the origin
    if angle_range.is_some() {
        polygon_points.push((origin.x, origin.y));
    }

    for angle in sorted_angles {
        let dir = Vec2::new(angle.cos(), angle.sin());
        let mut min_t = radius;

        for &(p1, p2) in &segments {
            let p = origin;
            let r = dir;
            let q = p1;
            let s = p2 - p1;

            let r_cross_s = r.x * s.y - r.y * s.x;
            let q_minus_p = q - p;

            if r_cross_s.abs() > 1e-6 {
                let t = (q_minus_p.x * s.y - q_minus_p.y * s.x) / r_cross_s;
                let u = (q_minus_p.x * r.y - q_minus_p.y * r.x) / r_cross_s;

                // Use a small epsilon to avoid self-intersection on edges
                if t > 1e-4 && u >= 0.0 && u <= 1.0 {
                    if t < min_t {
                        min_t = t;
                    }
                }
            }
        }

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
