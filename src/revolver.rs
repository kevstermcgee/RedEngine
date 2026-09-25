//! The silver revolver's viewmodel: a procedural six-shooter with a polished-steel frame, barrel and
//! fluted cylinder, a dark-walnut grip, the hand wrapped around it, a sleeve, and a muzzle flash.
//!
//! Everything is in the weapon's local frame, the same convention as the bat (ADR 0008: nothing
//! imported): the grip is at the origin, the barrel points `+Z`, `+Y` is up, and the placement
//! basis is `viewer::viewmodel_transform`'s (right, up, forward). The hand sits low on the grip with
//! the sleeve leaving toward the player's bottom-right.

use crate::mesh::Mesh;
use crate::viewer::{append_transformed, flip_winding_for_viewmodel, HeldPart, HAND_COLOR, SLEEVE_COLOR};
use crate::weapons::Weapon;
use glam::{Mat4, Quat, Vec3};

/// Linear-RGB polished steel: light and cool, with enough metalness for a specular sheen.
const STEEL: Vec3 = Vec3::new(0.70, 0.72, 0.76);
const STEEL_DARK: Vec3 = Vec3::new(0.02, 0.02, 0.025);
const WALNUT: Vec3 = Vec3::new(0.16, 0.07, 0.03);
/// Barrel tip along `+Z`, m (where the muzzle flash starts).
pub const MUZZLE_Z: f32 = 0.245;

/// A box of `size` centred at `center`, tilted `tilt_x_deg` about the X axis.
fn boxed(mesh: &mut Mesh, size: Vec3, center: Vec3, tilt_x_deg: f32) {
    append_transformed(mesh, &Mesh::cuboid(size), Mat4::from_translation(center) * Mat4::from_rotation_x(tilt_x_deg.to_radians()));
}

/// A cylinder whose axis runs along `+Z`, `len` long, centred at `center`.
fn tube(mesh: &mut Mesh, radius: f32, len: f32, center: Vec3) {
    append_transformed(mesh, &Mesh::cylinder(radius, len, 20), Mat4::from_translation(center) * Mat4::from_rotation_x(90f32.to_radians()));
}

/// Builds the revolver's parts (already wound for the viewmodel basis' mirror).
pub fn build_revolver_parts() -> Vec<HeldPart> {
    let v = Vec3::new;
    let axis_y = 0.046; // height of the barrel/cylinder axis

    // ---- Steel: frame, barrel, ejector rod, cylinder, hammer, trigger guard, sights -----------
    let mut steel = Mesh::default();
    boxed(&mut steel, v(0.024, 0.052, 0.10), v(0.0, 0.030, 0.030), 0.0); // frame
    tube(&mut steel, 0.0115, 0.165, v(0.0, axis_y, 0.1625)); // barrel (z 0.08 .. 0.245)
    tube(&mut steel, 0.0062, 0.085, v(0.0, axis_y - 0.0175, 0.1225)); // ejector-rod shroud under it
    boxed(&mut steel, v(0.007, 0.006, 0.150), v(0.0, axis_y + 0.0135, 0.165), 0.0); // top rib
    boxed(&mut steel, v(0.004, 0.011, 0.008), v(0.0, axis_y + 0.0225, MUZZLE_Z - 0.008), 0.0); // front sight
    boxed(&mut steel, v(0.014, 0.008, 0.010), v(0.0, 0.062, -0.012), 0.0); // rear sight
    boxed(&mut steel, v(0.008, 0.022, 0.013), v(0.0, 0.066, -0.026), -28.0); // hammer, cocked back
                                                                             // Trigger guard: front post, floor and the trigger inside it.
    boxed(&mut steel, v(0.006, 0.032, 0.007), v(0.0, -0.012, 0.062), 0.0);
    boxed(&mut steel, v(0.006, 0.006, 0.058), v(0.0, -0.028, 0.034), 0.0);
    boxed(&mut steel, v(0.005, 0.020, 0.007), v(0.0, -0.006, 0.030), 12.0);
    // The fluted cylinder: a drum a little wider than the frame, with the front rim.
    tube(&mut steel, 0.0285, 0.050, v(0.0, axis_y, 0.050));

    // ---- Dark: the six chamber mouths on the cylinder's face ----------------------------------
    let mut chambers = Mesh::default();
    for k in 0..6 {
        let a = k as f32 * std::f32::consts::TAU / 6.0;
        tube(&mut chambers, 0.0072, 0.006, v(0.0185 * a.sin(), axis_y + 0.0185 * a.cos(), 0.0755));
    }

    // ---- Walnut grip, leaning back -----------------------------------------------------------
    let mut grip = Mesh::default();
    boxed(&mut grip, v(0.029, 0.088, 0.037), v(0.0, -0.046, -0.021), 14.0);

    // ---- Hand: a fist around the grip, thumb toward the centre of the screen -------------------
    let mut hand = Mesh::default();
    let ellipsoid = |mesh: &mut Mesh, c: Vec3, radii: Vec3| {
        append_transformed(mesh, &Mesh::uv_sphere(1.0, 10, 14), Mat4::from_translation(c) * Mat4::from_scale(radii));
    };
    ellipsoid(&mut hand, v(0.0, -0.046, -0.026), v(0.030, 0.042, 0.027)); // palm and fingers wrapped round
    ellipsoid(&mut hand, v(0.0, -0.040, 0.010), v(0.027, 0.032, 0.010)); // front of the fingers
    ellipsoid(&mut hand, v(-0.019, -0.004, -0.010), v(0.012, 0.013, 0.026)); // thumb along the frame
    let arm = v(0.20, -0.55, -0.80).normalize();
    let wrist = v(0.0, -0.078, -0.040);
    let mut sleeve = Mesh::default();
    let along = |from: f32, len: f32| Mat4::from_translation(wrist + arm * (from + len * 0.5)) * Mat4::from_quat(Quat::from_rotation_arc(Vec3::Y, arm));
    append_transformed(&mut hand, &Mesh::capsule(0.0195, 0.05, 10, 4), along(0.0, 0.05));
    append_transformed(&mut sleeve, &Mesh::cylinder(0.038, 0.035, 14), along(0.045, 0.035));
    append_transformed(&mut sleeve, &Mesh::cylinder(0.030, 0.60, 14), along(0.075, 0.60));

    // ---- Muzzle flash: a hot cone and a bright ball at the barrel tip ---------------------------
    let mut flash = Mesh::default();
    append_transformed(
        &mut flash,
        &Mesh::cone(0.022, 0.14, 14),
        Mat4::from_translation(v(0.0, axis_y, MUZZLE_Z + 0.07)) * Mat4::from_rotation_x(90f32.to_radians()),
    );
    append_transformed(&mut flash, &Mesh::uv_sphere(0.024, 8, 12), Mat4::from_translation(v(0.0, axis_y, MUZZLE_Z + 0.005)));
    // Two thin cross flares so it reads as a burst, not a candle.
    append_transformed(&mut flash, &Mesh::cuboid(v(0.09, 0.004, 0.03)), Mat4::from_translation(v(0.0, axis_y, MUZZLE_Z + 0.02)));
    append_transformed(&mut flash, &Mesh::cuboid(v(0.004, 0.09, 0.03)), Mat4::from_translation(v(0.0, axis_y, MUZZLE_Z + 0.02)));

    let w = Weapon::Revolver;
    let mut parts = vec![
        HeldPart::lit(w, steel, STEEL, 0.7, 0.16, false),
        HeldPart::lit(w, chambers, STEEL_DARK, 0.0, 0.6, false),
        HeldPart::lit(w, grip, WALNUT, 0.0, 0.45, false),
        HeldPart::lit(w, hand, HAND_COLOR, 0.0, 0.5, false),
        HeldPart::lit(w, sleeve, SLEEVE_COLOR, 0.0, 0.85, true),
        HeldPart { emissive: Vec3::new(4.0, 2.4, 0.7), muzzle_flash: true, ..HeldPart::lit(w, flash, Vec3::new(1.0, 0.75, 0.3), 0.0, 0.9, false) },
    ];
    flip_winding_for_viewmodel(&mut parts);
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_revolver_is_a_believable_size_with_its_grip_at_the_origin() {
        let parts = build_revolver_parts();
        let steel = &parts[0].mesh;
        let (mut zmin, mut zmax, mut ymax) = (f32::MAX, f32::MIN, f32::MIN);
        for vtx in &steel.vertices {
            zmin = zmin.min(vtx.pos[2]);
            zmax = zmax.max(vtx.pos[2]);
            ymax = ymax.max(vtx.pos[1]);
        }
        assert!((zmax - MUZZLE_Z).abs() < 0.01, "barrel ends at the muzzle: {zmax}");
        assert!(zmin > -0.06 && zmax - zmin > 0.25 && zmax - zmin < 0.34, "a six-inch revolver, ~30 cm overall: {}", zmax - zmin);
        assert!(ymax < 0.09, "sights sit just above the frame: {ymax}");
    }

    #[test]
    fn only_the_flash_glows_and_everything_belongs_to_the_revolver() {
        let parts = build_revolver_parts();
        assert!(parts.iter().all(|p| p.weapon == Weapon::Revolver));
        assert_eq!(parts.iter().filter(|p| p.muzzle_flash).count(), 1);
        assert!(parts.iter().filter(|p| !p.muzzle_flash).all(|p| p.emissive == Vec3::ZERO));
        assert!(parts.iter().any(|p| p.first_person_only), "the sleeve is first-person only");
    }
}
