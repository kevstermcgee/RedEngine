//! The characters' meshes as the renderer builds them (moved from `characters`' own tests when `characters` moved into the core: the mesh builder is
//! the renderer's, which needs `gfx`).
#![cfg(feature = "gfx")]

use glam::Vec3;
use red_engine2::characters::*;
use red_engine2::render::build_prim_mesh;
use red_engine2::schema::{ObjectKind, PrimKind};
use red_engine2::skeleton::{pose_to_parts, HumanoidRig, PoseSample};

/// A built-in colour (as `characters` reads its own).
fn hex(s: &str) -> Vec3 {
    red_engine2::color::parse_hex_to_linear(s).expect("built-in character colour is valid hex")
}

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
    use red_engine2::player::Character::*;
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
    use red_engine2::player::Character;
    let look = HumanLook::styled(Character::Boy);
    let boy = HumanoidRig::for_look(BOY_HEIGHT, 0.92, &look);
    let (lo, hi) = y_range(&human_parts(&boy, &PoseSample::default(), &look));
    assert!((-0.02..0.05).contains(&lo), "shoes rest on the floor: {lo}");
    assert!((BOY_HEIGHT * 0.90..BOY_HEIGHT * 1.05).contains(&hi), "a child of about 1.25 m with the hair on: {hi}");
    let adult = HumanoidRig::new(HUMAN_HEIGHT, 1.0);
    assert!(boy.head_radius / boy.height > 1.25 * adult.head_radius / adult.height, "a child's head is a bigger share of the height");
    assert!(boy.upper_leg_len + boy.lower_leg_len < 0.93 * (adult.upper_leg_len + adult.lower_leg_len) * BOY_HEIGHT / HUMAN_HEIGHT, "and the legs are shorter");
    let o = character_object(Character::Boy, "boy");
    match o.kind {
        ObjectKind::Humanoid(h) => assert_eq!((h.height, h.look.style), (BOY_HEIGHT, Character::Boy)),
        _ => panic!("the boy is a humanoid"),
    }
    assert_eq!(Character::parse("boy"), Some(Character::Boy));
    assert_eq!(red_engine2::net::protocol::character_from_wire(red_engine2::net::protocol::character_to_wire(Character::Boy)), Character::Boy);
    let b = Character::Boy.body();
    assert!(b.stand_eye < 1.3 && !b.has_bat && b.walk_speed < Character::Human.body().walk_speed);
}

#[test]
fn the_boy_has_no_pack_arms_that_stop_at_the_hip_and_a_mouth_below_his_nose() {
    use red_engine2::player::Character;
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
