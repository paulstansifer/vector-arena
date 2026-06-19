// Right-click excavation and rubble spawning.
// Subtracts a 40-unit circle from solid_rock, unions it into playable_area, then
// shatters the removed rock: iteratively slices pieces along random-angle planes
// until all pieces are < 18 units, filters out tiny fragments, erodes by 4 units,
// and spawns Dynamic Rubble entities with convex hull colliders and random outward
// velocities.  Fragile entities (doors) inside the circle are despawned.
use crate::{
    GameLayer, GameState,
    dungeon::terrain::{
        DungeonCollider, DungeonState, DungeonVisuals, geometry_to_collider, geometry_to_mesh,
    },
    item::WandCrumblingEvent,
    nav::{DungeonNavMesh, playable_area_to_nav_mesh},
};
use avian2d::{math::PI, prelude::*};
use bevy::prelude::*;
use bevy_landmass::prelude::*;
use geo::{
    BooleanOps, BoundingRect, Buffer, Centroid, Coord, Intersects, LineString, MultiPolygon,
    Polygon, Rect, Translate,
};
use rand::Rng;

/// Radius (world units) of the circle subtracted from solid rock per right-click.
const EXCAVATION_RADIUS: f32 = 40.0;
/// Polygon points used to approximate the excavation circle.
const EXCAVATION_CIRCLE_POINTS: usize = 16;
/// Rubble pieces larger than this (half of the bounding-box's longer side) keep getting sliced.
const RUBBLE_MAX_RADIUS: f32 = 18.0;
/// Rubble pieces narrower than this on their shorter axis are discarded as too-tiny shards.
const RUBBLE_MIN_DIMENSION: f32 = 7.0;
/// Inward buffer (world units) applied to each rubble piece — gives a gap and rounds corners.
const RUBBLE_SHRINK: f32 = 4.0;

#[derive(Component)]
pub struct Rubble;

/// Marker for entities (e.g. doors) that should be destroyed if caught in an excavation.
#[derive(Component)]
pub struct Fragile;

#[derive(Resource)]
pub struct RubbleMaterial(pub Handle<ColorMaterial>);

/// Helper to create a circular polygon approximating an excavation area
pub fn create_circle_polygon(center: Vec2, radius: f32, points: usize) -> Polygon<f32> {
    use std::f32::consts::TAU;
    let mut coords = Vec::new();
    for i in 0..points {
        let angle = (i as f32) * TAU / (points as f32);
        let x = center.x + radius * angle.cos();
        let y = center.y + radius * angle.sin();
        coords.push((x, y));
    }
    if let Some(&first) = coords.first() {
        coords.push(first);
    }
    Polygon::new(LineString::from(coords), vec![])
}

/// Subtracts an input polygon from the terrain geometry, updates the playable area,
/// regenerates the visuals, collider, and navmesh, and spawns dynamic Rubble objects
/// from the intersection.
/// `outward_impulse`: if `Some((origin, speed))`, each rubble piece receives an
/// additional velocity directed away from `origin` at the given speed, on top of
/// the normal small random tumble.
pub fn subtract_polygon_from_terrain(
    commands: &mut Commands,
    input_polygon: &MultiPolygon<f32>,
    mut dungeon_state: ResMut<DungeonState>,
    mut dungeon_visuals: ResMut<DungeonVisuals>,
    mut dungeon_collider: ResMut<DungeonCollider>,
    mut dungeon_nav_mesh: ResMut<DungeonNavMesh>,
    meshes: &mut Assets<Mesh>,
    nav_meshes: &mut Assets<NavMesh2d>,
    rubble_material: &Handle<ColorMaterial>,
    outward_impulse: Option<(Vec2, f32)>,
) {
    // Calculate the intersection of the input polygon and the current terrain walls
    let intersection = dungeon_state.solid_rock.intersection(input_polygon);

    // Subtract the input polygon from the terrain walls
    let new_terrain = dungeon_state.solid_rock.difference(input_polygon);
    dungeon_state.solid_rock = new_terrain.clone();

    // Expand the playable area by the intersection
    let new_playable_area = dungeon_state.playable_area.union(&intersection);
    dungeon_state.playable_area = new_playable_area.clone();

    // Update the terrain visual mesh Resource
    let new_mesh = geometry_to_mesh(&new_terrain);
    dungeon_visuals.0 = meshes.add(new_mesh);

    // Update the terrain collider Resource
    dungeon_collider.0 = geometry_to_collider(&new_terrain);

    // Update the navmesh Resource
    let valid_nav_mesh = playable_area_to_nav_mesh(&new_playable_area, &dungeon_state.torpor_zones);
    dungeon_nav_mesh.0 = nav_meshes.add(NavMesh2d { nav_mesh: valid_nav_mesh });

    // Break up the rubble before shrinking
    let mut rubble_polygons: Vec<Polygon<f32>> = intersection.iter().cloned().collect();

    loop {
        // Find the index and radius of the largest piece
        let mut largest_idx = None;
        let mut largest_radius = 0.0;

        for (i, poly) in rubble_polygons.iter().enumerate() {
            if let Some(rect) = poly.bounding_rect() {
                let radius = rect.width().max(rect.height()) / 2.0;
                if radius > largest_radius {
                    largest_radius = radius;
                    largest_idx = Some(i);
                }
            }
        }

        if largest_radius < RUBBLE_MAX_RADIUS {
            break;
        }
        let idx = match largest_idx {
            Some(i) => i,
            None => break,
        };

        // Remove the largest polygon to slice it
        let poly_to_slice = rubble_polygons.remove(idx);

        // Generate a random angle and slice near the center
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let angle: f32 = rng.gen_range(0.0..std::f32::consts::TAU);

        let center = if let Some(centroid) = poly_to_slice.centroid() {
            centroid
        } else if let Some(rect) = poly_to_slice.bounding_rect() {
            geo::Point::new(rect.center().x, rect.center().y)
        } else {
            geo::Point::new(0.0, 0.0)
        };

        // Construct the cutting half-plane
        let half_width = 4000.0;
        let half_height = 2000.0;
        let local_coords = vec![
            (-half_width, 0.0),
            (half_width, 0.0),
            (half_width, half_height),
            (-half_width, half_height),
        ];
        let cos = angle.cos();
        let sin = angle.sin();
        let mut coords: Vec<(f32, f32)> = local_coords
            .into_iter()
            .map(|(lx, ly)| {
                let wx = lx * cos - ly * sin + center.x();
                let wy = lx * sin + ly * cos + center.y();
                (wx, wy)
            })
            .collect();
        if let Some(&first) = coords.first() {
            coords.push(first);
        }
        let cutting_half_plane = Polygon::new(LineString::from(coords), vec![]);

        // Slice
        let part_1 = poly_to_slice.intersection(&cutting_half_plane);
        let part_2 = poly_to_slice.difference(&cutting_half_plane);

        // Add back the sliced pieces
        for p in part_1.iter().chain(part_2.iter()) {
            if p.exterior().coords().count() >= 4 {
                rubble_polygons.push(p.clone());
            }
        }
    }

    // Discard slivers whose shorter dimension is too small to be visible rubble.
    rubble_polygons.retain(|poly| {
        if let Some(rect) = poly.bounding_rect() {
            let smaller_dim = rect.width().min(rect.height());
            smaller_dim >= RUBBLE_MIN_DIMENSION
        } else {
            false
        }
    });

    // Shrink the rubble pieces and round off sharp corners.
    use geo::{
        algorithm::buffer::BufferStyle,
        buffer::{LineCap, LineJoin},
    };

    let style = BufferStyle::new(-RUBBLE_SHRINK)
        .line_cap(LineCap::Round(PI / 4.0))
        .line_join(LineJoin::Round(PI / 4.0));

    for poly in rubble_polygons {
        // Buffer each individual polygon with the round style to shrink and round off corners
        let shrunk_poly_multi = poly.buffer_with_style(style.clone());
        for shrunk_poly in shrunk_poly_multi.iter() {
            if shrunk_poly.exterior().coords().count() < 4 {
                continue;
            }
            if let Some(centroid) = shrunk_poly.centroid() {
                let center = Vec2::new(centroid.x(), centroid.y());
                let local_poly = shrunk_poly.translate(-center.x, -center.y);
                let vertices: Vec<Vec2> =
                    local_poly.exterior().coords().map(|c| Vec2::new(c.x, c.y)).collect();
                if let Some(rubble_collider) = Collider::convex_hull(vertices) {
                    let local_multipoly = MultiPolygon::new(vec![local_poly]);
                    let rubble_mesh = geometry_to_mesh(&local_multipoly);

                    let mut rng = rand::thread_rng();
                    let angle: f32 = rng.gen_range(0.0..std::f32::consts::TAU);
                    let random_speed: f32 = rng.gen_range(10.0..30.0);
                    let random_vel = Vec2::new(angle.cos(), angle.sin()) * random_speed;
                    let directed_vel = outward_impulse.map_or(Vec2::ZERO, |(origin, speed)| {
                        (center - origin).normalize_or_zero() * speed
                    });
                    let velocity = LinearVelocity(random_vel + directed_vel);

                    commands.spawn((
                        DespawnOnExit(GameState::InLevel),
                        Rubble,
                        Mesh2d(meshes.add(rubble_mesh)),
                        MeshMaterial2d(rubble_material.clone()),
                        Transform::from_translation(center.extend(10.0)), // Set Z to 10.0 to render on top
                        RigidBody::Dynamic,
                        rubble_collider,
                        CollisionLayers::new(
                            GameLayer::Dynamic,
                            [GameLayer::Wall, GameLayer::Dynamic],
                        ),
                        velocity,
                        LinearDamping(1.5),
                        AngularDamping(1.5),
                        Friction::new(3.0),
                        Restitution::new(0.4),
                    ));
                }
            }
        }
    }
}

/// Like `create_circle_polygon` but with per-vertex radius and angle jitter for an organic shape.
fn create_irregular_circle_polygon(
    center: Vec2,
    radius: f32,
    points: usize,
    rng: &mut impl rand::Rng,
) -> Polygon<f32> {
    use std::f32::consts::TAU;
    let angle_step = TAU / (points as f32);
    let mut coords: Vec<(f32, f32)> = (0..points)
        .map(|i| {
            let base = (i as f32) * angle_step;
            let jitter = rng.gen_range(-0.25..0.25) * angle_step;
            let angle = base + jitter;
            let r = radius * rng.gen_range(0.65..1.35);
            (center.x + r * angle.cos(), center.y + r * angle.sin())
        })
        .collect();
    if let Some(&first) = coords.first() {
        coords.push(first);
    }
    Polygon::new(LineString::from(coords), vec![])
}

/// Crumbles an irregular circle of terrain around `target` (radius ~25 units).
pub fn on_wand_crumbling(
    trigger: On<WandCrumblingEvent>,
    mut commands: Commands,
    dungeon_state: Option<ResMut<DungeonState>>,
    dungeon_visuals: Option<ResMut<DungeonVisuals>>,
    dungeon_collider: Option<ResMut<DungeonCollider>>,
    dungeon_nav_mesh: Option<ResMut<DungeonNavMesh>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut nav_meshes: ResMut<Assets<NavMesh2d>>,
    rubble_material: Option<Res<RubbleMaterial>>,
    fragile_query: Query<(Entity, &ColliderAabb), With<Fragile>>,
) {
    let (dungeon_state, dungeon_visuals, dungeon_collider, dungeon_nav_mesh, rubble_material) =
        match (dungeon_state, dungeon_visuals, dungeon_collider, dungeon_nav_mesh, rubble_material)
        {
            (Some(ds), Some(dv), Some(dc), Some(dn), Some(rm)) => (ds, dv, dc, dn, rm),
            _ => return,
        };

    let target = trigger.event().target;
    let mut rng = rand::thread_rng();
    let poly = create_irregular_circle_polygon(target, 160.0, 14, &mut rng);
    let input_multipolygon = MultiPolygon::new(vec![poly]);

    for (entity, aabb) in &fragile_query {
        let rect_poly = Rect::new(Coord { x: aabb.min.x, y: aabb.min.y }, Coord {
            x: aabb.max.x,
            y: aabb.max.y,
        })
        .to_polygon();
        if input_multipolygon.intersects(&rect_poly) {
            commands.entity(entity).despawn();
        }
    }

    subtract_polygon_from_terrain(
        &mut commands,
        &input_multipolygon,
        dungeon_state,
        dungeon_visuals,
        dungeon_collider,
        dungeon_nav_mesh,
        &mut meshes,
        &mut nav_meshes,
        &rubble_material.0,
        Some((target, 60.0)),
    );
}

pub fn handle_right_click_excavation(
    mut commands: Commands,
    window: Single<&Window>,
    camera_query: Single<(&Camera, &GlobalTransform)>,
    mouse_button_input: Res<ButtonInput<MouseButton>>,
    dungeon_state: Option<ResMut<DungeonState>>,
    dungeon_visuals: Option<ResMut<DungeonVisuals>>,
    dungeon_collider: Option<ResMut<DungeonCollider>>,
    dungeon_nav_mesh: Option<ResMut<DungeonNavMesh>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut nav_meshes: ResMut<Assets<NavMesh2d>>,
    rubble_material: Option<Res<RubbleMaterial>>,
    fragile_query: Query<(Entity, &ColliderAabb), With<Fragile>>,
) {
    if !mouse_button_input.just_pressed(MouseButton::Right) {
        return;
    }

    let cursor_position = match window.cursor_position() {
        Some(position) => position,
        None => return,
    };

    let (camera, camera_transform) = *camera_query;
    let world_position = match camera.viewport_to_world_2d(camera_transform, cursor_position) {
        Ok(world_pos) => world_pos,
        Err(_) => return,
    };

    let (dungeon_state, dungeon_visuals, dungeon_collider, dungeon_nav_mesh, rubble_material) =
        match (dungeon_state, dungeon_visuals, dungeon_collider, dungeon_nav_mesh, rubble_material)
        {
            (Some(ds), Some(dv), Some(dc), Some(dn), Some(rm)) => (ds, dv, dc, dn, rm),
            _ => return,
        };

    // Create a circular polygon approximating the excavation area
    let input_polygon =
        create_circle_polygon(world_position, EXCAVATION_RADIUS, EXCAVATION_CIRCLE_POINTS);
    let input_multipolygon = MultiPolygon::new(vec![input_polygon]);

    for (entity, aabb) in &fragile_query {
        let rect_poly = Rect::new(Coord { x: aabb.min.x, y: aabb.min.y }, Coord {
            x: aabb.max.x,
            y: aabb.max.y,
        })
        .to_polygon();
        if input_multipolygon.intersects(&rect_poly) {
            commands.entity(entity).despawn();
        }
    }

    subtract_polygon_from_terrain(
        &mut commands,
        &input_multipolygon,
        dungeon_state,
        dungeon_visuals,
        dungeon_collider,
        dungeon_nav_mesh,
        &mut meshes,
        &mut nav_meshes,
        &rubble_material.0,
        None,
    );
}
