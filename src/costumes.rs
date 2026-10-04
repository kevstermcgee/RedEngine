//! Joint-attached costume geometry for human-rig characters; shared by live/offline rendering and ray tests.
use crate::{characters::CharPart, player::Character, schema::PrimKind, skeleton::BonePart};
use glam::{Mat4, Quat, Vec3};

/// Append accessories without changing the rig's first twelve bone parts.
pub fn decorate(out: &mut Vec<CharPart>, bones: &[BonePart], height: f32, style: Character) {
    let mut add = |bone: usize, shape: PrimKind, offset: Vec3, scale: Vec3, color: &str, metallic: f32| {
        let anchor = &bones[bone];
        out.push(CharPart {
            shape,
            local: Mat4::from_rotation_translation(anchor.rotation, anchor.center)
                * Mat4::from_scale_rotation_translation(scale * height, Quat::IDENTITY, offset * height),
            color: Some(crate::color::parse_hex_to_linear(color).unwrap()),
            metallic,
            roughness: 0.55,
        });
    };
    let sphere = PrimKind::Sphere { radius: 1.0 };
    let box_shape = PrimKind::Box { size: Vec3::ONE };
    if let Some(u) = crate::uniforms::for_team(match style {
        Character::Ridgeback => 1,
        Character::Nightfall => 2,
        _ => 0,
    }) {
        soldier_gear(out, bones, height, &u);
        return;
    }
    match style {
        Character::Wizard => {
            add(1, PrimKind::Cylinder { radius: 0.13, height: 0.018 }, Vec3::new(0.0, 0.055, 0.0), Vec3::ONE, "#43397c", 0.0);
            add(1, PrimKind::Cone { radius: 0.095, height: 0.28 }, Vec3::new(0.0, 0.20, 0.0), Vec3::ONE, "#6454b0", 0.0);
            add(1, sphere, Vec3::new(0.0, 0.125, 0.06), Vec3::splat(0.018), "#f8d56b", 0.5);
            add(1, sphere, Vec3::new(0.0, -0.072, 0.075), Vec3::new(0.047, 0.092, 0.035), "#e4e0d7", 0.0);
            add(0, PrimKind::Cone { radius: 0.15, height: 0.40 }, Vec3::new(0.0, -0.15, 0.0), Vec3::new(1.0, 1.0, 0.7), "#443b9c", 0.0);
            add(0, sphere, Vec3::new(0.0, 0.07, 0.074), Vec3::splat(0.018), "#f8d56b", 0.5);
        }
        Character::Cowboy => {
            add(1, PrimKind::Cylinder { radius: 0.15, height: 0.018 }, Vec3::new(0.0, 0.06, 0.0), Vec3::new(1.0, 1.0, 0.8), "#bb8a53", 0.0);
            add(1, PrimKind::Cylinder { radius: 0.08, height: 0.085 }, Vec3::new(0.0, 0.10, 0.0), Vec3::new(1.0, 1.0, 0.85), "#c8955a", 0.0);
            add(1, PrimKind::Cylinder { radius: 0.083, height: 0.016 }, Vec3::new(0.0, 0.068, 0.0), Vec3::new(1.0, 1.0, 0.85), "#4a3025", 0.0);
            add(0, sphere, Vec3::new(0.0, 0.13, 0.058), Vec3::new(0.065, 0.045, 0.037), "#b72f3c", 0.0);
            for side in [-1.0, 1.0] {
                add(0, box_shape, Vec3::new(side * 0.06, 0.0, 0.065), Vec3::new(0.065, 0.25, 0.025), "#513c2d", 0.0);
            }
            add(0, box_shape, Vec3::new(0.0, -0.15, 0.065), Vec3::new(0.037, 0.033, 0.014), "#d5b668", 0.65);
        }
        Character::Alien => {
            add(1, sphere, Vec3::new(0.0, 0.018, 0.0), Vec3::new(0.09, 0.105, 0.08), "#70c99c", 0.0);
            for side in [-1.0, 1.0] {
                add(1, sphere, Vec3::new(side * 0.044, 0.026, 0.07), Vec3::new(0.032, 0.047, 0.018), "#172e39", 0.3);
                add(1, PrimKind::Cylinder { radius: 0.007, height: 0.10 }, Vec3::new(side * 0.05, 0.14, 0.0), Vec3::ONE, "#70c99c", 0.0);
                add(1, sphere, Vec3::new(side * 0.05, 0.195, 0.0), Vec3::splat(0.019), "#b6f4bd", 0.0);
            }
            add(0, box_shape, Vec3::new(0.0, 0.055, 0.068), Vec3::new(0.08, 0.07, 0.035), "#795eab", 0.4);
        }
        Character::Boy => {
            // A scarf round the neck, one end hanging down the front.
            add(0, sphere, Vec3::new(0.0, 0.150, 0.0), Vec3::new(0.098, 0.030, 0.088), "#f0b93f", 0.0);
            add(0, box_shape, Vec3::new(0.05, 0.075, 0.082), Vec3::new(0.04, 0.15, 0.014), "#e39a2c", 0.0);
            add(0, box_shape, Vec3::new(0.05, 0.0, 0.083), Vec3::new(0.04, 0.012, 0.016), "#c4571f", 0.0);
        }
        Character::Hollow => {
            let black = "#08070a";
            // A larger, elongated, off-axis head shape fully encloses (and so hides) the rig's built-in
            // face — no eyes, brows, nose or mouth should read through it. Restraint over accessories:
            // the body itself is the costume, nothing is "worn".
            add(1, sphere, Vec3::new(0.012, 0.03, -0.01), Vec3::new(0.125, 0.165, 0.118), black, 0.0);
            // Two small, pale, deep-set points — the only light the head gives back.
            for side in [-1.0f32, 1.0] {
                add(1, sphere, Vec3::new(side * 0.034, 0.02, 0.085), Vec3::splat(0.009), "#e8e4da", 0.1);
            }
            // Claw-like fingers: thin cones well past where the ordinary hand ellipsoid sits.
            for fore in [3usize, 5] {
                add(fore, PrimKind::Cone { radius: 1.0, height: 1.0 }, Vec3::new(0.0, 0.205, 0.0), Vec3::new(0.014, 0.09, 0.014), black, 0.0);
            }
        }
        Character::Robot => {
            add(1, box_shape, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.16, 0.15, 0.15), "#a2bbc5", 0.65);
            add(1, box_shape, Vec3::new(0.0, 0.014, 0.079), Vec3::new(0.125, 0.035, 0.015), "#172e47", 0.4);
            for side in [-1.0, 1.0] {
                add(1, sphere, Vec3::new(side * 0.034, 0.016, 0.09), Vec3::splat(0.013), "#8ee5ed", 0.4);
            }
            add(1, PrimKind::Cylinder { radius: 0.007, height: 0.10 }, Vec3::new(0.0, 0.12, 0.0), Vec3::ONE, "#687f99", 0.65);
            add(1, sphere, Vec3::new(0.0, 0.175, 0.0), Vec3::splat(0.017), "#e9a756", 0.4);
            add(0, box_shape, Vec3::new(0.0, 0.02, 0.074), Vec3::new(0.12, 0.13, 0.024), "#263d53", 0.6);
            add(0, sphere, Vec3::new(0.0, 0.045, 0.09), Vec3::splat(0.025), "#83dce3", 0.5);
        }
        _ => {}
    }
}

/// A soldier's kit on top of the human rig: a combat helmet (with chin strap, night-vision mount and a band in the team's accent colour), a plate
/// carrier front and back with magazine pouches, shoulder straps, a belt, a small pack, shoulder patches and knee pads. All in the team's colours
/// (`uniforms`), so the body, the first-person sleeves and the lobby agree. Sizes are in metres (the rig is 1.8 m tall); offsets are in each bone's frame.
fn soldier_gear(out: &mut Vec<CharPart>, bones: &[BonePart], height: f32, u: &crate::uniforms::Uniform) {
    let mut add = |bone: usize, shape: PrimKind, offset_m: Vec3, scale_m: Vec3, color: Vec3, metallic: f32, roughness: f32| {
        let anchor = &bones[bone];
        out.push(CharPart {
            shape,
            local: Mat4::from_rotation_translation(anchor.rotation, anchor.center) * Mat4::from_scale_rotation_translation(scale_m, Quat::IDENTITY, offset_m),
            color: Some(color),
            metallic,
            roughness,
        });
    };
    let _ = height;
    let sphere = PrimKind::Sphere { radius: 1.0 };
    let cube = PrimKind::Box { size: Vec3::ONE };
    let black = Vec3::new(0.02, 0.022, 0.025);
    let dark = u.vest * 0.55;
    // The helmet: a dome sitting high on the head (the eyes stay clear), a rim band, a front mount, chin straps.
    add(1, sphere, Vec3::new(0.0, 0.105, -0.006), Vec3::new(0.136, 0.112, 0.142), u.helmet, 0.15, 0.6);
    add(1, PrimKind::Cylinder { radius: 0.5, height: 1.0 }, Vec3::new(0.0, 0.012, -0.006), Vec3::new(0.272, 0.014, 0.284), u.helmet * 0.7, 0.1, 0.7);
    add(1, cube, Vec3::new(0.0, 0.058, 0.128), Vec3::new(0.15, 0.012, 0.014), u.accent, 0.2, 0.6);
    add(1, cube, Vec3::new(0.0, 0.165, 0.108), Vec3::new(0.05, 0.04, 0.035), black, 0.3, 0.5);
    for side in [-1.0f32, 1.0] {
        add(1, cube, Vec3::new(side * 0.098, -0.05, 0.03), Vec3::new(0.012, 0.11, 0.012), black, 0.0, 0.9);
        add(1, cube, Vec3::new(side * 0.142, 0.07, 0.0), Vec3::new(0.02, 0.05, 0.12), dark, 0.2, 0.6);
    }
    // The plate carrier, front and back, and its straps over the shoulders.
    add(0, cube, Vec3::new(0.0, 0.07, 0.112), Vec3::new(0.31, 0.27, 0.05), u.vest, 0.1, 0.85);
    add(0, cube, Vec3::new(0.0, 0.07, -0.112), Vec3::new(0.31, 0.27, 0.05), u.vest, 0.1, 0.85);
    for side in [-1.0f32, 1.0] {
        add(0, cube, Vec3::new(side * 0.115, 0.255, 0.0), Vec3::new(0.065, 0.028, 0.24), u.vest, 0.1, 0.85);
    }
    for x in [-0.09f32, 0.0, 0.09] {
        add(0, cube, Vec3::new(x, -0.005, 0.15), Vec3::new(0.07, 0.11, 0.04), dark, 0.1, 0.9);
    }
    add(0, cube, Vec3::new(0.0, -0.225, 0.0), Vec3::new(0.40, 0.06, 0.26), black, 0.1, 0.85);
    add(0, cube, Vec3::new(0.0, 0.05, -0.19), Vec3::new(0.26, 0.30, 0.11), u.jacket * 0.7, 0.05, 0.9);
    // Shoulder patches (the team's accent) and knee pads.
    for bone in [2usize, 4] {
        let side = if bone == 2 { 1.0 } else { -1.0 };
        add(bone, cube, Vec3::new(side * 0.045, 0.055, 0.0), Vec3::new(0.012, 0.07, 0.06), u.accent, 0.0, 0.8);
    }
    for bone in [7usize, 10] {
        add(bone, cube, Vec3::new(0.0, 0.14, 0.05), Vec3::new(0.10, 0.11, 0.045), black, 0.05, 0.9);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_costume_keeps_a_stable_rig_and_tracks_the_head_pose() {
        let rig = crate::skeleton::HumanoidRig::new(1.8, 1.0);
        for style in [
            Character::Wizard,
            Character::Cowboy,
            Character::Alien,
            Character::Robot,
            Character::Ridgeback,
            Character::Nightfall,
            Character::Hollow,
            Character::Boy,
        ] {
            let look = crate::characters::HumanLook::styled(style);
            let rest = crate::characters::human_parts(&rig, &Default::default(), &look);
            let posed = crate::characters::human_parts(&rig, &crate::skeleton::PoseSample { head: Vec3::new(0.0, 35.0, 0.0), ..Default::default() }, &look);
            assert_eq!(rest.len(), posed.len());
            assert!(rest.len() > crate::characters::human_parts(&rig, &Default::default(), &Default::default()).len());
            assert!(rest.iter().all(|part| part.local.is_finite()));
            assert_ne!(rest[1].local, posed[1].local);
            assert_eq!(rest[3].local, posed[3].local, "weapon attachment bone must stay stable");
        }
    }
}
