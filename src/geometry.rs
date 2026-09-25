//! Shared scene geometry helpers with no GPU dependency: the transform an object's pose builds and the box
//! treads a `stairs` object is made of. Used by the renderer, the physics/hit code and the tools.

use crate::schema::{PrimKind, StairsDef};
use glam::{Mat4, Quat, Vec3};

/// Translation/rotation (degrees, XYZ order)/scale to a matrix — the one transform convention every object uses.
pub fn trs(pos: Vec3, rot_deg: Vec3, scale: Vec3) -> Mat4 {
    let rot = Quat::from_euler(glam::EulerRot::XYZ, rot_deg.x.to_radians(), rot_deg.y.to_radians(), rot_deg.z.to_radians());
    Mat4::from_scale_rotation_translation(scale, rot, pos)
}

/// `steps` solid stacked box treads: step `i` owns its own depth slice of the run
/// (`[-run/2 + i*step_d, -run/2 + (i+1)*step_d]`) and spans height `[0, (i+1)*step_h]` — each
/// box is a self-contained solid block, so the whole thing reads as a real staircase silhouette
/// rather than floating slabs. Purely visual; `collide::ground_height_at` uses a separate smooth
/// ramp formula for actually walking on it (see that function's doc comment for why).
pub fn build_stairs_parts(s: &StairsDef) -> Vec<(PrimKind, Mat4)> {
    let step_h = s.rise / s.steps as f32;
    let step_d = s.run / s.steps as f32;
    (0..s.steps)
        .map(|i| {
            let z_start = -s.run * 0.5 + step_d * i as f32;
            let y_height = step_h * (i + 1) as f32;
            let shape = PrimKind::Box { size: Vec3::new(s.width, y_height, step_d) };
            let center = Vec3::new(0.0, y_height * 0.5, z_start + step_d * 0.5);
            (shape, Mat4::from_translation(center))
        })
        .collect()
}
