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
    if !weapon.is_gun() {
        return 0.0;
    }
    0.04 + shape(weapon).body_h * 0.5 + 0.045
}

/// Primary grip anchor used to attach the weapon to a third-person wrist.
pub fn grip_anchor(weapon: Weapon) -> Vec3 {
    if !weapon.is_gun() {
        return Vec3::new(0.0, -0.02, 0.0);
    }
    let s = shape(weapon);
    Vec3::new(0.0, -0.065, 0.10 + s.stock + (s.length - s.stock - s.barrel) * 0.5 - 0.03)
}

/// Camera-local pose: sights converge to the center ray without hip yaw or pitch.
pub fn held_pose(weapon: Weapon, ads: f32, kick: f32, dip: f32) -> (Vec3, Mat4) {
    if !weapon.is_gun() {
        // A knife or hatchet is held low and forward and slashes across the view as `kick` rises; a grenade sits in the hand and is pulled back to throw.
        let swing = kick.clamp(0.0, 1.0);
        let offset = Vec3::new(0.20 - 0.40 * swing, -0.20 + 0.04 * swing - 0.30 * dip, 0.42 + 0.10 * swing);
        let rotation = Mat4::from_rotation_y((-18.0 + 52.0 * swing).to_radians()) * Mat4::from_rotation_x((-12.0 + 40.0 * swing + 25.0 * dip).to_radians());
        return (offset, rotation);
    }
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

/// What else a gun carries besides its receiver, barrel, stock and box magazine.
#[derive(Clone, Copy, PartialEq)]
enum Extra {
    Plain,
    /// A revolver's cylinder.
    Drum,
    /// A tube magazine under the barrel.
    Tube,
    /// A second barrel beside the first.
    Double,
    /// A fat launch tube with a warhead at the muzzle.
    Launcher,
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
    extra: Extra,
    /// A long scope tube over the receiver instead of the open aperture.
    scope: bool,
    /// Barrel radius (0 = a fifth of the body width).
    bore: f32,
}

/// A classic gun shape: `(length, stock, barrel, body_h, body_w, magazine, optic)`.
const fn sh(length: f32, stock: f32, barrel: f32, body_h: f32, body_w: f32, magazine: f32, optic: bool, color: Vec3) -> Shape {
    Shape { length, stock, barrel, body_h, body_w, magazine, optic, color, extra: Extra::Plain, scope: false, bore: 0.0 }
}

fn shape(w: Weapon) -> Shape {
    let red = Vec3::new(0.34, 0.025, 0.035);
    let slate = Vec3::new(0.055, 0.065, 0.08);
    let black = Vec3::new(0.022, 0.024, 0.026);
    let steel = Vec3::new(0.16, 0.17, 0.18);
    let olive = Vec3::new(0.085, 0.10, 0.05);
    let tan = Vec3::new(0.30, 0.22, 0.12);
    match w {
        Weapon::Pistol => sh(0.22, 0.0, 0.11, 0.055, 0.032, 0.10, false, slate),
        Weapon::MachinePistol => sh(0.27, 0.08, 0.14, 0.07, 0.038, 0.15, false, red),
        Weapon::Smg => sh(0.42, 0.15, 0.17, 0.09, 0.05, 0.20, true, slate),
        Weapon::Carbine => sh(0.62, 0.20, 0.26, 0.085, 0.05, 0.18, true, red),
        Weapon::Rifle => sh(0.76, 0.23, 0.34, 0.09, 0.052, 0.20, true, slate),
        Weapon::Bullpup => sh(0.58, 0.18, 0.31, 0.11, 0.055, 0.15, true, red),
        Weapon::Marksman => sh(0.88, 0.25, 0.43, 0.085, 0.048, 0.13, true, slate),
        Weapon::Shotgun => Shape { extra: Extra::Tube, ..sh(0.82, 0.25, 0.48, 0.075, 0.052, 0.0, false, red) },
        Weapon::Lmg => sh(0.84, 0.23, 0.38, 0.13, 0.072, 0.22, true, slate),
        Weapon::Scout => sh(0.94, 0.27, 0.50, 0.07, 0.045, 0.10, true, red),
        Weapon::Bulldog => sh(0.23, 0.0, 0.12, 0.055, 0.034, 0.09, false, steel),
        Weapon::HandCannon => sh(0.27, 0.0, 0.15, 0.072, 0.042, 0.095, false, steel),
        Weapon::Marshal => Shape { extra: Extra::Drum, ..sh(0.27, 0.0, 0.14, 0.06, 0.036, 0.0, false, steel) },
        Weapon::Stinger => sh(0.45, 0.17, 0.18, 0.09, 0.05, 0.18, true, black),
        Weapon::Ranger => sh(0.50, 0.20, 0.16, 0.105, 0.06, 0.10, true, black),
        Weapon::Wasp => sh(0.40, 0.15, 0.16, 0.10, 0.05, 0.17, true, black),
        Weapon::Ironside => sh(0.80, 0.24, 0.36, 0.095, 0.055, 0.15, true, tan),
        Weapon::Gale => Shape { scope: true, ..sh(1.00, 0.28, 0.50, 0.09, 0.05, 0.14, true, olive) },
        Weapon::Sentinel => Shape { scope: true, ..sh(1.25, 0.32, 0.62, 0.085, 0.048, 0.11, true, olive) },
        Weapon::Auto12 => Shape { extra: Extra::Tube, ..sh(0.85, 0.25, 0.46, 0.085, 0.055, 0.0, false, black) },
        Weapon::Coach => Shape { extra: Extra::Double, ..sh(0.65, 0.20, 0.36, 0.06, 0.05, 0.0, false, steel) },
        Weapon::Hammer => sh(0.95, 0.26, 0.42, 0.14, 0.075, 0.22, true, olive),
        Weapon::Lancer => Shape { extra: Extra::Launcher, bore: 0.042, ..sh(1.05, 0.0, 0.78, 0.07, 0.05, 0.0, false, olive) },
        Weapon::Thumper => Shape { extra: Extra::Launcher, bore: 0.024, ..sh(0.76, 0.22, 0.42, 0.06, 0.042, 0.0, false, black) },
        Weapon::Bat | Weapon::Knife | Weapon::Hatchet | Weapon::Frag | Weapon::Flash | Weapon::Smoke | Weapon::Incendiary => {
            unreachable!("melee weapons and grenades are built by their own functions, not the firearm library")
        }
    }
}

fn boxed(mesh: &mut Mesh, size: Vec3, center: Vec3) {
    append_transformed(mesh, &Mesh::cuboid(size), Mat4::from_translation(center));
}

fn tube(mesh: &mut Mesh, radius: f32, length: f32, center: Vec3) {
    append_transformed(mesh, &Mesh::cylinder(radius, length, 16), Mat4::from_translation(center) * Mat4::from_rotation_x(90f32.to_radians()));
}

/// The two looks a hand and sleeve can have in a loadout match, matching `uniforms`; skin `0` (no team) keeps the prototype's slate sleeve and skin-tone hand.
pub const SKINS: [u8; 3] = [0, 1, 2];

/// The sleeve and hand colours for a hand skin.
pub fn skin_colors(skin: u8) -> (Vec3, Vec3) {
    match crate::uniforms::by_skin(skin) {
        Some(u) => (u.sleeve, u.glove),
        None => (SLEEVE_COLOR, HAND_COLOR),
    }
}

/// Builds the knife, the hatchet and the four grenades (and the hands that hold them).
pub fn build_thrown_and_melee_parts(weapon: Weapon) -> Vec<HeldPart> {
    let steel = Vec3::new(0.42, 0.44, 0.47);
    let black = Vec3::new(0.02, 0.022, 0.025);
    let wood = Vec3::new(0.26, 0.13, 0.05);
    let mut body = Mesh::default();
    let mut detail = Mesh::default();
    let (body_color, detail_color) = match weapon {
        Weapon::Knife => {
            boxed(&mut body, Vec3::new(0.004, 0.034, 0.17), Vec3::new(0.0, 0.0, 0.14));
            boxed(&mut body, Vec3::new(0.004, 0.012, 0.05), Vec3::new(0.0, -0.012, 0.25));
            boxed(&mut detail, Vec3::new(0.022, 0.03, 0.12), Vec3::new(0.0, 0.0, -0.01));
            boxed(&mut detail, Vec3::new(0.012, 0.05, 0.012), Vec3::new(0.0, 0.0, 0.055));
            (steel, black)
        }
        Weapon::Hatchet => {
            boxed(&mut detail, Vec3::new(0.03, 0.03, 0.40), Vec3::new(0.0, 0.0, 0.10));
            boxed(&mut body, Vec3::new(0.018, 0.10, 0.09), Vec3::new(0.0, 0.03, 0.27));
            boxed(&mut body, Vec3::new(0.006, 0.12, 0.03), Vec3::new(0.0, 0.03, 0.325));
            boxed(&mut body, Vec3::new(0.03, 0.04, 0.04), Vec3::new(0.0, 0.015, 0.22));
            (steel, wood)
        }
        _ => {
            // A grenade: a canister in the fist with a spoon (or a cap) on top.
            let (radius, height, color, band) = match weapon {
                Weapon::Frag => (0.036, 0.085, Vec3::new(0.06, 0.08, 0.03), Vec3::new(0.35, 0.3, 0.05)),
                Weapon::Flash => (0.028, 0.115, Vec3::new(0.16, 0.17, 0.18), black),
                Weapon::Smoke => (0.030, 0.115, Vec3::new(0.25, 0.26, 0.27), Vec3::new(0.55, 0.55, 0.2)),
                _ => (0.030, 0.115, Vec3::new(0.30, 0.04, 0.03), Vec3::new(0.65, 0.4, 0.05)),
            };
            if weapon == Weapon::Frag {
                append_transformed(&mut body, &Mesh::uv_sphere(1.0, 12, 16), Mat4::from_translation(Vec3::new(0.0, 0.0, 0.05)) * Mat4::from_scale(Vec3::new(radius, radius * 1.15, radius)));
            } else {
                append_transformed(&mut body, &Mesh::cylinder(radius, height, 16), Mat4::from_translation(Vec3::new(0.0, 0.0, 0.05)));
            }
            append_transformed(&mut detail, &Mesh::cylinder(radius * 0.55, 0.03, 12), Mat4::from_translation(Vec3::new(0.0, height * 0.5 + 0.012, 0.05)));
            boxed(&mut detail, Vec3::new(0.012, 0.012, 0.075), Vec3::new(0.0, height * 0.5 + 0.03, 0.085));
            boxed(&mut detail, Vec3::new(radius * 2.06, 0.018, radius * 2.06), Vec3::new(0.0, 0.0, 0.05));
            (color, band)
        }
    };
    let mut hand = Mesh::default();
    let grip = grip_anchor(weapon);
    append_transformed(&mut hand, &Mesh::uv_sphere(1.0, 10, 14), Mat4::from_translation(grip) * Mat4::from_scale(Vec3::new(0.040, 0.048, 0.046)));
    let mut sleeve = Mesh::default();
    let (wrist, elbow) = (grip + Vec3::new(0.0, -0.03, -0.02), grip + Vec3::new(0.16, -0.30, -0.36));
    let delta = elbow - wrist;
    append_transformed(
        &mut sleeve,
        &Mesh::cylinder(0.036, delta.length(), 14),
        Mat4::from_translation((wrist + elbow) * 0.5) * Mat4::from_quat(Quat::from_rotation_arc(Vec3::Y, delta.normalize())),
    );
    let mut parts = vec![HeldPart::lit(weapon, body, body_color, 0.4, 0.4, false), HeldPart::lit(weapon, detail, detail_color, 0.2, 0.5, false)];
    parts.extend(crate::viewer::skinned_hands(weapon, &hand, &sleeve));
    flip_winding_for_viewmodel(&mut parts);
    parts
}

/// Builds one distinctive, correctly scaled firearm and its hands, sight and muzzle flash.
pub fn build_firearm_parts(weapon: Weapon) -> Vec<HeldPart> {
    let s = shape(weapon);
    let receiver_z = 0.10 + s.stock + (s.length - s.stock - s.barrel) * 0.5;
    let muzzle_z = s.length;
    let bore = if s.bore > 0.0 { s.bore } else { s.body_w * 0.20 };
    let mut body = Mesh::default();
    boxed(&mut body, Vec3::new(s.body_w, s.body_h, (s.length - s.stock - s.barrel).max(0.10)), Vec3::new(0.0, 0.04, receiver_z));
    tube(&mut body, bore, s.barrel, Vec3::new(0.0, 0.055, s.length - s.barrel * 0.5));
    if s.stock > 0.0 {
        boxed(&mut body, Vec3::new(s.body_w * 0.72, s.body_h * 0.72, s.stock), Vec3::new(0.0, 0.025, 0.10 + s.stock * 0.5));
    }
    boxed(&mut body, Vec3::new(s.body_w * 0.58, 0.12, 0.036), Vec3::new(0.0, -0.035, receiver_z - 0.03));
    match s.extra {
        Extra::Plain => {}
        Extra::Drum => append_transformed(
            &mut body,
            &Mesh::cylinder(s.body_h * 0.62, s.body_w * 1.5, 16),
            Mat4::from_translation(Vec3::new(0.0, 0.04, receiver_z + 0.02)) * Mat4::from_rotation_z(90f32.to_radians()),
        ),
        Extra::Tube => tube(&mut body, s.body_w * 0.22, s.length * 0.55, Vec3::new(0.0, 0.01, s.length * 0.56)),
        Extra::Double => {
            tube(&mut body, bore * 1.2, s.barrel, Vec3::new(0.0, 0.055 - bore * 2.4, s.length - s.barrel * 0.5));
        }
        Extra::Launcher => {
            // A flared blast cone at the breech and the warhead's nose standing out of the muzzle.
            append_transformed(
                &mut body,
                &Mesh::cone(bore * 1.9, 0.20, 16),
                Mat4::from_translation(Vec3::new(0.0, 0.055, 0.10)) * Mat4::from_rotation_x(-90f32.to_radians()),
            );
            append_transformed(
                &mut body,
                &Mesh::cone(bore * 1.45, 0.17, 14),
                Mat4::from_translation(Vec3::new(0.0, 0.055, muzzle_z + 0.05)) * Mat4::from_rotation_x(90f32.to_radians()),
            );
            boxed(&mut body, Vec3::new(0.02, 0.05, 0.06), Vec3::new(0.0, 0.055 - bore - 0.012, receiver_z));
        }
    }
    if s.magazine > 0.0 {
        boxed(&mut body, Vec3::new(s.body_w * 0.65, s.magazine, 0.045), Vec3::new(0.0, -0.06 - s.magazine * 0.5, receiver_z + 0.035));
    }

    let mut detail = Mesh::default();
    let sight = sight_height(weapon);
    if s.scope {
        // A scope tube with bell ends on a rail; aimed, the client shows the scope picture instead.
        let z = receiver_z + 0.02;
        tube(&mut detail, 0.022, 0.30, Vec3::new(0.0, sight + 0.004, z));
        tube(&mut detail, 0.030, 0.07, Vec3::new(0.0, sight + 0.004, z + 0.15));
        tube(&mut detail, 0.027, 0.06, Vec3::new(0.0, sight + 0.004, z - 0.15));
        boxed(&mut detail, Vec3::new(0.012, 0.03, 0.05), Vec3::new(0.0, sight - 0.028, z + 0.07));
        boxed(&mut detail, Vec3::new(0.012, 0.03, 0.05), Vec3::new(0.0, sight - 0.028, z - 0.07));
    } else if s.optic {
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
    if s.stock > 0.0 || s.extra == Extra::Launcher {
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
        HeldPart { emissive: Vec3::new(4.2, 2.2, 0.55), muzzle_flash: true, ..HeldPart::lit(weapon, flash, Vec3::new(1.0, 0.65, 0.2), 0.0, 0.9, false) },
    ];
    parts.extend(crate::viewer::skinned_hands(weapon, &hand, &sleeve));
    flip_winding_for_viewmodel(&mut parts);
    parts
}

/// One primitive of a world model.
fn part(id: String, kind: crate::schema::PrimKind, pos: Vec3, rot_deg: Vec3, color: Vec3, metallic: f32) -> crate::schema::Object {
    use crate::schema::{Material, Object, ObjectKind};
    use crate::track::Track;
    Object {
        id,
        position: Track::constant(pos),
        rotation: Track::constant(rot_deg),
        scale: Track::constant(Vec3::ONE),
        material: Some(Material { color: Track::constant(color), metallic, roughness: 0.5, emissive: Vec3::ZERO, opacity: 1.0 }),
        collide: false,
        prefab: None,
        movable: Some(false),
        kind: ObjectKind::Prim(kind),
    }
}

/// The model of a weapon lying on the floor (or a crate of ammunition when `weapon` is `None`): a group of plain primitives, barrel along +Z, the
/// sights up, its origin under the middle of the weapon. Rotate the group about Y to turn it; lift it a hand's width off the floor.
pub fn world_model(weapon: Option<Weapon>, id: &str) -> crate::schema::Object {
    use crate::schema::{ObjectKind, PrimKind};
    use crate::track::Track;
    let mut kids: Vec<crate::schema::Object> = Vec::new();
    let mut n = 0;
    let mut add = |kind: PrimKind, pos: Vec3, rot: Vec3, color: Vec3, metallic: f32| {
        n += 1;
        kids.push(part(format!("{id}_{n}"), kind, pos, rot, color, metallic));
    };
    let black = Vec3::new(0.02, 0.022, 0.025);
    let steel = Vec3::new(0.42, 0.44, 0.47);
    match weapon {
        None => {
            let olive = Vec3::new(0.07, 0.085, 0.035);
            add(PrimKind::Box { size: Vec3::new(0.46, 0.24, 0.30) }, Vec3::new(0.0, 0.12, 0.0), Vec3::ZERO, olive, 0.1);
            add(PrimKind::Box { size: Vec3::new(0.48, 0.03, 0.12) }, Vec3::new(0.0, 0.255, 0.0), Vec3::ZERO, Vec3::new(0.45, 0.4, 0.06), 0.0);
            add(PrimKind::Box { size: Vec3::new(0.06, 0.14, 0.32) }, Vec3::new(0.16, 0.12, 0.0), Vec3::ZERO, Vec3::new(0.12, 0.14, 0.06), 0.0);
            add(PrimKind::Box { size: Vec3::new(0.06, 0.14, 0.32) }, Vec3::new(-0.16, 0.12, 0.0), Vec3::ZERO, Vec3::new(0.12, 0.14, 0.06), 0.0);
        }
        Some(w) if w.is_melee() => match w {
            Weapon::Hatchet => {
                add(PrimKind::Box { size: Vec3::new(0.04, 0.04, 0.42) }, Vec3::new(0.0, 0.03, 0.0), Vec3::ZERO, Vec3::new(0.26, 0.13, 0.05), 0.0);
                add(PrimKind::Box { size: Vec3::new(0.03, 0.14, 0.11) }, Vec3::new(0.0, 0.08, 0.17), Vec3::ZERO, steel, 0.8);
            }
            Weapon::Bat => {
                add(PrimKind::Capsule { radius: 0.035, height: 0.8 }, Vec3::new(0.0, 0.04, 0.0), Vec3::new(90.0, 0.0, 0.0), Vec3::new(0.65, 0.4, 0.2), 0.0);
            }
            _ => {
                add(PrimKind::Box { size: Vec3::new(0.03, 0.03, 0.12) }, Vec3::new(0.0, 0.03, -0.09), Vec3::ZERO, black, 0.0);
                add(PrimKind::Box { size: Vec3::new(0.008, 0.04, 0.2) }, Vec3::new(0.0, 0.03, 0.08), Vec3::ZERO, steel, 0.9);
            }
        },
        Some(w) if w.is_grenade() => {
            let (color, r) = match w {
                Weapon::Frag => (Vec3::new(0.06, 0.08, 0.03), 0.05),
                Weapon::Flash => (Vec3::new(0.16, 0.17, 0.18), 0.04),
                Weapon::Smoke => (Vec3::new(0.25, 0.26, 0.27), 0.04),
                _ => (Vec3::new(0.30, 0.04, 0.03), 0.04),
            };
            if w == Weapon::Frag {
                add(PrimKind::Sphere { radius: r }, Vec3::new(0.0, r, 0.0), Vec3::ZERO, color, 0.2);
            } else {
                add(PrimKind::Cylinder { radius: r, height: 0.14 }, Vec3::new(0.0, r, 0.0), Vec3::new(90.0, 0.0, 0.0), color, 0.3);
            }
            add(PrimKind::Box { size: Vec3::new(0.02, 0.03, 0.05) }, Vec3::new(0.0, r * 2.0, 0.0), Vec3::ZERO, steel, 0.8);
        }
        Some(w) => {
            let s = shape(w);
            let receiver_len = (s.length - s.stock - s.barrel).max(0.10);
            let z0 = -s.length * 0.5;
            let bore = if s.bore > 0.0 { s.bore } else { s.body_w * 0.20 };
            add(PrimKind::Box { size: Vec3::new(s.body_w, s.body_h, receiver_len) }, Vec3::new(0.0, 0.05 + s.body_h * 0.5, z0 + s.stock + receiver_len * 0.5), Vec3::ZERO, s.color, 0.4);
            add(
                PrimKind::Cylinder { radius: bore.max(0.012), height: s.barrel },
                Vec3::new(0.0, 0.05 + s.body_h * 0.6, z0 + s.length - s.barrel * 0.5),
                Vec3::new(90.0, 0.0, 0.0),
                if s.extra == Extra::Launcher { s.color } else { black },
                0.6,
            );
            if s.stock > 0.0 {
                add(PrimKind::Box { size: Vec3::new(s.body_w * 0.75, s.body_h * 0.8, s.stock) }, Vec3::new(0.0, 0.05 + s.body_h * 0.45, z0 + s.stock * 0.5), Vec3::ZERO, s.color, 0.1);
            }
            if s.magazine > 0.0 {
                add(PrimKind::Box { size: Vec3::new(s.body_w * 0.65, s.magazine, 0.05) }, Vec3::new(0.0, 0.05 - s.magazine * 0.4, z0 + s.stock + receiver_len * 0.6), Vec3::ZERO, black, 0.2);
            }
            if s.scope {
                add(PrimKind::Cylinder { radius: 0.025, height: 0.3 }, Vec3::new(0.0, 0.05 + s.body_h + 0.04, z0 + s.stock + receiver_len * 0.5), Vec3::new(90.0, 0.0, 0.0), black, 0.5);
            }
            if s.extra == Extra::Launcher {
                add(PrimKind::Cone { radius: bore * 1.45, height: 0.17 }, Vec3::new(0.0, 0.05 + s.body_h * 0.6, z0 + s.length + 0.07), Vec3::new(90.0, 0.0, 0.0), Vec3::new(0.3, 0.3, 0.12), 0.3);
            }
            if s.extra == Extra::Drum {
                add(PrimKind::Cylinder { radius: s.body_h * 0.6, height: s.body_w * 1.5 }, Vec3::new(0.0, 0.05 + s.body_h * 0.5, z0 + s.stock + receiver_len * 0.5), Vec3::new(0.0, 0.0, 90.0), steel, 0.7);
            }
        }
    }
    crate::schema::Object {
        id: id.to_string(),
        position: Track::constant(Vec3::ZERO),
        rotation: Track::constant(Vec3::ZERO),
        scale: Track::constant(Vec3::ONE),
        material: None,
        collide: false,
        prefab: None,
        movable: Some(false),
        kind: ObjectKind::Group(kids),
    }
}

/// The model of a rocket or grenade in flight: a group with +Z as its heading.
pub fn projectile_model(weapon: Weapon, id: &str) -> crate::schema::Object {
    use crate::schema::{ObjectKind, PrimKind};
    use crate::track::Track;
    let mut kids = Vec::new();
    match weapon {
        Weapon::Lancer => {
            kids.push(part(format!("{id}_body"), PrimKind::Cylinder { radius: 0.045, height: 0.55 }, Vec3::ZERO, Vec3::new(90.0, 0.0, 0.0), Vec3::new(0.12, 0.14, 0.07), 0.3));
            kids.push(part(format!("{id}_nose"), PrimKind::Cone { radius: 0.06, height: 0.2 }, Vec3::new(0.0, 0.0, 0.36), Vec3::new(90.0, 0.0, 0.0), Vec3::new(0.3, 0.3, 0.12), 0.3));
            let mut flame = part(format!("{id}_flame"), PrimKind::Cone { radius: 0.05, height: 0.35 }, Vec3::new(0.0, 0.0, -0.42), Vec3::new(-90.0, 0.0, 0.0), Vec3::new(1.0, 0.6, 0.2), 0.0);
            if let Some(m) = flame.material.as_mut() {
                m.emissive = Vec3::new(4.0, 2.0, 0.5);
            }
            kids.push(flame);
        }
        Weapon::Thumper => {
            kids.push(part(format!("{id}_shell"), PrimKind::Cylinder { radius: 0.022, height: 0.1 }, Vec3::ZERO, Vec3::new(90.0, 0.0, 0.0), Vec3::new(0.3, 0.26, 0.1), 0.6));
            kids.push(part(format!("{id}_tip"), PrimKind::Sphere { radius: 0.022 }, Vec3::new(0.0, 0.0, 0.05), Vec3::ZERO, Vec3::new(0.2, 0.2, 0.2), 0.6));
        }
        Weapon::Frag => kids.push(part(format!("{id}_body"), PrimKind::Sphere { radius: 0.05 }, Vec3::ZERO, Vec3::ZERO, Vec3::new(0.06, 0.08, 0.03), 0.2)),
        Weapon::Flash => kids.push(part(format!("{id}_body"), PrimKind::Cylinder { radius: 0.04, height: 0.13 }, Vec3::ZERO, Vec3::new(90.0, 0.0, 0.0), Vec3::new(0.16, 0.17, 0.18), 0.3)),
        Weapon::Smoke => kids.push(part(format!("{id}_body"), PrimKind::Cylinder { radius: 0.04, height: 0.13 }, Vec3::ZERO, Vec3::new(90.0, 0.0, 0.0), Vec3::new(0.25, 0.26, 0.27), 0.3)),
        _ => kids.push(part(format!("{id}_body"), PrimKind::Cylinder { radius: 0.04, height: 0.13 }, Vec3::ZERO, Vec3::new(90.0, 0.0, 0.0), Vec3::new(0.3, 0.04, 0.03), 0.3)),
    }
    crate::schema::Object {
        id: id.to_string(),
        position: Track::constant(Vec3::ZERO),
        rotation: Track::constant(Vec3::ZERO),
        scale: Track::constant(Vec3::ONE),
        material: None,
        collide: false,
        prefab: None,
        movable: Some(false),
        kind: ObjectKind::Group(kids),
    }
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
    fn every_weapon_of_the_arsenal_has_a_model_and_a_hand_for_each_team() {
        for weapon in Weapon::ROSTER {
            if weapon == Weapon::Bat {
                continue;
            }
            let parts = if weapon.is_gun() { build_firearm_parts(weapon) } else { build_thrown_and_melee_parts(weapon) };
            assert!(parts.iter().all(|p| p.weapon == weapon));
            assert!(parts.iter().map(|p| p.mesh.vertices.len()).sum::<usize>() > 100, "{}", weapon.name());
            for skin in SKINS {
                assert_eq!(parts.iter().filter(|p| p.skin == skin && p.first_person_only).count(), 1, "{} has one sleeve for look {skin}", weapon.name());
                assert_eq!(parts.iter().filter(|p| p.skin == skin && !p.first_person_only).count(), 1, "{} has one hand for look {skin}", weapon.name());
            }
            if weapon.is_gun() {
                assert_eq!(parts.iter().filter(|p| p.muzzle_flash).count(), 1, "{}", weapon.name());
            }
        }
    }

    #[test]
    fn every_firearm_has_a_visible_model_and_one_flash() {
        for weapon in Weapon::FIREARMS {
            let parts = build_firearm_parts(weapon);
            assert!(parts.iter().all(|p| p.weapon == weapon));
            assert_eq!(parts.iter().filter(|p| p.muzzle_flash).count(), 1, "{}", weapon.name());
            assert!(parts.iter().map(|p| p.mesh.vertices.len()).sum::<usize>() > 100, "{}", weapon.name());
        }
    }
}
