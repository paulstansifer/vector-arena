// Bridges TerrainGeometry into Bevy/Avian2D entities.
// `geometry_to_mesh` triangulates polygons into a Bevy Mesh.
// `geometry_to_collider` turns polygon rings into an Avian2D polyline Collider.
// `sync_dungeon_to_entities` is a system that rebuilds both whenever DungeonState
// changes (e.g. after excavation). Navmesh types and conversion live in `nav`.
use avian2d::prelude::*;
use bevy::{math::Vec2, prelude::*};
use bevy_mesh::{Indices, PrimitiveTopology};
use geo::{
    BoundingRect, Contains,
    algorithm::{
        buffer::BufferStyle,
        triangulate_delaunay::{DelaunayTriangulationConfig, TriangulateDelaunay},
    },
    buffer::{LineCap, LineJoin},
};
use rand::Rng;

use crate::util::safegeo::{SafeMultiPolygon, SafePolygon};

/// Convert the terrain geometry to a Bevy mesh for rendering.
pub fn geometry_to_mesh(geometry: &SafeMultiPolygon) -> Mesh {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    let mut normals = Vec::new();

    for polygon in geometry.iter() {
        let triangulation = polygon
            .constrained_triangulation(DelaunayTriangulationConfig::default())
            .expect("generating visual terrain mesh");
        for triangle in &triangulation {
            for coord in &[triangle.v1(), triangle.v2(), triangle.v3()] {
                indices.push(positions.len() as u32);
                positions.push([coord.x, coord.y, 0.0]);
                normals.push([0.0, 0.0, 1.0]);
            }
        }
    }

    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, Default::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Convert the terrain geometry to a solid triangle-mesh collider.
pub fn geometry_to_collider(geometry: &SafeMultiPolygon) -> Collider {
    let mut shapes = Vec::new();

    for polygon in geometry.iter() {
        let triangulation = polygon
            .constrained_triangulation(DelaunayTriangulationConfig::default())
            .expect("generating terrain collider");
        for triangle in &triangulation {
            let a = Vec2::new(triangle.v1().x, triangle.v1().y);
            let b = Vec2::new(triangle.v2().x, triangle.v2().y);
            let c = Vec2::new(triangle.v3().x, triangle.v3().y);
            shapes.push((Vec2::ZERO, 0.0_f32, Collider::triangle(a, b, c)));
        }
    }

    if shapes.is_empty() {
        return Collider::circle(0.0);
    }
    Collider::compound(shapes)
}

#[derive(Component)]
pub struct TerrainMarker;

/// Marks a torpor-zone visual entity, carrying the bounding box of the zone so
/// the (render-only) particle effect can seed particles across it. Spawned
/// unconditionally; consumed by `effects::torpor_particles` when that plugin is
/// registered (the rendering binaries), so GPU-less tests can ignore it.
#[derive(Component)]
pub struct TorporZoneParticles {
    pub center: Vec2,
    pub half_size: Vec2,
}

pub const TORPOR_FACTOR: f32 = 0.25;

#[derive(Component, Default)]
pub struct TorporMultiplier(pub f32);

impl TorporMultiplier {
    pub fn get(&self) -> f32 { self.0 }
}

#[derive(Resource)]
pub struct DungeonState {
    pub solid_rock: SafeMultiPolygon,
    pub playable_area: SafeMultiPolygon,
    pub glass_walls: SafeMultiPolygon,
    pub torpor_zones: Vec<SafePolygon>,
}

pub fn torpor_factor_at(pos: Vec2, dungeon_state: &DungeonState) -> f32 {
    let p = geo::Point::new(pos.x, pos.y);
    if dungeon_state.torpor_zones.iter().any(|z| z.contains(&p)) { TORPOR_FACTOR } else { 1.0 }
}

pub fn update_torpor_multipliers(
    dungeon_state: Res<DungeonState>,
    mut query: Query<(&Transform, &mut TorporMultiplier)>,
) {
    for (transform, mut mult) in query.iter_mut() {
        mult.0 = torpor_factor_at(transform.translation.truncate(), &dungeon_state);
    }
}

#[derive(Resource)]
pub struct DungeonCollider(pub Collider);

#[derive(Resource)]
pub struct DungeonVisuals(pub Handle<Mesh>);

#[derive(Component)]
pub struct GlassWallsMarker;

#[derive(Resource)]
pub struct PointsOfInterest {
    pub points: Vec<Vec2>,
}

/// Returns a random point inside `playable_area`, eroded by `AGENT_RADIUS` so results
/// are never right against the wall. Returns `None` after 1000 failed attempts.
pub fn random_in_playable_area(
    playable_area: &SafeMultiPolygon,
    rng: &mut impl Rng,
) -> Option<Vec2> {
    let style =
        BufferStyle::new(-crate::AGENT_RADIUS).line_cap(LineCap::Square).line_join(LineJoin::Bevel);
    let eroded = playable_area.buffer_with_style(style);
    let bbox = eroded.bounding_rect()?;
    for _ in 0..1000 {
        let x = rng.gen_range(bbox.min().x..bbox.max().x);
        let y = rng.gen_range(bbox.min().y..bbox.max().y);
        if eroded.contains(&geo::Point::new(x, y)) {
            return Some(Vec2::new(x, y));
        }
    }
    None
}

/// Returns a random point inside `playable_area` (eroded by `AGENT_RADIUS`) whose distance from
/// `origin` lies within `[min_dist, max_dist]`. Returns `None` after 1000 failed attempts.
pub fn random_near(
    playable_area: &SafeMultiPolygon,
    origin: Vec2,
    min_dist: f32,
    max_dist: f32,
    rng: &mut impl Rng,
) -> Option<Vec2> {
    let style =
        BufferStyle::new(-crate::AGENT_RADIUS).line_cap(LineCap::Square).line_join(LineJoin::Bevel);
    let eroded = playable_area.buffer_with_style(style);
    for _ in 0..1000 {
        let angle = rng.gen_range(0.0..std::f32::consts::TAU);
        let dist = rng.gen_range(min_dist..max_dist);
        let p = origin + Vec2::new(angle.cos(), angle.sin()) * dist;
        if eroded.contains(&geo::Point::new(p.x, p.y)) {
            return Some(p);
        }
    }
    None
}

/// Bevy system to sync dungeon visuals and collider from resources to entity components.
pub fn sync_dungeon_to_entities(
    dungeon_visuals: Res<DungeonVisuals>,
    dungeon_collider: Res<DungeonCollider>,
    mut terrain_query: Query<(&mut Mesh2d, &mut Collider), With<TerrainMarker>>,
) {
    if dungeon_visuals.is_changed()
        && let Ok((mut mesh, _)) = terrain_query.single_mut()
    {
        mesh.0 = dungeon_visuals.0.clone();
    }
    if dungeon_collider.is_changed()
        && let Ok((_, mut collider)) = terrain_query.single_mut()
    {
        *collider = dungeon_collider.0.clone();
    }
}

/// Bevy system to recompute the glass walls mesh and collider from `DungeonState` when it changes.
pub fn sync_glass_walls_to_entities(
    dungeon_state: Res<DungeonState>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut query: Query<(&mut Mesh2d, &mut Collider), With<GlassWallsMarker>>,
) {
    if !dungeon_state.is_changed() {
        return;
    }
    let Ok((mut mesh, mut collider)) = query.single_mut() else { return };
    let border = glass_wall_border(&dungeon_state.glass_walls);
    mesh.0 = meshes.add(geometry_to_mesh(&border));
    *collider = geometry_to_collider(&dungeon_state.glass_walls);
}

/// Computes the 2-unit inner-border ring of the glass walls polygon for rendering.
pub fn glass_wall_border(glass_walls: &SafeMultiPolygon) -> SafeMultiPolygon {
    glass_walls.difference(&glass_walls.buffer_with_style(
        BufferStyle::new(-2.0).line_cap(LineCap::Square).line_join(LineJoin::Bevel),
    ))
}
