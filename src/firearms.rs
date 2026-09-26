//! Procedural first-person models for the reusable Red Engine firearm library.
//!
//! The library intentionally uses engine-native primitives: every model is redistributable,
//! deterministic, lightweight, and available to games without an imported-mesh pipeline.

use crate::mesh::Mesh;
use crate::viewer::{append_transformed, flip_winding_for_viewmodel, HeldPart, HAND_COLOR, SLEEVE_COLOR};
use crate::weapons::Weapon;
use glam::{Mat4, Vec3};

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
    boxed(&mut detail, Vec3::new(s.body_w * 0.72, 0.012, 0.08), Vec3::new(0.0, 0.04 + s.body_h * 0.55, receiver_z));
    boxed(&mut detail, Vec3::new(0.008, 0.018, 0.012), Vec3::new(0.0, 0.075, muzzle_z - 0.018));
    if s.optic {
        tube(&mut detail, 0.018, 0.085, Vec3::new(0.0, 0.095, receiver_z + 0.02));
        boxed(&mut detail, Vec3::new(0.025, 0.025, 0.055), Vec3::new(0.0, 0.078, receiver_z + 0.02));
    }

    let mut hand = Mesh::default();
    append_transformed(
        &mut hand,
        &Mesh::uv_sphere(1.0, 10, 14),
        Mat4::from_translation(Vec3::new(0.0, -0.08, receiver_z - 0.04)) * Mat4::from_scale(Vec3::new(0.035, 0.055, 0.035)),
    );
    let mut sleeve = Mesh::default();
    boxed(&mut sleeve, Vec3::new(0.065, 0.08, 0.24), Vec3::new(0.12, -0.24, receiver_z - 0.18));

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
    fn every_non_revolver_firearm_has_a_visible_model_and_one_flash() {
        for weapon in Weapon::FIREARMS.into_iter().filter(|w| *w != Weapon::Revolver) {
            let parts = build_firearm_parts(weapon);
            assert!(parts.iter().all(|p| p.weapon == weapon));
            assert_eq!(parts.iter().filter(|p| p.muzzle_flash).count(), 1, "{}", weapon.name());
            assert!(parts.iter().map(|p| p.mesh.vertices.len()).sum::<usize>() > 100, "{}", weapon.name());
        }
    }
}
