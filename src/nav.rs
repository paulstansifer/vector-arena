// Based on https://github.com/vleue/vleue_navigator/blob/main/examples/helpers/agent2d.rs

use std::ops::Deref;

use bevy::prelude::*;
use vleue_navigator::{NavMesh, prelude::*};

#[derive(Component)]
pub struct Navigator {
    pub speed: f32,
    pub current: Vec2,
    pub next: Vec<Vec2>,
    pub target: Target,
}

#[derive(Clone, Copy)]
pub enum Target {
    ILikeItHere,
    Follow(Entity),
    Spot(Vec2),
}

pub fn refresh_path(
    mut navigator: Query<(Entity, &Transform, &mut Navigator)>,
    targets: Query<&Transform>,
    mut navmeshes: ResMut<Assets<NavMesh>>,
    navmesh: Single<(&ManagedNavMesh, Ref<NavMeshStatus>)>,
    mut delta: Local<f32>,
) {
    let (navmesh_handle, status) = navmesh.deref();
    if (!status.is_changed() || **status != NavMeshStatus::Built) && *delta == 0.0 {
        return;
    }
    let Some(navmesh) = navmeshes.get_mut(*navmesh_handle) else {
        panic!("Need a navmesh!")
    };

    for (entity, transform, mut navigator) in &mut navigator {
        let dest_loc = match navigator.target {
            Target::ILikeItHere => continue,
            Target::Follow(entity) => {
                if let Ok(dest) = targets.get(entity) {
                    dest.translation.xy()
                } else {
                    panic!("invalid target")
                }
            }
            Target::Spot(dest) => dest,
        };

        if !navmesh.transformed_is_in_mesh(transform.translation) {
            *delta += 0.1;
            navmesh.set_search_delta(*delta);
            continue;
        }
        if !navmesh.transformed_is_in_mesh(dest_loc.extend(0.0)) {
            navigator.current = Vec2::ZERO;
            navigator.next.clear();
            continue;
        }

        let Some(new_path) = navmesh.transformed_path(transform.translation, dest_loc.extend(0.0))
        else {
            navigator.current = Vec2::ZERO;
            navigator.next.clear();
            continue;
        };
        if let Some((first, remaining)) = new_path.path.split_first() {
            let mut remaining = remaining.iter().map(|p| p.xy()).collect::<Vec<_>>();
            remaining.reverse();
            navigator.current = first.xy();
            navigator.next = remaining;
            *delta = 0.0;
        }
    }
}

pub fn move_navigator(
    mut navigator: Query<(&mut Transform, &mut Navigator, Entity)>,
    time: Res<Time>,
) {
    for (mut transform, mut nav, entity) in navigator.iter_mut() {
        let move_direction = nav.current - transform.translation.xy();
        transform.translation +=
            (move_direction.normalize() * time.delta_secs() * nav.speed).extend(0.0);
        while transform.translation.xy().distance(nav.current) < nav.speed / 50.0 {
            if let Some(next) = nav.next.pop() {
                nav.current = next;
            } else {
                nav.current = Vec2::ZERO;
                nav.next.clear();
                break;
            }
        }
    }
}
