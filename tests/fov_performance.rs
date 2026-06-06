mod test_lib;
use test_lib::preview;

use bevy::prelude::*;
use geo::{CoordsIter, MultiPolygon};
use rand::{SeedableRng, rngs::StdRng};
use vector_arena::{
    WorldBounds,
    dungeon::{
        bsp::Partition,
        level_generation::{PartitionRole, RoomVariant, TerrainGeometry},
    },
    fov::{ExplorationState, update_fov_from_pov},
};

#[test]
fn test_fov_point_explosion() {
    let mut rng = StdRng::seed_from_u64(1234);

    let terrain_geometry = TerrainGeometry::from_partitions_and_roles(
        800.0,
        310.0,
        vec![
            (
                Partition {
                    x: (10.0, 300.0),
                    y: (10.0, 300.0),
                    horz_conn: (vec![], vec![200.0]),
                    vert_conn: (vec![], vec![]),
                },
                PartitionRole::Room { variant: RoomVariant::Colonnade },
            ),
            (
                Partition {
                    x: (300.0, 500.0),
                    y: (10.0, 300.0),
                    horz_conn: (vec![200.0], vec![200.0]),
                    vert_conn: (vec![], vec![]),
                },
                PartitionRole::Corridor { double_width: false },
            ),
            (
                Partition {
                    x: (500.0, 790.0),
                    y: (10.0, 300.0),
                    horz_conn: (vec![200.0], vec![]),
                    vert_conn: (vec![], vec![]),
                },
                PartitionRole::Room { variant: RoomVariant::Oval },
            ),
        ],
        &mut rng,
    );

    // TODO: terrain coordinates are all positive, but everything else centers on (0,0). We should pick a system!

    let world_bounds = WorldBounds { width: 1200.0, height: 800.0 };
    let w = world_bounds.width;
    let h = world_bounds.height;
    let bg_rect =
        geo::Rect::new((-w / 2.0 - 200.0, -h / 2.0 - 200.0), (w / 2.0 + 200.0, h / 2.0 + 200.0));
    let mut exploration_state = ExplorationState(MultiPolygon::new(vec![bg_rect.to_polygon()]));

    // Terrain spans x: 10-790, y: 10-300; camera at (400, 155) frames it well.
    let pw = preview::PreviewWindow::from_env("FOV Preview", Vec2::new(0.0, 0.0));

    let mut points = vec![];
    for i in 0..=20 {
        let t = i as f32 / 20.0;
        points.push(Vec2::new(-150.0 * (1.0 - t) + 250.0 * t, 50.0));
    }

    for pt in &points {
        let (new_exp, _, _, _) = update_fov_from_pov(
            *pt,
            600.0,
            &terrain_geometry.solid_rock,
            &world_bounds,
            &exploration_state.0,
            &[],
        );
        exploration_state.0 = new_exp;
        if let Some(pw) = &pw {
            pw.show(make_frame(&terrain_geometry.solid_rock, &exploration_state.0, *pt));
        }
    }

    let pass_1_points: usize = exploration_state.0.coords_count();
    println!("First pass points: {pass_1_points}");

    for extra_pass in 0..10 {
        for pt in &points {
            let (new_exp, _, _, _) = update_fov_from_pov(
                Vec2::new(pt.x + 0.01 * (extra_pass as f32), pt.y + 0.01 * (extra_pass as f32)),
                600.0,
                &terrain_geometry.solid_rock,
                &world_bounds,
                &exploration_state.0,
                &[],
            );
            exploration_state.0 = new_exp;
            if let Some(pw) = &pw {
                pw.show(make_frame(&terrain_geometry.solid_rock, &exploration_state.0, *pt));
            }
        }
        if extra_pass % 2 == 0 {
            println!("Extra pass {extra_pass} points: {}", exploration_state.0.coords_count());
        }
    }

    let pass_2_points: usize = exploration_state.0.coords_count();
    assert!(pass_1_points < 500, "{}", pass_1_points);
    let extra_points = pass_2_points as i32 - pass_1_points as i32; // simplification might make it go down!
    assert!(extra_points < 30, "{}", extra_points);
}

fn make_frame(
    solid_rock: &MultiPolygon<f32>,
    exploration: &MultiPolygon<f32>,
    pt: Vec2,
) -> preview::Frame {
    preview::Frame {
        layers: vec![
            preview::poly_layer(solid_rock, Color::srgb(0.2, 0.5, 0.2), 0.0),
            preview::poly_layer(exploration, Color::srgba(0.5, 0.5, 0.7, 0.5), 1.0),
            preview::point_layer(pt, 8.0, Color::srgb(1.0, 0.8, 0.0), 2.0),
        ],
    }
}
