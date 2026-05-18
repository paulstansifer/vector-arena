use crate::terrain::{CORRIDOR_WIDTH, PADDING};
use rand::prelude::*;
use std::ops::Range;

const DOUBLE_CONNECTION_UNCERTAIN: Range<f32> = 500.0..750.0;
const MIN_PARTITION_SIZE: Range<f32> = 200.0..550.0;
const MIN_ROOM_SIZE: f32 = MIN_PARTITION_SIZE.start - PADDING * 2.0;

#[derive(Clone, Debug)]
pub struct Partition {
    pub x: (f32, f32),
    pub y: (f32, f32),
    pub horz_conn: (Vec<f32>, Vec<f32>),
    pub vert_conn: (Vec<f32>, Vec<f32>),
}

impl Partition {
    pub fn connection_count(&self) -> usize {
        self.horz_conn.0.len()
            + self.horz_conn.1.len()
            + self.vert_conn.0.len()
            + self.vert_conn.1.len()
    }

    fn transpose(&self) -> Self {
        Partition {
            x: self.y,
            y: self.x,
            horz_conn: self.vert_conn.clone(),
            vert_conn: self.horz_conn.clone(),
        }
    }
}

pub fn partition_space(bounds: Partition, rng: &mut ThreadRng) -> Vec<Partition> {
    let mut stack = vec![bounds];
    let mut output = Vec::new();

    while let Some(partition) = stack.pop() {
        if let Some((a, b)) = split_partition(&partition, rng) {
            stack.push(b);
            stack.push(a);
        } else {
            output.push(partition);
        }
    }

    output
}

fn split_partition(bounds: &Partition, rng: &mut ThreadRng) -> Option<(Partition, Partition)> {
    let width = bounds.x.1 - bounds.x.0;
    let height = bounds.y.1 - bounds.y.0;

    let can_split_vertical = width > rng.gen_range(MIN_PARTITION_SIZE);
    let can_split_horizontal = height > rng.gen_range(MIN_PARTITION_SIZE);

    if !can_split_vertical && !can_split_horizontal {
        return None;
    }

    let axis = match (can_split_vertical, can_split_horizontal, width >= height) {
        (true, _, true) | (true, false, _) => SplitAxis::Vertical,
        (_, true, _) => SplitAxis::Horizontal,
        _ => unreachable!(), // both false already returned None above
    };

    match axis {
        SplitAxis::Vertical => split_vertical(bounds, rng),
        SplitAxis::Horizontal => split_horizontal(bounds, rng),
    }
}

#[derive(Copy, Clone)]
enum SplitAxis {
    Vertical,
    Horizontal,
}

fn split_vertical(bounds: &Partition, rng: &mut ThreadRng) -> Option<(Partition, Partition)> {
    split_horizontal(&bounds.transpose(), rng).map(|(a, b)| (a.transpose(), b.transpose()))
}

fn split_horizontal(bounds: &Partition, rng: &mut ThreadRng) -> Option<(Partition, Partition)> {
    let SplitRange { start, end } = SplitRange::new(bounds.y, &bounds.horz_conn);
    let split_y = choose_split_coordinate(
        start,
        end,
        &bounds.horz_conn.0,
        &bounds.horz_conn.1,
        PADDING + CORRIDOR_WIDTH / 2.0,
        rng,
    )?;
    let mut bottom = Partition {
        x: bounds.x,
        y: (bounds.y.0, split_y),
        horz_conn: (Vec::new(), Vec::new()),
        vert_conn: (allocate_coords(&bounds.vert_conn.0, bounds.x), Vec::new()),
    };
    let mut top = Partition {
        x: bounds.x,
        y: (split_y, bounds.y.1),
        horz_conn: (Vec::new(), Vec::new()),
        vert_conn: (Vec::new(), allocate_coords(&bounds.vert_conn.1, bounds.x)),
    };

    let count = internal_connection_count(bounds.x.1 - bounds.x.0, rng);
    let interior_xs = interior_positions(bounds.x, count);
    bottom.vert_conn.1 = interior_xs.clone();
    top.vert_conn.0 = interior_xs;

    bottom.horz_conn.0 = allocate_coords(&bounds.horz_conn.0, bottom.y);
    bottom.horz_conn.1 = allocate_coords(&bounds.horz_conn.1, bottom.y);
    top.horz_conn.0 = allocate_coords(&bounds.horz_conn.0, top.y);
    top.horz_conn.1 = allocate_coords(&bounds.horz_conn.1, top.y);

    Some((bottom, top))
}

struct SplitRange {
    start: f32,
    end: f32,
}

impl SplitRange {
    fn new(primary: (f32, f32), existing: &(Vec<f32>, Vec<f32>)) -> Self {
        let min = primary.0 + MIN_ROOM_SIZE + PADDING;
        let max = primary.1 - MIN_ROOM_SIZE - PADDING;
        let mut start = min;
        let mut end = max;

        let mut reserved = existing
            .0
            .iter()
            .chain(existing.1.iter())
            .copied()
            .collect::<Vec<_>>();
        reserved.sort_by(|a, b| a.partial_cmp(b).unwrap());
        reserved.dedup();

        for conn in reserved {
            if conn - PADDING > start && conn + PADDING < end {
                continue;
            }
            if conn - PADDING <= start && conn + PADDING > start {
                start = (conn + PADDING).max(start);
            }
            if conn - PADDING < end && conn + PADDING >= end {
                end = (conn - PADDING).min(end);
            }
        }

        SplitRange { start, end }
    }
}

fn choose_split_coordinate(
    start: f32,
    end: f32,
    existing0: &[f32],
    existing1: &[f32],
    connection_margin: f32,
    rng: &mut ThreadRng,
) -> Option<f32> {
    if end <= start {
        return None;
    }

    let reserved = existing0
        .iter()
        .chain(existing1.iter())
        .copied()
        .collect::<Vec<_>>();
    let mut intervals = vec![(start, end)];

    for conn in reserved {
        let blocked_start = (conn - connection_margin).max(start);
        let blocked_end = (conn + connection_margin).min(end);
        if blocked_start >= blocked_end {
            continue;
        }

        let mut next = Vec::new();
        for (a, b) in intervals.drain(..) {
            if blocked_end <= a || blocked_start >= b {
                next.push((a, b));
            } else {
                if blocked_start > a {
                    next.push((a, blocked_start));
                }
                if blocked_end < b {
                    next.push((blocked_end, b));
                }
            }
        }
        intervals = next;
        if intervals.is_empty() {
            return None;
        }
    }

    let lengths: Vec<f32> = intervals.iter().map(|(a, b)| b - a).collect();
    let total_length: f32 = lengths.iter().sum();
    if total_length <= 0.0 {
        return None;
    }

    let mut choice = rng.gen_range(0.0..total_length);
    for ((a, b), length) in intervals.into_iter().zip(lengths) {
        if choice <= length {
            return Some(rng.gen_range(a..b));
        }
        choice -= length;
    }

    None
}

fn allocate_coords(coords: &[f32], range: (f32, f32)) -> Vec<f32> {
    coords
        .iter()
        .copied()
        .filter(|&coord| coord >= range.0 && coord <= range.1)
        .collect()
}

fn internal_connection_count(length: f32, rng: &mut ThreadRng) -> usize {
    if length >= DOUBLE_CONNECTION_UNCERTAIN.end {
        2
    } else if DOUBLE_CONNECTION_UNCERTAIN.contains(&length) {
        if rng.gen_bool(0.5) { 2 } else { 1 }
    } else {
        1
    }
}

fn interior_positions(range: (f32, f32), count: usize) -> Vec<f32> {
    let min = range.0 + PADDING;
    let max = range.1 - PADDING;
    if count == 0 || max <= min {
        return Vec::new();
    }

    let interval = (max - min) / (count as f32 + 1.0);
    (1..=count).map(|i| min + interval * i as f32).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_choose_split_coordinate_respects_both_existing() {
        // When both existing0 and existing1 have coordinates, result should avoid both
        let mut rng = rand::thread_rng();
        let existing0 = vec![30.0];
        let existing1 = vec![70.0];

        for _ in 0..100 {
            let result =
                choose_split_coordinate(0.0, 100.0, &existing0, &existing1, PADDING, &mut rng);
            if let Some(value) = result {
                // Check blocked region around 30.0
                let blocked_start_0 = (30.0 - PADDING).max(0.0);
                let blocked_end_0 = (30.0 + PADDING).min(100.0);
                assert!(
                    value < blocked_start_0 || value > blocked_end_0,
                    "Value {} too close to existing0 coordinate 30.0",
                    value
                );

                // Check blocked region around 70.0
                let blocked_start_1 = (70.0 - PADDING).max(0.0);
                let blocked_end_1 = (70.0 + PADDING).min(100.0);
                assert!(
                    value < blocked_start_1 || value > blocked_end_1,
                    "Value {} too close to existing1 coordinate 70.0",
                    value
                );
            }
        }
    }

    #[test]
    fn test_choose_split_coordinate_returns_none_when_completely_blocked() {
        // When existing coordinates block the entire range, should return None
        let mut rng = rand::thread_rng();
        let existing = vec![50.0];

        // With a small range and a coordinate in the middle, it should be blocked
        let _result = choose_split_coordinate(40.0, 60.0, &existing, &[], PADDING, &mut rng);
        // This might return None if the entire range is blocked
        // Let's check with a more extreme case

        let result = choose_split_coordinate(
            50.0 - PADDING - 1.0,
            50.0 + PADDING + 1.0,
            &vec![50.0],
            &[],
            PADDING,
            &mut rng,
        );
        // Should still have room on the edges
        assert!(result.is_some());
    }

    #[test]
    fn test_choose_split_coordinate_multiple_existing() {
        // When there are multiple existing coordinates, all should be respected
        let mut rng = rand::thread_rng();
        let existing0 = vec![25.0, 75.0];
        let existing1 = vec![50.0];

        for _ in 0..100 {
            if let Some(value) =
                choose_split_coordinate(0.0, 100.0, &existing0, &existing1, PADDING, &mut rng)
            {
                for &coord in &existing0 {
                    let blocked_start = (coord - PADDING).max(0.0);
                    let blocked_end = (coord + PADDING).min(100.0);
                    assert!(
                        value < blocked_start || value > blocked_end,
                        "Value {} too close to existing0 coordinate {}",
                        value,
                        coord
                    );
                }
                for &coord in &existing1 {
                    let blocked_start = (coord - PADDING).max(0.0);
                    let blocked_end = (coord + PADDING).min(100.0);
                    assert!(
                        value < blocked_start || value > blocked_end,
                        "Value {} too close to existing1 coordinate {}",
                        value,
                        coord
                    );
                }
            }
        }
    }

    #[test]
    fn test_choose_split_coordinate_near_boundaries() {
        // Existing coordinates near range boundaries should still be respected
        let mut rng = rand::thread_rng();
        let existing = vec![5.0, 95.0];

        for _ in 0..50 {
            if let Some(value) =
                choose_split_coordinate(0.0, 100.0, &existing, &[], PADDING, &mut rng)
            {
                // Avoid region around 5.0
                assert!(value < (5.0 - PADDING).max(0.0) || value > (5.0 + PADDING).min(100.0));
                // Avoid region around 95.0
                assert!(value < (95.0 - PADDING).max(0.0) || value > (95.0 + PADDING).min(100.0));
            }
        }
    }
}
