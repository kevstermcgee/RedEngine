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
use crate::skeleton::{pose_to_parts, BonePart, HumanoidRig, PoseSample};
use crate::track::Track;
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
    CharPart { shape: PrimKind::Sphere { radius: 1.0 }, local: Mat4::from_scale_rotation_translation(radii, rot, center), color, metallic: 0.0, roughness }
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
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HumanLook {
    /// Costume on the shared human rig (Human means ordinary clothing).
    pub style: crate::player::Character,
    /// Face, neck, forearms, hands, ears (linear RGB).
    pub skin: Vec3,
    /// Hair and eyebrows.
    pub hair: Vec3,
    /// Trousers.
    pub pants: Vec3,
    /// Shoes.
    pub shoes: Vec3,
    /// Gloves, for a soldier (the hands and cuffs are these colours instead of skin); `None` = bare hands.
    pub glove: Option<Vec3>,
}

impl Default for HumanLook {
    fn default() -> Self {
        HumanLook {
            style: crate::player::Character::Human,
            skin: hex("#d9a684"),
            hair: hex("#3a281c"),
            pants: hex("#36445e"),
            shoes: hex("#2a2622"),
            glove: None,
        }
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
    if look.style == crate::player::Character::Boy {
        return boy_parts(rig, pose, look);
    }
    let core = pose_to_parts(rig, pose);
    let h = rig.height; // the object's nominal height, for size-relative details
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
    // 2..6: arms — sleeve to the elbow, then bare forearm. A soldier wears long sleeves (the object's jacket colour) and gloves.
    let soldier = look.glove.is_some();
    let arm = if soldier { None } else { skin };
    out.push(capsule(&core[2], Vec3::ONE, arm, if soldier { 0.85 } else { 0.6 }));
    out.push(capsule(&core[3], if soldier { Vec3::new(1.12, 1.0, 1.12) } else { Vec3::ONE }, arm, if soldier { 0.85 } else { 0.6 }));
    out.push(capsule(&core[4], Vec3::ONE, arm, if soldier { 0.85 } else { 0.6 }));
    out.push(capsule(&core[5], if soldier { Vec3::new(1.12, 1.0, 1.12) } else { Vec3::ONE }, arm, if soldier { 0.85 } else { 0.6 }));
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
        out.push(ellipsoid(elbow, Vec3::splat(ua * 0.86), Quat::IDENTITY, if soldier { None } else { skin }, if soldier { 0.85 } else { 0.6 })); // elbow
        let wrist = bone_end(&core[fore], 1.0);
        let dir = core[fore].rotation * Vec3::Y;
        let hand = look.glove.map(Some).unwrap_or(skin);
        out.push(ellipsoid(wrist + dir * 0.028 * h, Vec3::new(0.021, 0.034, 0.025) * h * if soldier { 1.12 } else { 1.0 }, core[fore].rotation, hand, 0.7));
    }
    for leg in [6usize, 9] {
        out.push(ellipsoid(bone_end(&core[leg], 1.0), Vec3::splat(rig.upper_leg_radius * 0.84), Quat::IDENTITY, pants, 0.8)); // knee
        out.push(ellipsoid(bone_end(&core[leg + 1], 1.0), Vec3::splat(rig.foot_radius * 0.98), Quat::IDENTITY, shoes, 0.5));
        // ankle
    }
    // Pelvis (trousers) with the shirt hem hanging over its top edge.
    out.push(ellipsoid(Vec3::new(0.0, rig.hip_y - 0.012 * h, 0.0), Vec3::new(0.098, 0.058, 0.070) * h, Quat::IDENTITY, pants, 0.8));
    out.push(ellipsoid(Vec3::new(0.0, rig.hip_y + 0.040 * h, 0.0), Vec3::new(0.099, 0.052, 0.062) * h, Quat::IDENTITY, None, 0.85));
    crate::costumes::decorate(&mut out, &core, h, look.style);
    out
}

/// The boy (Marcel): the same bones as [`human_parts`] in the same order (so everything that finds a bone by index still does), built as a picture-book child. A round
/// head with big friendly eyes, rosy cheeks and a soft smile; a jumper with long sleeves and chunky cuffs; round hands; sturdy trousers with turned-up cuffs; and big
/// round shoes. No pack, no bat.
fn boy_parts(rig: &HumanoidRig, pose: &PoseSample, look: &HumanLook) -> Vec<CharPart> {
    let core = pose_to_parts(rig, pose);
    let h = rig.height;
    let r = rig.head_radius;
    let skin = Some(look.skin);
    let pants = Some(look.pants);
    let shoes = Some(look.shoes);
    let mut out: Vec<CharPart> = Vec::with_capacity(64);
    let at = |p: &BonePart, s: Vec3| Mat4::from_scale_rotation_translation(s, p.rotation, p.center);
    let capsule = |p: &BonePart, s: Vec3, color: Option<Vec3>, rough: f32| CharPart {
        shape: PrimKind::Capsule { radius: p.radius, height: p.length },
        local: at(p, s),
        color,
        metallic: 0.0,
        roughness: rough,
    };

    // 0 torso (a soft barrel: broader than deep), 1 head (a round egg).
    out.push(capsule(&core[0], Vec3::new(1.22, 1.0, 0.88), None, 0.9));
    out.push(ellipsoid(core[1].center, Vec3::new(0.94 * r, 1.08 * r, r), core[1].rotation, skin, 0.6));
    // 2..6: arms, in jumper sleeves to the wrist (the object's colour), 6..12: legs in trousers, shoes (wide, rounded, a little flat).
    out.push(capsule(&core[2], Vec3::ONE, None, 0.9));
    out.push(capsule(&core[3], Vec3::ONE, None, 0.9));
    out.push(capsule(&core[4], Vec3::ONE, None, 0.9));
    out.push(capsule(&core[5], Vec3::ONE, None, 0.9));
    for leg in [6usize, 9] {
        out.push(capsule(&core[leg], Vec3::ONE, pants, 0.85));
        out.push(capsule(&core[leg + 1], Vec3::ONE, pants, 0.85));
        out.push(capsule(&core[leg + 2], Vec3::new(1.3, 1.0, 0.9), shoes, 0.55));
    }

    // Neck, short and thick.
    let spine_up = core[0].rotation * Vec3::Y;
    let neck_a = bone_end(&core[0], 1.0) - spine_up * 0.035 * h;
    out.push(limb(neck_a, core[1].center - core[1].rotation * Vec3::Y * 0.3 * r, 0.036 * h, skin, 0.6));

    // The head's own frame: unit coordinates times the radius; the face sits on the front of the egg.
    let hr = core[1].rotation;
    let head = |p: Vec3| core[1].center + hr * (p * r);
    let egg = Vec3::new(0.94, 1.08, 1.0);
    // A point on the surface of the face at (x, y) in head units, pushed out by `lift`.
    let face = |x: f32, y: f32, lift: f32| {
        let z = (1.0 - (x / egg.x).powi(2) - (y / egg.y).powi(2)).max(0.05).sqrt() * egg.z;
        head(Vec3::new(x, y, z + lift))
    };
    // Hair: a thick cap that comes well down over the brow, then a few tufts of different lengths on the crown, each leaning its own way.
    out.push(ellipsoid(head(Vec3::new(0.0, 0.20, -0.08)), Vec3::new(1.02 * r, 1.04 * r, 1.08 * r), hr, Some(look.hair), 0.9));
    for (x, y, z, rx, ry, rz, lean) in [
        (0.18f32, 1.14f32, 0.10f32, 0.22f32, 0.12f32, 0.17f32, -32.0f32),
        (-0.18, 1.13, 0.06, 0.23, 0.12, 0.17, 30.0),
        (0.02, 1.16, -0.18, 0.18, 0.13, 0.20, 8.0),
    ] {
        out.push(ellipsoid(head(Vec3::new(x, y, z)), Vec3::new(rx, ry, rz) * r, hr * Quat::from_rotation_z(lean.to_radians()), Some(look.hair), 0.9));
    }
    // Eyes: big, dark and wide apart, each with a small bright glint; brows as gentle arches.
    for sx in [-1.0f32, 1.0] {
        out.push(ellipsoid(face(0.36 * sx, 0.00, 0.01), Vec3::new(0.15, 0.185, 0.08) * r, hr, Some(hex("#f4f0ea")), 0.3));
        out.push(ellipsoid(face(0.36 * sx, -0.01, 0.06), Vec3::new(0.105, 0.135, 0.06) * r, hr, Some(hex("#2c1d14")), 0.2));
        out.push(ellipsoid(face(0.36 * sx + 0.035, 0.05, 0.095), Vec3::splat(0.036) * r, hr, Some(hex("#ffffff")), 0.1));
        out.push(ellipsoid(face(0.38 * sx, 0.27, 0.0), Vec3::new(0.20, 0.04, 0.06) * r, hr, Some(look.hair), 0.9));
        // Rosy cheeks, ears.
        out.push(ellipsoid(face(0.56 * sx, -0.30, 0.0), Vec3::new(0.15, 0.10, 0.05) * r, hr, Some(hex("#eb9786")), 0.8));
        out.push(ellipsoid(head(Vec3::new(0.94 * sx, -0.05, -0.05)), Vec3::new(0.10, 0.19, 0.14) * r, hr, skin, 0.6));
    }
    // A button nose.
    out.push(ellipsoid(face(0.0, -0.20, 0.07), Vec3::new(0.10, 0.09, 0.09) * r, hr, skin, 0.6));
    // A soft smile: a flat dark-red oval with a flat skin-coloured one laid over its upper part, which leaves a crescent whose corners curl up. Each plate is turned to lie
    // along the face where it sits, so the skin plate takes exactly the shading of the cheek around it and no lip shows.
    let decal = |x: f32, y: f32, lift: f32, w: f32, tall: f32, color: Vec3, rough: f32| {
        let z = (1.0 - (x / egg.x).powi(2) - (y / egg.y).powi(2)).max(0.05).sqrt() * egg.z;
        let normal = Vec3::new(x / (egg.x * egg.x), y / (egg.y * egg.y), z / (egg.z * egg.z)).normalize();
        let turn = hr * Quat::from_rotation_arc(Vec3::Z, normal);
        ellipsoid(head(Vec3::new(x, y, z) + normal * lift), Vec3::new(w, tall, 0.012) * r, turn, Some(color), rough)
    };
    out.push(decal(0.0, -0.45, 0.004, 0.30, 0.13, hex("#a4443f"), 0.5));
    out.push(decal(0.0, -0.29, 0.010, 0.36, 0.17, look.skin, 0.6));

    // Joint fillers, cuffs and hands: round and a little oversized, as a child's are.
    let ua = rig.upper_arm_radius;
    for (upper, fore) in [(2usize, 3usize), (4, 5)] {
        let (shoulder, elbow) = (bone_end(&core[upper], -1.0), bone_end(&core[upper], 1.0));
        out.push(ellipsoid(shoulder, Vec3::splat(ua), Quat::IDENTITY, None, 0.9));
        out.push(ellipsoid(elbow, Vec3::splat(rig.forearm_radius * 1.02), Quat::IDENTITY, None, 0.9));
        let wrist = bone_end(&core[fore], 1.0);
        let dir = core[fore].rotation * Vec3::Y;
        // The cuff: a slightly darker ribbed band, then the hand.
        out.push(limb(wrist - dir * 0.020 * h, wrist + dir * 0.004 * h, rig.forearm_radius * 1.10, Some(hex("#c2452c")), 0.95));
        out.push(ellipsoid(wrist + dir * 0.034 * h, Vec3::new(0.036, 0.044, 0.032) * h, core[fore].rotation, skin, 0.7));
    }
    for leg in [6usize, 9] {
        out.push(ellipsoid(bone_end(&core[leg], 1.0), Vec3::splat(rig.lower_leg_radius * 1.02), Quat::IDENTITY, pants, 0.85)); // knee
                                                                                                                               // The turned-up trouser cuff at the ankle, a little wider than the shin, and the round of the ankle above the shoe.
        let ankle = bone_end(&core[leg + 1], 1.0);
        let up = -(core[leg + 1].rotation * Vec3::Y); // the shin's axis runs knee to ankle, so up the leg is the other way
        out.push(limb(ankle + up * 0.045 * h, ankle - up * 0.006 * h, rig.lower_leg_radius * 1.10, Some(look.pants * 1.2), 0.9));
        out.push(ellipsoid(ankle, Vec3::splat(rig.foot_radius * 1.02), Quat::IDENTITY, shoes, 0.55));
    }
    // Seat of the trousers, and the jumper's hem hanging over it.
    out.push(ellipsoid(Vec3::new(0.0, rig.hip_y - 0.010 * h, 0.0), Vec3::new(0.104, 0.060, 0.078) * h, Quat::IDENTITY, pants, 0.85));
    out.push(ellipsoid(Vec3::new(0.0, rig.hip_y + 0.040 * h, 0.0), Vec3::new(0.108, 0.056, 0.074) * h, Quat::IDENTITY, None, 0.9));
    crate::costumes::decorate(&mut out, &core, h, look.style);
    out
}

/// Builds a playable or showcase character, retaining the human rig's stable bone indices.
pub fn character_object(who: crate::player::Character, id: &str) -> Object {
    use crate::player::Character;
    if who == Character::Rat {
        return rat_object(id);
    }
    let mut object = human_object(id);
    if let ObjectKind::Humanoid(h) = &mut object.kind {
        h.look = HumanLook::styled(who);
        let shirt = match who {
            Character::Wizard => "#443b9c",
            Character::Cowboy => "#995a35",
            Character::Alien => "#efad38",
            Character::Robot => "#3c9fba",
            c if c.soldier_team() == Some(1) => "#555e2b",
            c if c.soldier_team() == Some(2) => "#1f2d4d",
            // Matches the skin, not a costume accent: one wrong-colored shape, not a person wearing black.
            Character::Hollow => "#0d0c10",
            // A warm, cheerful red-orange jumper: the one saturated thing in a green and gold world.
            Character::Boy => "#e0573a",
            _ => "#af303c",
        };
        h.material.color = Track::constant(hex(shirt));
        if who == Character::Boy {
            h.height = BOY_HEIGHT;
            h.build = 0.92;
        }
        if who == Character::Hollow {
            // Unnaturally tall and gaunt — the cheapest, most reliable "this is wrong" silhouette cue.
            h.height = HUMAN_HEIGHT * 1.28;
            h.build = 0.82;
        }
    }
    object
}

impl HumanLook {
    /// Costume palette; scene authors can still override individual colours.
    pub fn styled(style: crate::player::Character) -> Self {
        use crate::player::Character;
        let mut look = Self { style, ..Self::default() };
        if let Some(u) = crate::uniforms::for_team(style.soldier_team().unwrap_or(0)) {
            look.skin = u.skin;
            look.hair = hex("#2a1e16");
            look.pants = u.trousers;
            look.shoes = u.boots;
            look.glove = Some(u.glove);
        }
        match style {
            Character::Wizard => {
                look.hair = hex("#ddd9d2");
                look.pants = hex("#30294e");
            }
            Character::Cowboy => {
                look.pants = hex("#334e75");
                look.shoes = hex("#613622");
            }
            Character::Alien => {
                look.skin = hex("#70c99c");
                look.hair = look.skin;
                look.pants = hex("#454278");
            }
            Character::Robot => {
                look.skin = hex("#9ab8c5");
                look.hair = look.skin;
                look.pants = hex("#344f63");
                look.shoes = hex("#243c4f");
            }
            Character::Boy => {
                look.skin = hex("#e8b48f");
                look.hair = hex("#6a4426");
                look.pants = hex("#51648a");
                look.shoes = hex("#7b5233");
            }
            Character::Hollow => {
                // Near-black, barely differentiated from itself: no warm tones anywhere, nothing reads as "wearing" clothes.
                look.skin = hex("#0d0c10");
                look.hair = look.skin;
                look.pants = hex("#0a090c");
                look.shoes = look.pants;
            }
            _ => {}
        }
        look
    }
}

// ---------------------------------------------------------------------------------------------
// Cheddar the rat
// ---------------------------------------------------------------------------------------------

/// Gait/pose numbers for [`rat_parts`].
#[derive(Debug, Clone, Copy, Default, PartialEq)]
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
/// The boy's height in metres: about nine years old.
pub const BOY_HEIGHT: f32 = 1.35;
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
    let material = Material { color: Track::constant(hex(HUMAN_SHIRT_HEX)), metallic: 0.0, roughness: 0.85, emissive: Vec3::ZERO, opacity: 1.0 };
    object(id, ObjectKind::Humanoid(Box::new(HumanoidDef { height: HUMAN_HEIGHT, build: 1.0, material, look: HumanLook::default(), pose })))
}

/// Cheddar standing still at the origin, in his brownish-grey fur.
pub fn rat_object(id: &str) -> Object {
    let material = Material { color: Track::constant(hex(RAT_FUR_HEX)), metallic: 0.0, roughness: 0.9, emissive: Vec3::ZERO, opacity: 1.0 };
    object(id, ObjectKind::Rat(Box::new(RatDef { material, gait: Track::constant(0.0), stride: Track::constant(0.0), sway: Track::constant(0.0) })))
}

#[cfg(all(test, feature = "gfx"))]
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

    /// Parts `0..12` of every style on the human rig are the rig's bones, in `pose_to_parts`' order (torso, head, each arm's upper and fore, each leg's thigh, shin and foot):
    /// `re2` welds the third-person bat to part 3 and the animation code finds a limb by its index, so a style that adds, drops or reorders a part breaks both silently.
    /// Every bone's part sits on the bone's centre with its shape (the head a sphere, the rest capsules of the bone's own radius and length).
    #[test]
    fn every_style_starts_with_the_rigs_twelve_bones_in_order() {
        use crate::player::Character::*;
        let rig = HumanoidRig::new(1.8, 1.0);
        let mut pose = PoseSample::default();
        pose.l_hip = Vec3::new(30.0, 0.0, 0.0);
        pose.r_knee = 40.0;
        pose.l_shoulder = Vec3::new(-50.0, 0.0, 8.0);
        pose.head = Vec3::new(10.0, 25.0, 0.0);
        let bones = pose_to_parts(&rig, &pose);
        assert_eq!(bones.len(), 12, "the rig has twelve bones");
        for style in [Human, Wizard, Cowboy, Alien, Robot, Ridgeback, Boy] {
            let look = HumanLook { style, ..HumanLook::default() };
            let parts = human_parts(&rig, &pose, &look);
            assert!(parts.len() > 12, "{style:?}: details follow the bones");
            for (i, (part, bone)) in parts.iter().zip(&bones).enumerate() {
                let (_, _, at) = part.local.to_scale_rotation_translation();
                assert!(at.distance(bone.center) < 1e-4, "{style:?} part {i} must sit on bone {i}'s centre: {at} vs {}", bone.center);
                match (&part.shape, i) {
                    (PrimKind::Sphere { .. }, 1) => {}
                    (PrimKind::Capsule { radius, height }, _) if i != 1 => {
                        assert!((radius - bone.radius).abs() < 1e-5 && (height - bone.length).abs() < 1e-5, "{style:?} part {i} is not bone {i}'s capsule");
                    }
                    (shape, _) => panic!("{style:?} part {i} has shape {shape:?}: bone 1 is the head sphere, the other eleven are capsules"),
                }
            }
        }
    }

    #[test]
    fn human_is_about_as_tall_as_asked_and_stands_on_the_floor() {
        let parts = human_parts(&HumanoidRig::new(1.8, 1.0), &PoseSample::default(), &HumanLook::default());
        let (lo, hi) = y_range(&parts);
        assert!((-0.02..0.05).contains(&lo), "shoes rest on the floor: {lo}");
        assert!((1.75..1.95).contains(&hi), "hair top near 1.8 m: {hi}");
    }

    #[test]
    fn the_boy_is_a_child_with_a_big_head_standing_on_the_floor() {
        use crate::player::Character;
        let look = HumanLook::styled(Character::Boy);
        let boy = HumanoidRig::for_look(BOY_HEIGHT, 0.92, &look);
        let (lo, hi) = y_range(&human_parts(&boy, &PoseSample::default(), &look));
        assert!((-0.02..0.05).contains(&lo), "shoes rest on the floor: {lo}");
        assert!((BOY_HEIGHT * 0.90..BOY_HEIGHT * 1.05).contains(&hi), "a child of about 1.25 m with the hair on: {hi}");
        let adult = HumanoidRig::new(HUMAN_HEIGHT, 1.0);
        assert!(boy.head_radius / boy.height > 1.25 * adult.head_radius / adult.height, "a child's head is a bigger share of the height");
        assert!(
            boy.upper_leg_len + boy.lower_leg_len < 0.93 * (adult.upper_leg_len + adult.lower_leg_len) * BOY_HEIGHT / HUMAN_HEIGHT,
            "and the legs are shorter"
        );
        let o = character_object(Character::Boy, "boy");
        match o.kind {
            ObjectKind::Humanoid(h) => assert_eq!((h.height, h.look.style), (BOY_HEIGHT, Character::Boy)),
            _ => panic!("the boy is a humanoid"),
        }
        assert_eq!(Character::parse("boy"), Some(Character::Boy));
        assert_eq!(crate::net::protocol::character_from_wire(crate::net::protocol::character_to_wire(Character::Boy)), Character::Boy);
        let b = Character::Boy.body();
        assert!(b.stand_eye < 1.3 && !b.has_bat && b.walk_speed < Character::Human.body().walk_speed);
    }

    #[test]
    fn the_boy_has_no_pack_arms_that_stop_at_the_hip_and_a_mouth_below_his_nose() {
        use crate::player::Character;
        let look = HumanLook::styled(Character::Boy);
        let rig = HumanoidRig::for_look(BOY_HEIGHT, 0.92, &look);
        let parts = human_parts(&rig, &PoseSample::default(), &look);
        // Nothing sticks out behind the back further than the body is deep: no book bag, no straps.
        let back = parts.iter().map(|p| p.local.transform_point3(Vec3::ZERO).z).fold(f32::MAX, f32::min);
        assert!(back > -0.16, "the furthest part behind the origin is at {back} m: a pack would be well past 0.17");
        // Arms and legs make sense together: the arm, shoulder to the middle of the hand, is about three quarters as long as the leg from hip to floor, so the hands
        // hang about at the hip, as a child's do.
        let arm = rig.upper_arm_len + rig.forearm_len + 0.034 * rig.height;
        let leg = rig.upper_leg_len + rig.lower_leg_len + rig.foot_radius;
        assert!((0.62..0.88).contains(&(arm / leg)), "arm {arm} m, leg {leg} m");
        // The smile: a red crescent in the lower face, under the nose and above the chin.
        let red = hex("#a4443f");
        let mouth = parts.iter().find(|p| p.color == Some(red)).expect("the boy has a mouth").local.transform_point3(Vec3::ZERO);
        let head_c = parts[1].local.transform_point3(Vec3::ZERO);
        assert!(mouth.y < head_c.y - 0.03 && mouth.y > head_c.y - 0.15, "mouth {} head {}", mouth.y, head_c.y);
        assert!(mouth.z > head_c.z + 0.06, "on the front of the face");
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
