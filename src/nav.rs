// Based on https://github.com/vleue/vleue_navigator/blob/main/examples/helpers/agent2d.rs

use std::ops::Deref;

use bevy::prelude::*;
use vleue_navigator::{NavMesh, prelude::*};

#[derive(Component)]
pub struct Navigator {
    pub speed: f32,
}

#[derive(Component, Default)]
pub struct Path {
    current: Vec2,
    next: Vec<Vec2>,
}

#[derive(Component)]
pub enum Target {
    ILikeItHere,
    Follow(Entity),
    Spot(Vec2),
}

pub fn refresh_path(
    mut commands: Commands,
    mut navigator: Query<(Entity, &Transform, &Target), With<Navigator>>,
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
        return;
    };

    for (entity, transform, target) in &mut navigator {
        let dest_loc = match *target {
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
            commands.entity(entity).remove::<Path>();
            continue;
        }

        let Some(new_path) = navmesh.transformed_path(transform.translation, dest_loc.extend(0.0))
        else {
            commands.entity(entity).remove::<Path>();
            continue;
        };
        if let Some((first, remaining)) = new_path.path.split_first() {
            let mut remaining = remaining.iter().map(|p| p.xy()).collect::<Vec<_>>();
            remaining.reverse();
            commands.entity(entity).insert(Path {
                current: first.xy(),
                next: remaining,
            });
            // path.current = first.xy();
            // path.next = remaining;
            *delta = 0.0;
        }
    }
}

pub fn move_navigator(
    mut commands: Commands,
    mut navigator: Query<(&mut Transform, &mut Path, Entity, &Navigator)>,
    time: Res<Time>,
) {
    for (mut transform, mut path, entity, navigator) in navigator.iter_mut() {
        let move_direction = path.current - transform.translation.xy();
        transform.translation +=
            (move_direction.normalize() * time.delta_secs() * navigator.speed).extend(0.0);
        while transform.translation.xy().distance(path.current) < navigator.speed / 50.0 {
            if let Some(next) = path.next.pop() {
                path.current = next;
            } else {
                commands.entity(entity).remove::<Path>();
                break;
            }
        }
    }
}
