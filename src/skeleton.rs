use glam::{Quat, Vec3};

/// Fixed bone lengths/radii derived once from a humanoid's `height`/`build`. Pose only changes
/// joint *rotations*, never these lengths, so this is computed once per object, not per frame.
pub struct HumanoidRig {
    pub hip_y: f32,
    pub torso_len: f32,
    pub torso_radius: f32,
    pub neck_len: f32,
    pub head_radius: f32,
    pub shoulder_half: f32,
    pub hip_half: f32,
    pub upper_arm_len: f32,
    pub upper_arm_radius: f32,
    pub forearm_len: f32,
    pub forearm_radius: f32,
    pub upper_leg_len: f32,
    pub upper_leg_radius: f32,
    pub lower_leg_len: f32,
    pub lower_leg_radius: f32,
    pub foot_len: f32,
    pub foot_radius: f32,
}

impl HumanoidRig {
    pub fn new(height: f32, build: f32) -> Self {
        let h = height;
        HumanoidRig {
            hip_y: 0.50 * h,
            torso_len: 0.30 * h,
            torso_radius: 0.09 * h * build,
            neck_len: -0.02 * h,
            head_radius: 0.075 * h * build.sqrt(),
            shoulder_half: 0.10 * h,
            hip_half: 0.075 * h,
            upper_arm_len: 0.19 * h,
            upper_arm_radius: 0.045 * h * build,
            forearm_len: 0.16 * h,
            forearm_radius: 0.038 * h * build,
            upper_leg_len: 0.27 * h,
            upper_leg_radius: 0.062 * h * build,
            lower_leg_len: 0.23 * h,
            lower_leg_radius: 0.05 * h * build,
            foot_len: 0.12 * h,
            foot_radius: 0.045 * h * build,
        }
    }
}

/// Joint angles already sampled at a specific time `t` (degrees).
#[derive(Clone, Copy, Default)]
pub struct PoseSample {
    pub spine: Vec3,
    pub head: Vec3,
    pub l_shoulder: Vec3,
    pub r_shoulder: Vec3,
    pub l_elbow: f32,
    pub r_elbow: f32,
    pub l_hip: Vec3,
    pub r_hip: Vec3,
    pub l_knee: f32,
    pub r_knee: f32,
}

pub enum BoneKind {
    Capsule,
    Sphere,
}

/// One renderable rig part in rig-local space (i.e. relative to the humanoid object's own
/// position/rotation/scale, which the renderer applies on top).
pub struct BonePart {
    pub center: Vec3,
    pub rotation: Quat,
    pub length: f32,
    pub radius: f32,
    pub kind: BoneKind,
}

fn euler_deg(v: Vec3) -> Quat {
    Quat::from_euler(glam::EulerRot::XYZ, v.x.to_radians(), v.y.to_radians(), v.z.to_radians())
}

fn capsule_between(a: Vec3, b: Vec3, radius: f32) -> BonePart {
    let diff = b - a;
    let len = diff.length().max(1e-4);
    let dir = diff / len;
    let rotation = Quat::from_rotation_arc(Vec3::Y, dir);
    BonePart { center: (a + b) * 0.5, rotation, length: len, radius, kind: BoneKind::Capsule }
}

/// Forward-kinematic solve: joint rotations -> world-space (rig-local) bone segments.
pub fn pose_to_parts(rig: &HumanoidRig, pose: &PoseSample) -> Vec<BonePart> {
    let mut parts = Vec::with_capacity(11);

    let pelvis = Vec3::new(0.0, rig.hip_y, 0.0);
    let r_spine = euler_deg(pose.spine);
    let shoulder = pelvis + r_spine * Vec3::new(0.0, rig.torso_len, 0.0);
    parts.push(capsule_between(pelvis, shoulder, rig.torso_radius));

    let r_head = r_spine * euler_deg(pose.head);
    let head_center = shoulder + (r_head * Vec3::Y) * (rig.neck_len + rig.head_radius);
    parts.push(BonePart {
        center: head_center,
        rotation: r_head,
        length: rig.head_radius * 2.0,
        radius: rig.head_radius,
        kind: BoneKind::Sphere,
    });

    let sl = shoulder + r_spine * Vec3::new(-rig.shoulder_half, 0.0, 0.0);
    let sr = shoulder + r_spine * Vec3::new(rig.shoulder_half, 0.0, 0.0);

    let r_lsh = r_spine * euler_deg(pose.l_shoulder);
    let el = sl + r_lsh * Vec3::new(0.0, -rig.upper_arm_len, 0.0);
    parts.push(capsule_between(sl, el, rig.upper_arm_radius));
    let r_lel = r_lsh * Quat::from_rotation_x((-pose.l_elbow).to_radians());
    let wl = el + r_lel * Vec3::new(0.0, -rig.forearm_len, 0.0);
    parts.push(capsule_between(el, wl, rig.forearm_radius));

    let r_rsh = r_spine * euler_deg(pose.r_shoulder);
    let er = sr + r_rsh * Vec3::new(0.0, -rig.upper_arm_len, 0.0);
    parts.push(capsule_between(sr, er, rig.upper_arm_radius));
    let r_rel = r_rsh * Quat::from_rotation_x((-pose.r_elbow).to_radians());
    let wr = er + r_rel * Vec3::new(0.0, -rig.forearm_len, 0.0);
    parts.push(capsule_between(er, wr, rig.forearm_radius));

    let hl = pelvis + Vec3::new(-rig.hip_half, 0.0, 0.0);
    let hr = pelvis + Vec3::new(rig.hip_half, 0.0, 0.0);

    let r_lhip = euler_deg(pose.l_hip);
    let kl = hl + r_lhip * Vec3::new(0.0, -rig.upper_leg_len, 0.0);
    parts.push(capsule_between(hl, kl, rig.upper_leg_radius));
    let r_lkn = r_lhip * Quat::from_rotation_x(pose.l_knee.to_radians());
    let al = kl + r_lkn * Vec3::new(0.0, -rig.lower_leg_len, 0.0);
    parts.push(capsule_between(kl, al, rig.lower_leg_radius));
    parts.push(capsule_between(al, al + (r_lkn * Vec3::Z) * rig.foot_len, rig.foot_radius));

    let r_rhip = euler_deg(pose.r_hip);
    let kr = hr + r_rhip * Vec3::new(0.0, -rig.upper_leg_len, 0.0);
    parts.push(capsule_between(hr, kr, rig.upper_leg_radius));
    let r_rkn = r_rhip * Quat::from_rotation_x(pose.r_knee.to_radians());
    let ar = kr + r_rkn * Vec3::new(0.0, -rig.lower_leg_len, 0.0);
    parts.push(capsule_between(kr, ar, rig.lower_leg_radius));
    parts.push(capsule_between(ar, ar + (r_rkn * Vec3::Z) * rig.foot_len, rig.foot_radius));

    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_pose_is_upright_and_symmetric() {
        let rig = HumanoidRig::new(1.8, 1.0);
        let pose = PoseSample::default();
        let parts = pose_to_parts(&rig, &pose);
        assert_eq!(parts.len(), 12); // torso, head, 2x(upper arm, forearm), 2x(upper leg, lower leg, foot)
        // torso (part 0) should run straight up
        assert!(parts[0].center.x.abs() < 1e-4);
        // left/right upper arms (parts 2, 4) should mirror in x
        assert!((parts[2].center.x + parts[4].center.x).abs() < 1e-4);
    }

    #[test]
    fn bending_elbow_shortens_hand_to_shoulder_distance() {
        let rig = HumanoidRig::new(1.8, 1.0);
        let mut pose = PoseSample::default();
        let straight = pose_to_parts(&rig, &pose);
        let wrist_straight = straight[3].center; // left forearm segment center
        pose.l_elbow = 90.0;
        let bent = pose_to_parts(&rig, &pose);
        let wrist_bent = bent[3].center;
        // Bending the elbow should pull the forearm segment noticeably closer to the shoulder.
        let shoulder = bent[0].center; // approx torso center, good enough for a distance check
        assert!((wrist_bent - shoulder).length() < (wrist_straight - shoulder).length());
    }
}
