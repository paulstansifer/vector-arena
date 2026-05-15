use crate::terrain::PADDING;
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

    let axis = if width >= height {
        if can_split_vertical {
            SplitAxis::Vertical
        } else {
            SplitAxis::Horizontal
        }
    } else {
        if can_split_horizontal {
            SplitAxis::Horizontal
        } else {
            SplitAxis::Vertical
        }
    };

    match axis {
        SplitAxis::Vertical => split_vertical(&bounds, rng),
        SplitAxis::Horizontal => split_horizontal(&bounds, rng),
    }
}

#[derive(Copy, Clone)]
enum SplitAxis {
    Vertical,
    Horizontal,
}

// TODO: Maybe we should write `fn transpose` and use that to simplify this:

fn split_vertical(bounds: &Partition, rng: &mut ThreadRng) -> Option<(Partition, Partition)> {
    split_horizontal(&bounds.transpose(), rng).map(|(a, b)| (a.transpose(), b.transpose()))
}

fn split_horizontal(bounds: &Partition, rng: &mut ThreadRng) -> Option<(Partition, Partition)> {
    let SplitRange { start, end } = SplitRange::new(bounds.y, &bounds.horz_conn);
    let split_y =
        choose_split_coordinate(start, end, &bounds.horz_conn.0, &bounds.horz_conn.1, rng)?;
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
        let blocked_start = (conn - PADDING).max(start);
        let blocked_end = (conn + PADDING).min(end);
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
