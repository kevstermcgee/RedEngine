//! Character models: the ordinary-looking human and Cheddar the lab rat, each a list of coloured
//! primitive parts (capsules and squashed spheres) posed from a handful of joint/gait numbers.
//!
//! Both follow the engine's "schema keyword -> Rust-built parts" precedent (`humanoid`, `prop`,
//! `stairs`): the renderer builds one mesh per part once ([`CharPart::shape`] never depends on the
//! pose) and re-places them every frame from [`CharPart::local`]. Parts that must not follow the
//! object's own material (skin, hair, pink ears...) carry an explicit `color`; `None` means "use
//! the object's material colour" (a human's shirt, a rat's fur).
//!
//! Conventions shared with the rest of the engine: origin at the feet, +Y up, the character faces
//! local +Z. Add a new character here, add an `ObjectKind` arm, and `render.rs` needs nothing else.

use crate::color::parse_hex_to_linear;
use crate::schema::{HumanoidDef, Material, Object, ObjectKind, Pose, PrimKind, RatDef};
use crate::track::Track;
use crate::skeleton::{pose_to_parts, BonePart, HumanoidRig, PoseSample};
use glam::{Mat4, Quat, Vec3};

/// One coloured piece of a character, in the character's local space.
pub struct CharPart {
    /// Mesh shape; fixed for a given character (only `local` changes with the pose).
    pub shape: PrimKind,
    /// Placement (and squash/stretch) of the mesh relative to the character's origin.
    pub local: Mat4,
    /// Linear-RGB colour, or `None` to use the object's own material colour.
    pub color: Option<Vec3>,
    /// Material metalness (0 for everything organic here).
    pub metallic: f32,
    /// Material roughness.
    pub roughness: f32,
}

fn hex(s: &str) -> Vec3 {
    parse_hex_to_linear(s).expect("built-in character colour is valid hex")
}

fn ellipsoid(center: Vec3, radii: Vec3, rot: Quat, color: Option<Vec3>, roughness: f32) -> CharPart {
    CharPart {
        shape: PrimKind::Sphere { radius: 1.0 },
        local: Mat4::from_scale_rotation_translation(radii, rot, center),
        color,
        metallic: 0.0,
        roughness,
    }
}

/// A capsule whose axis runs `a -> b` (its mesh is `length` long in total, caps included).
fn limb(a: Vec3, b: Vec3, radius: f32, color: Option<Vec3>, roughness: f32) -> CharPart {
    let d = b - a;
    let len = d.length().max(1e-4);
    CharPart {
        shape: PrimKind::Capsule { radius, height: len },
        local: Mat4::from_rotation_translation(Quat::from_rotation_arc(Vec3::Y, d / len), (a + b) * 0.5),
        color,
        metallic: 0.0,
        roughness,
    }
}

// ---------------------------------------------------------------------------------------------
// Human
// ---------------------------------------------------------------------------------------------

/// The colours of a human that are not the shirt (which is the humanoid's `material.color`).
#[derive(Debug, Clone, Copy)]
pub struct HumanLook {
    /// Face, neck, forearms, hands, ears (linear RGB).
    pub skin: Vec3,
    /// Hair and eyebrows.
    pub hair: Vec3,
    /// Trousers.
    pub pants: Vec3,
    /// Shoes.
    pub shoes: Vec3,
}

impl Default for HumanLook {
    fn default() -> Self {
        HumanLook { skin: hex("#d9a684"), hair: hex("#3a281c"), pants: hex("#36445e"), shoes: hex("#2a2622") }
    }
}

fn bone_end(p: &BonePart, sign: f32) -> Vec3 {
    p.center + p.rotation * Vec3::Y * (p.length * 0.5 * sign)
}

/// Builds an ordinary-looking adult from the FK rig: T-shirt (the object's colour), bare
/// forearms and hands, jeans, shoes, a neck, hair, eyes, brows, nose, mouth and ears.
///
/// Parts `0..12` are exactly `pose_to_parts`' bones in the same order (torso, head, then each arm's
/// upper/fore, then each leg's thigh/shin/foot) — `re2` welds the third-person bat to part 3, the
/// left forearm — followed by the joint fillers and face details.
pub fn human_parts(rig: &HumanoidRig, pose: &PoseSample, look: &HumanLook) -> Vec<CharPart> {
    let core = pose_to_parts(rig, pose);
    let h = rig.hip_y / 0.53; // the object's nominal height, for size-relative details
    let r = rig.head_radius;
    let skin = Some(look.skin);
    let pants = Some(look.pants);
    let shoes = Some(look.shoes);
    let mut out: Vec<CharPart> = Vec::with_capacity(36);

    let at = |p: &BonePart, s: Vec3| Mat4::from_scale_rotation_translation(s, p.rotation, p.center);
    let capsule = |p: &BonePart, s: Vec3, color: Option<Vec3>, rough: f32| CharPart {
        shape: PrimKind::Capsule { radius: p.radius, height: p.length },
        local: at(p, s),
        color,
        metallic: 0.0,
        roughness: rough,
    };

    // 0 torso (broader than deep), 1 head (egg-shaped).
    out.push(capsule(&core[0], Vec3::new(1.40, 1.0, 0.80), None, 0.85));
    out.push(ellipsoid(core[1].center, Vec3::new(0.9 * r, 1.13 * r, r), core[1].rotation, skin, 0.6));
    // 2..6: arms — sleeve to the elbow, then bare forearm.
    out.push(capsule(&core[2], Vec3::ONE, skin, 0.6));
    out.push(capsule(&core[3], Vec3::ONE, skin, 0.6));
    out.push(capsule(&core[4], Vec3::ONE, skin, 0.6));
    out.push(capsule(&core[5], Vec3::ONE, skin, 0.6));
    // 6..12: legs — jeans, then shoes (wider and flatter than the shin).
    for leg in [6usize, 9] {
        out.push(capsule(&core[leg], Vec3::ONE, pants, 0.8));
        out.push(capsule(&core[leg + 1], Vec3::ONE, pants, 0.8));
        out.push(capsule(&core[leg + 2], Vec3::new(1.25, 1.0, 0.82), shoes, 0.5));
    }

    // Neck: from just under the shoulder line up into the head.
    let spine_up = core[0].rotation * Vec3::Y;
    let neck_a = bone_end(&core[0], 1.0) - spine_up * 0.03 * h;
    out.push(limb(neck_a, core[1].center - core[1].rotation * Vec3::Y * 0.2 * r, 0.03 * h, skin, 0.6));

    // Hair: a cap sitting up and back on the skull, leaving the face and forehead clear.
    let hr = core[1].rotation;
    let head = |p: Vec3| core[1].center + hr * (p * r);
    out.push(ellipsoid(head(Vec3::new(0.0, 0.14, -0.11)), Vec3::new(0.99 * r, 1.12 * r, 1.06 * r), hr, Some(look.hair), 0.9));
    // Face: eye whites + pupils, brows, nose, mouth, ears.
    for sx in [-1.0f32, 1.0] {
        out.push(ellipsoid(head(Vec3::new(0.38 * sx, 0.10, 0.88)), Vec3::new(0.15, 0.15, 0.09) * r, hr, Some(hex("#f1ede6")), 0.3));
        out.push(ellipsoid(head(Vec3::new(0.38 * sx, 0.10, 0.945)), Vec3::splat(0.075) * r, hr, Some(hex("#1c1410")), 0.2));
        out.push(ellipsoid(head(Vec3::new(0.38 * sx, 0.30, 0.86)), Vec3::new(0.22, 0.045, 0.07) * r, hr, Some(look.hair), 0.9));
        out.push(ellipsoid(head(Vec3::new(0.90 * sx, 0.0, -0.05)), Vec3::new(0.10, 0.20, 0.14) * r, hr, skin, 0.6));
    }
    out.push(ellipsoid(head(Vec3::new(0.0, -0.12, 0.92)), Vec3::new(0.11, 0.16, 0.14) * r, hr, skin, 0.6));
    out.push(ellipsoid(head(Vec3::new(0.0, -0.50, 0.86)), Vec3::new(0.26, 0.035, 0.05) * r, hr, Some(hex("#a4544c")), 0.5));

    // Joint fillers (a capsule's round end leaves a notch at every bend) and the hands.
    let ua = rig.upper_arm_radius;
    for (upper, fore) in [(2usize, 3usize), (4, 5)] {
        let (shoulder, elbow) = (bone_end(&core[upper], -1.0), bone_end(&core[upper], 1.0));
        out.push(ellipsoid(shoulder, Vec3::splat(ua * 1.16), Quat::IDENTITY, None, 0.85)); // shoulder (sleeve)
        out.push(limb(shoulder, shoulder + (elbow - shoulder) * 0.5, ua * 1.06, None, 0.85)); // short sleeve
        out.push(ellipsoid(elbow, Vec3::splat(ua * 0.86), Quat::IDENTITY, skin, 0.6)); // bare elbow
        let wrist = bone_end(&core[fore], 1.0);
        let dir = core[fore].rotation * Vec3::Y;
        out.push(ellipsoid(wrist + dir * 0.028 * h, Vec3::new(0.021, 0.034, 0.025) * h, core[fore].rotation, skin, 0.6));
    }
    for leg in [6usize, 9] {
        out.push(ellipsoid(bone_end(&core[leg], 1.0), Vec3::splat(rig.upper_leg_radius * 0.84), Quat::IDENTITY, pants, 0.8)); // knee
        out.push(ellipsoid(bone_end(&core[leg + 1], 1.0), Vec3::splat(rig.foot_radius * 0.98), Quat::IDENTITY, shoes, 0.5)); // ankle
    }
    // Pelvis (trousers) with the shirt hem hanging over its top edge.
    out.push(ellipsoid(Vec3::new(0.0, rig.hip_y - 0.012 * h, 0.0), Vec3::new(0.098, 0.058, 0.070) * h, Quat::IDENTITY, pants, 0.8));
    out.push(ellipsoid(Vec3::new(0.0, rig.hip_y + 0.040 * h, 0.0), Vec3::new(0.099, 0.052, 0.062) * h, Quat::IDENTITY, None, 0.85));
    out
}

// ---------------------------------------------------------------------------------------------
// Cheddar the rat
// ---------------------------------------------------------------------------------------------

/// Gait/pose numbers for [`rat_parts`].
#[derive(Debug, Clone, Copy, Default)]
pub struct RatPose {
    /// Gait phase in radians (advance it with distance travelled).
    pub gait: f32,
    /// Gait amplitude: 0 = standing still, 1 = flat-out scurry.
    pub stride: f32,
    /// Tail/head idle phase in radians (advance it with time).
    pub sway: f32,
}

/// Nose-to-rump length of the standard rat, metres (tail excluded). About a foot and a half of
/// rat including tail: small next to a 1.8 m human, big enough to read on screen.
pub const RAT_BODY_LENGTH: f32 = 0.43;
/// Height of the rat's back, metres.
pub const RAT_BACK_HEIGHT: f32 = 0.165;
/// Default fur colour: Cheddar's brownish-grey coat.
pub const RAT_FUR_HEX: &str = "#7b6a5d";

/// Builds Cheddar: a pear-shaped body, a pointed head with big round ears and whiskers, four
/// scurrying legs with pink paws, and a long tapering tail that trails and wags.
pub fn rat_parts(pose: &RatPose) -> Vec<CharPart> {
    let pink = Some(hex("#d9a2a0"));
    let tail_col = Some(hex("#ad8378"));
    let dark = Some(hex("#0b0a0c"));
    let (g, k) = (pose.gait, pose.stride.clamp(0.0, 1.0));
    let bob = 0.004 * k * (2.0 * g).sin();
    let sniff = 0.004 * (pose.sway * 3.0).sin() * (1.0 - k);
    let id = Quat::IDENTITY;
    let v = Vec3::new;
    let mut out: Vec<CharPart> = Vec::with_capacity(40);

    // Body: heavy hindquarters tapering to the shoulders, then a long-nosed head.
    let lift = 0.012; // body clearance: rats run on their toes
    out.push(ellipsoid(v(0.0, 0.088 + lift + bob, -0.078), v(0.066, 0.063, 0.092), id, None, 0.9));
    out.push(ellipsoid(v(0.0, 0.090 + lift + bob, 0.000), v(0.058, 0.056, 0.102), id, None, 0.9));
    out.push(ellipsoid(v(0.0, 0.090 + lift + bob, 0.078), v(0.050, 0.049, 0.075), id, None, 0.9));
    let head_y = 0.094 + lift + bob * 0.5 + sniff;
    out.push(ellipsoid(v(0.0, head_y, 0.158), v(0.036, 0.036, 0.056), id, None, 0.9));
    out.push(ellipsoid(v(0.0, head_y - 0.008, 0.220), v(0.020, 0.019, 0.046), id, None, 0.9));
    out.push(ellipsoid(v(0.0, head_y - 0.008, 0.262), v(0.011, 0.010, 0.010), id, pink, 0.4));
    // Eyes (with a glint) and ears (fur back, pink inside).
    for sx in [-1.0f32, 1.0] {
        out.push(ellipsoid(v(0.027 * sx, head_y + 0.014, 0.186), Vec3::splat(0.0085), id, dark, 0.15));
        out.push(ellipsoid(v(0.0262 * sx, head_y + 0.019, 0.1935), Vec3::splat(0.0028), id, Some(hex("#ffffff")), 0.1));
        let rot = Quat::from_rotation_y(-30f32.to_radians() * sx) * Quat::from_rotation_z(-15f32.to_radians() * sx);
        let c = v(0.034 * sx, head_y + 0.040, 0.130);
        out.push(ellipsoid(c, v(0.006, 0.027, 0.025), rot, None, 0.9));
        out.push(ellipsoid(c + rot * v(0.0035, 0.0, 0.0), v(0.0035, 0.021, 0.019), rot, pink, 0.5));
    }
    // Whiskers: three fine hairs a side fanning out and slightly forward.
    for sx in [-1.0f32, 1.0] {
        for (i, pitch) in [-12.0f32, 0.0, 12.0].into_iter().enumerate() {
            let yaw = (38.0 + 14.0 * i as f32).to_radians() * sx;
            let dir = v(yaw.sin() * pitch.to_radians().cos(), pitch.to_radians().sin(), yaw.cos() * pitch.to_radians().cos()).normalize();
            let origin = v(0.011 * sx, head_y - 0.008 + 0.005 * (i as f32 - 1.0), 0.242);
            out.push(limb(origin, origin + dir * 0.075, 0.0012, Some(hex("#d9d3c9")), 0.6));
        }
    }
    // Front legs (alternate diagonally with the hind legs: a scurrying trot) and pink paws.
    for (sx, phase) in [(-1.0f32, 0.0f32), (1.0, std::f32::consts::PI)] {
        let th = 0.55 * k * (g + phase).sin();
        let hip = v(0.042 * sx, 0.066 + lift + bob, 0.078);
        let foot = hip + v(0.0, -0.064 * th.cos(), 0.064 * th.sin());
        out.push(limb(hip, foot, 0.010, None, 0.9));
        out.push(ellipsoid(foot + v(0.0, 0.0, 0.009), v(0.011, 0.007, 0.020), id, pink, 0.5));
    }
    for (sx, phase) in [(-1.0f32, std::f32::consts::PI), (1.0, 0.0f32)] {
        let th = 0.6 * k * (g + phase).sin();
        out.push(ellipsoid(v(0.060 * sx, 0.068 + lift + bob, -0.088), v(0.027, 0.048, 0.054), id, None, 0.9)); // haunch
        let hip = v(0.058 * sx, 0.062 + lift + bob, -0.098);
        let foot = hip + v(0.0, -0.070 * th.cos(), 0.070 * th.sin());
        out.push(limb(hip, foot, 0.011, None, 0.9));
        out.push(ellipsoid(foot + v(0.0, 0.002, 0.014), v(0.011, 0.006, 0.028), id, pink, 0.5));
    }
    // Tail: six tapering segments trailing behind, drooping at rest, streaming when running, and
    // wagging in a travelling wave.
    let mut p = v(0.0, 0.086 + lift + bob, -0.160);
    let amp = (6.0 + 14.0 * k).to_radians();
    for i in 0..6 {
        let yaw = amp * (pose.sway * 1.6 - i as f32 * 0.9 + g * 0.5 * k).sin();
        let droop = ((8.0 + 3.0 * i as f32) * (1.0 - k) + (-3.0 + 2.0 * i as f32) * k).to_radians();
        let dir = v(yaw.sin() * droop.cos(), -droop.sin(), -yaw.cos() * droop.cos());
        let next = p + dir * 0.045;
        out.push(limb(p, next, 0.0075 - 0.0007 * i as f32, tail_col, 0.7));
        p = next;
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Ready-made scene objects (the player's body, the launch-screen models)
// ---------------------------------------------------------------------------------------------

/// Height of the standard human, m.
pub const HUMAN_HEIGHT: f32 = 1.8;
/// The player's default T-shirt colour.
pub const HUMAN_SHIRT_HEX: &str = "#4b7fb0";

fn object(id: &str, kind: ObjectKind) -> Object {
    Object {
        id: id.to_string(),
        position: Track::constant(Vec3::ZERO),
        rotation: Track::constant(Vec3::ZERO),
        scale: Track::constant(Vec3::ONE),
        material: None,
        collide: false,
        prefab: None,
        movable: None,
        kind,
    }
}

/// The standard human standing relaxed at the origin: a `humanoid` object in the default look with
/// a blue T-shirt. Its pose and transform are rewritten by whoever animates it.
pub fn human_object(id: &str) -> Object {
    let pose = Pose {
        spine: Track::constant(Vec3::ZERO),
        head: Track::constant(Vec3::ZERO),
        l_shoulder: Track::constant(Vec3::new(0.0, 0.0, -6.0)),
        r_shoulder: Track::constant(Vec3::new(0.0, 0.0, 6.0)),
        l_elbow: Track::constant(4.0),
        r_elbow: Track::constant(4.0),
        l_hip: Track::constant(Vec3::ZERO),
        r_hip: Track::constant(Vec3::ZERO),
        l_knee: Track::constant(4.0),
        r_knee: Track::constant(4.0),
    };
    let material = Material { color: Track::constant(hex(HUMAN_SHIRT_HEX)), metallic: 0.0, roughness: 0.85, emissive: Vec3::ZERO };
    object(id, ObjectKind::Humanoid(Box::new(HumanoidDef { height: HUMAN_HEIGHT, build: 1.0, material, look: HumanLook::default(), pose })))
}

/// Cheddar standing still at the origin, in his brownish-grey fur.
pub fn rat_object(id: &str) -> Object {
    let material = Material { color: Track::constant(hex(RAT_FUR_HEX)), metallic: 0.0, roughness: 0.9, emissive: Vec3::ZERO };
    object(
        id,
        ObjectKind::Rat(Box::new(RatDef { material, gait: Track::constant(0.0), stride: Track::constant(0.0), sway: Track::constant(0.0) })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::build_prim_mesh;

    /// Lowest and highest world Y of the actual mesh vertices.
    fn y_range(parts: &[CharPart]) -> (f32, f32) {
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for p in parts {
            for v in &build_prim_mesh(&p.shape).vertices {
                let y = p.local.transform_point3(Vec3::from_array(v.pos)).y;
                lo = lo.min(y);
                hi = hi.max(y);
            }
        }
        (lo, hi)
    }

    /// Mesh kind and size to a few millimetres: FK float noise must not count as a change.
    fn shapes(parts: &[CharPart]) -> Vec<String> {
        let q = |x: f32| (x * 200.0).round() as i32;
        parts
            .iter()
            .map(|p| match p.shape {
                PrimKind::Capsule { radius, height } => format!("capsule {} {}", q(radius), q(height)),
                PrimKind::Sphere { radius } => format!("sphere {}", q(radius)),
                other => format!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn human_keeps_the_rig_bones_first_and_shapes_are_pose_independent() {
        let rig = HumanoidRig::new(1.8, 1.0);
        let rest = human_parts(&rig, &PoseSample::default(), &HumanLook::default());
        assert!(rest.len() > 30);
        // Part 3 must stay the left forearm: `re2` welds the third-person bat to it.
        assert_eq!(pose_to_parts(&rig, &PoseSample::default()).len(), 12);
        let mut posed = PoseSample::default();
        posed.l_hip = Vec3::new(30.0, 0.0, 0.0);
        posed.l_knee = 40.0;
        posed.head = Vec3::new(10.0, 25.0, 0.0);
        posed.r_shoulder = Vec3::new(-60.0, 0.0, 6.0);
        posed.spine = Vec3::new(6.0, 0.0, 0.0);
        let moved = human_parts(&rig, &posed, &HumanLook::default());
        assert_eq!(shapes(&rest), shapes(&moved), "meshes are built once from the rest pose");
    }

    #[test]
    fn human_is_about_as_tall_as_asked_and_stands_on_the_floor() {
        let parts = human_parts(&HumanoidRig::new(1.8, 1.0), &PoseSample::default(), &HumanLook::default());
        let (lo, hi) = y_range(&parts);
        assert!((-0.02..0.05).contains(&lo), "shoes rest on the floor: {lo}");
        assert!((1.75..1.95).contains(&hi), "hair top near 1.8 m: {hi}");
    }

    #[test]
    fn rat_is_small_and_shapes_are_pose_independent() {
        let rest = rat_parts(&RatPose::default());
        let run = rat_parts(&RatPose { gait: 1.3, stride: 1.0, sway: 2.0 });
        assert_eq!(shapes(&rest), shapes(&run), "meshes are built once from the rest pose");
        let (lo, hi) = y_range(&rest);
        assert!((-0.02..0.03).contains(&lo), "paws on the floor: {lo}");
        assert!((0.12..0.25).contains(&hi), "the rat is knee-high to a chair: {hi}");
        for p in rest.iter().chain(run.iter()) {
            assert!(p.local.to_cols_array().iter().all(|x| x.is_finite()));
        }
    }

    #[test]
    fn the_rat_is_much_smaller_than_the_human() {
        let (_, rat_top) = y_range(&rat_parts(&RatPose::default()));
        let (_, human_top) = y_range(&human_parts(&HumanoidRig::new(HUMAN_HEIGHT, 1.0), &PoseSample::default(), &HumanLook::default()));
        assert!(rat_top * 6.0 < human_top, "rat {rat_top} vs human {human_top}");
    }
}
