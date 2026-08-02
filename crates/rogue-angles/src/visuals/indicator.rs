// HitFlash: a color flash on missile hit. Pure Bevy, no UI framework — the
// egui-based StateIndicator (character label on AI state change) stays in the
// game crate since it draws through egui, which the engine doesn't depend on
// until the presentation split.
use bevy::prelude::*;

#[derive(Component)]
pub struct HitFlash {
    pub timer: f32,
    pub duration: f32,
    pub base_color: Color,
}

pub fn update_hit_flash(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut query: Query<(Entity, &mut HitFlash, &MeshMaterial2d<ColorMaterial>)>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    for (entity, mut flash, mat_handle) in query.iter_mut() {
        flash.timer -= time.delta_secs();
        if flash.timer <= 0.0 {
            if let Some(mat) = materials.get_mut(&mat_handle.0) {
                mat.color = flash.base_color;
            }
            commands.entity(entity).remove::<HitFlash>();
        } else {
            let t = flash.timer / flash.duration;
            let base = flash.base_color.to_srgba();
            let color = Color::srgba(
                base.red + t * (1.0 - base.red),
                base.green + t * (1.0 - base.green),
                base.blue + t * (1.0 - base.blue),
                base.alpha,
            );
            if let Some(mat) = materials.get_mut(&mat_handle.0) {
                mat.color = color;
            }
        }
    }
}
