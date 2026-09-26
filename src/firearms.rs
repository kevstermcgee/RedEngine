//! Procedural first-person models for the reusable Red Engine firearm library.
//!
//! The library intentionally uses engine-native primitives: every model is redistributable,
//! deterministic, lightweight, and available to games without an imported-mesh pipeline.

use crate::mesh::Mesh;
use crate::viewer::{append_transformed, flip_winding_for_viewmodel, HeldPart, HAND_COLOR, SLEEVE_COLOR};
use crate::weapons::Weapon;
use glam::{Mat4, Quat, Vec3};

/// Sight height above the model origin, shared by geometry and camera alignment.
pub fn sight_height(weapon: Weapon) -> f32 {
    if weapon == Weapon::Revolver {
        0.074
    } else {
        0.04 + shape(weapon).body_h * 0.5 + 0.045
    }
}

/// Primary grip anchor used to attach the weapon to a third-person wrist.
pub fn grip_anchor(weapon: Weapon) -> Vec3 {
    if weapon == Weapon::Revolver {
        return Vec3::new(0.0, -0.046, -0.026);
    }
    let s = shape(weapon);
    Vec3::new(0.0, -0.065, 0.10 + s.stock + (s.length - s.stock - s.barrel) * 0.5 - 0.03)
}

/// Camera-local pose: sights converge to the center ray without hip yaw or pitch.
pub fn held_pose(weapon: Weapon, ads: f32, kick: f32, dip: f32) -> (Vec3, Mat4) {
    let ads = ads.clamp(0.0, 1.0);
    let grip = grip_anchor(weapon);
    let hip = Vec3::new(0.18, -0.19, 0.38) - Vec3::new(0.0, 0.0, grip.z);
    let aimed = Vec3::new(0.0, -sight_height(weapon), 0.32 - grip.z);
    let offset = hip.lerp(aimed, ads) + Vec3::new(0.0, -0.30 * dip, -0.045 * kick);
    let rotation =
        Mat4::from_rotation_y((-8.0 * (1.0 - ads)).to_radians()) * Mat4::from_rotation_x((-3.0 * (1.0 - ads) - 12.0 * kick + 25.0 * dip).to_radians());
    (offset, rotation)
}

/// Screen-space sensitivity compensation across a changing perspective FOV.
pub fn zoom_sensitivity(fov: f32, hip_fov: f32) -> f32 {
    (0.5 * fov.to_radians()).tan() / (0.5 * hip_fov.to_radians()).tan()
}

#[derive(Clone, Copy)]
struct Shape {
    length: f32,
    stock: f32,
    barrel: f32,
    body_h: f32,
    body_w: f32,
    magazine: f32,
    optic: bool,
    color: Vec3,
}

fn shape(w: Weapon) -> Shape {
    let red = Vec3::new(0.34, 0.025, 0.035);
    let slate = Vec3::new(0.055, 0.065, 0.08);
    match w {
        Weapon::Pistol => Shape { length: 0.22, stock: 0.0, barrel: 0.11, body_h: 0.055, body_w: 0.032, magazine: 0.10, optic: false, color: slate },
        Weapon::MachinePistol => Shape { length: 0.27, stock: 0.08, barrel: 0.14, body_h: 0.07, body_w: 0.038, magazine: 0.15, optic: false, color: red },
        Weapon::Smg => Shape { length: 0.42, stock: 0.15, barrel: 0.17, body_h: 0.09, body_w: 0.05, magazine: 0.20, optic: true, color: slate },
        Weapon::Carbine => Shape { length: 0.62, stock: 0.20, barrel: 0.26, body_h: 0.085, body_w: 0.05, magazine: 0.18, optic: true, color: red },
        Weapon::Rifle => Shape { length: 0.76, stock: 0.23, barrel: 0.34, body_h: 0.09, body_w: 0.052, magazine: 0.20, optic: true, color: slate },
        Weapon::Bullpup => Shape { length: 0.58, stock: 0.18, barrel: 0.31, body_h: 0.11, body_w: 0.055, magazine: 0.15, optic: true, color: red },
        Weapon::Marksman => Shape { length: 0.88, stock: 0.25, barrel: 0.43, body_h: 0.085, body_w: 0.048, magazine: 0.13, optic: true, color: slate },
        Weapon::Shotgun => Shape { length: 0.82, stock: 0.25, barrel: 0.48, body_h: 0.075, body_w: 0.052, magazine: 0.0, optic: false, color: red },
        Weapon::Lmg => Shape { length: 0.84, stock: 0.23, barrel: 0.38, body_h: 0.13, body_w: 0.072, magazine: 0.22, optic: true, color: slate },
        Weapon::Scout => Shape { length: 0.94, stock: 0.27, barrel: 0.50, body_h: 0.07, body_w: 0.045, magazine: 0.10, optic: true, color: red },
        _ => unreachable!("firearms.rs only builds the non-revolver firearm library"),
    }
}

fn boxed(mesh: &mut Mesh, size: Vec3, center: Vec3) {
    append_transformed(mesh, &Mesh::cuboid(size), Mat4::from_translation(center));
}

fn tube(mesh: &mut Mesh, radius: f32, length: f32, center: Vec3) {
    append_transformed(mesh, &Mesh::cylinder(radius, length, 16), Mat4::from_translation(center) * Mat4::from_rotation_x(90f32.to_radians()));
}

/// Builds one distinctive, correctly scaled firearm and its hands, sight and muzzle flash.
pub fn build_firearm_parts(weapon: Weapon) -> Vec<HeldPart> {
    let s = shape(weapon);
    let receiver_z = 0.10 + s.stock + (s.length - s.stock - s.barrel) * 0.5;
    let muzzle_z = s.length;
    let mut body = Mesh::default();
    boxed(&mut body, Vec3::new(s.body_w, s.body_h, (s.length - s.stock - s.barrel).max(0.10)), Vec3::new(0.0, 0.04, receiver_z));
    tube(&mut body, s.body_w * 0.20, s.barrel, Vec3::new(0.0, 0.055, s.length - s.barrel * 0.5));
    if s.stock > 0.0 {
        boxed(&mut body, Vec3::new(s.body_w * 0.72, s.body_h * 0.72, s.stock), Vec3::new(0.0, 0.025, 0.10 + s.stock * 0.5));
    }
    boxed(&mut body, Vec3::new(s.body_w * 0.58, 0.12, 0.036), Vec3::new(0.0, -0.035, receiver_z - 0.03));
    if s.magazine > 0.0 {
        boxed(&mut body, Vec3::new(s.body_w * 0.65, s.magazine, 0.045), Vec3::new(0.0, -0.06 - s.magazine * 0.5, receiver_z + 0.035));
    } else {
        tube(&mut body, s.body_w * 0.22, s.length * 0.55, Vec3::new(0.0, 0.01, s.length * 0.56));
    }

    let mut detail = Mesh::default();
    let sight = sight_height(weapon);
    if s.optic {
        // An open rectangular aperture. No end caps or opaque lens on the aim ray.
        let z = receiver_z + 0.02;
        for x in [-0.024, 0.024] {
            boxed(&mut detail, Vec3::new(0.006, 0.046, 0.045), Vec3::new(x, sight, z));
        }
        for y in [-0.025, 0.025] {
            boxed(&mut detail, Vec3::new(0.054, 0.006, 0.045), Vec3::new(0.0, sight + y, z));
        }
        boxed(&mut detail, Vec3::new(0.022, 0.020, 0.045), Vec3::new(0.0, sight - 0.035, z));
    } else {
        // Front-post tip and rear-notch top share the same authored sight line.
        boxed(&mut detail, Vec3::new(0.004, 0.025, 0.010), Vec3::new(0.0, sight - 0.0125, muzzle_z - 0.018));
        for x in [-0.012, 0.012] {
            boxed(&mut detail, Vec3::new(0.009, 0.021, 0.012), Vec3::new(x, sight - 0.0105, receiver_z - 0.07));
        }
    }

    let mut hand = Mesh::default();
    append_transformed(
        &mut hand,
        &Mesh::uv_sphere(1.0, 10, 14),
        Mat4::from_translation(grip_anchor(weapon)) * Mat4::from_scale(Vec3::new(0.034, 0.047, 0.033)),
    );
    let mut sleeve = Mesh::default();
    let arm = |mesh: &mut Mesh, wrist: Vec3, elbow: Vec3| {
        let delta = elbow - wrist;
        append_transformed(
            mesh,
            &Mesh::cylinder(0.035, delta.length(), 14),
            Mat4::from_translation((wrist + elbow) * 0.5) * Mat4::from_quat(Quat::from_rotation_arc(Vec3::Y, delta.normalize())),
        );
    };
    let grip = grip_anchor(weapon);
    arm(&mut sleeve, grip + Vec3::new(0.0, -0.025, -0.012), grip + Vec3::new(0.18, -0.32, -0.35));
    if s.stock > 0.0 {
        let support = Vec3::new(-0.018, -0.005, (s.length - s.barrel * 0.65).max(receiver_z + 0.10));
        append_transformed(&mut hand, &Mesh::uv_sphere(1.0, 10, 14), Mat4::from_translation(support) * Mat4::from_scale(Vec3::new(0.038, 0.030, 0.055)));
        arm(&mut sleeve, support + Vec3::new(-0.008, -0.014, -0.01), grip + Vec3::new(-0.24, -0.30, -0.23));
    }

    let mut flash = Mesh::default();
    append_transformed(
        &mut flash,
        &Mesh::cone(0.032, 0.18, 14),
        Mat4::from_translation(Vec3::new(0.0, 0.055, muzzle_z + 0.09)) * Mat4::from_rotation_x(90f32.to_radians()),
    );
    append_transformed(&mut flash, &Mesh::uv_sphere(0.026, 8, 12), Mat4::from_translation(Vec3::new(0.0, 0.055, muzzle_z + 0.01)));

    let mut parts = vec![
        HeldPart::lit(weapon, body, s.color, 0.35, 0.38, false),
        HeldPart::lit(weapon, detail, Vec3::new(0.015, 0.018, 0.022), 0.55, 0.25, false),
        HeldPart::lit(weapon, hand, HAND_COLOR, 0.0, 0.5, false),
        HeldPart::lit(weapon, sleeve, SLEEVE_COLOR, 0.0, 0.85, true),
        HeldPart { emissive: Vec3::new(4.2, 2.2, 0.55), muzzle_flash: true, ..HeldPart::lit(weapon, flash, Vec3::new(1.0, 0.65, 0.2), 0.0, 0.9, false) },
    ];
    flip_winding_for_viewmodel(&mut parts);
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sight_axis_matches_the_camera_ray_when_aimed() {
        for weapon in Weapon::FIREARMS {
            let (offset, rotation) = held_pose(weapon, 1.0, 0.0, 0.0);
            for z in [0.0, 0.25, 1.0] {
                let point = offset + rotation.transform_point3(Vec3::new(0.0, sight_height(weapon), z));
                assert!(point.x.abs() < 1e-6 && point.y.abs() < 1e-6, "{weapon:?}: {point:?}");
            }
            assert!((rotation.transform_vector3(Vec3::Z) - Vec3::Z).length() < 1e-6);
        }
    }

    #[test]
    fn zoom_preserves_small_screen_space_mouse_motion() {
        for hip in [60.0_f32, 75.0, 90.0, 110.0] {
            for zoom in [1.0_f32, 1.25, 2.0, 2.5] {
                let aimed = (2.0 * ((0.5 * hip.to_radians()).tan() / zoom).atan()).to_degrees();
                assert!((zoom_sensitivity(aimed, hip) * zoom - 1.0).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn every_non_revolver_firearm_has_a_visible_model_and_one_flash() {
        for weapon in Weapon::FIREARMS.into_iter().filter(|w| *w != Weapon::Revolver) {
            let parts = build_firearm_parts(weapon);
            assert!(parts.iter().all(|p| p.weapon == weapon));
            assert_eq!(parts.iter().filter(|p| p.muzzle_flash).count(), 1, "{}", weapon.name());
            assert!(parts.iter().map(|p| p.mesh.vertices.len()).sum::<usize>() > 100, "{}", weapon.name());
        }
    }
}
