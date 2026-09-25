//! The player's own body model: keeping the drawn humanoid / rat posed from movement (walk cycle, swing, carry, aim).

use super::*;

impl App {
    /// Updates the player body's world transform and pose for this frame: facing/position from
    /// `planar_pos`/`yaw_deg` (the pre-third-person-pullback player position — the character
    /// moves, the camera just watches it from farther away in third person), a walk cycle driven
    /// by `speed` when moving, and a subtle idle sway otherwise. The right arm's shoulder/elbow
    /// are additionally blended toward the swing pose (via `swing_blend`) whenever a bat
    /// swing is in progress, so a melee attack visibly moves the arm instead of just the weapon.
    /// Also computes `self.hand_prop_transform` — the third-person bat rigidly welded to
    /// that same right forearm bone — and flips both the body's and the hand prop's scale
    /// between `HIDDEN_SCALE` and life-size depending on `self.view_mode`, since there's no
    /// per-object render-visibility flag to hide the player's own body in first person instead.
    pub(crate) fn update_player_body(&mut self, planar_pos: Vec2, yaw_deg: f32, speed: f32, dt: f32) {
        let third_person = self.view_mode == ViewMode::ThirdPerson;
        let scale = if third_person { Vec3::ONE } else { Vec3::splat(HIDDEN_SCALE) };
        let body_pos = Vec3::new(planar_pos.x, self.foot_y, planar_pos.y);
        {
            let body = &mut self.scene.objects[self.player_object_index];
            body.position = Track::constant(body_pos);
            body.rotation = Track::constant(Vec3::new(0.0, yaw_deg, 0.0));
            body.scale = Track::constant(scale);
        }
        match self.character {
            Character::Human => self.pose_human(body_pos, yaw_deg, speed, dt, scale),
            Character::Rat => self.pose_rat(speed, dt),
        }
    }

    /// Cheddar's gait: legs and body bob follow distance travelled, the tail streams when he
    /// runs and idles with a gentle sway when he stops.
    pub(crate) fn pose_rat(&mut self, speed: f32, dt: f32) {
        self.walk_phase += dt * speed * RAT_GAIT_RAD_PER_M;
        let stride = (speed / self.body.sprint_speed).clamp(0.0, 1.0);
        let (gait, sway) = (self.walk_phase, self.start.elapsed().as_secs_f32() * 1.3);
        if let ObjectKind::Rat(r) = &mut self.scene.objects[self.player_object_index].kind {
            r.gait = Track::constant(gait);
            r.stride = Track::constant(stride);
            r.sway = Track::constant(sway);
        }
        self.hand_prop_transform = Mat4::from_scale(Vec3::splat(HIDDEN_SCALE));
    }

    /// The human's walk cycle and bat swing (see `update_player_body`'s notes above): the walk
    /// cycle is driven by `speed`, the right arm blends toward the swing pose, and the
    /// third-person bat is welded to the right forearm bone.
    pub(crate) fn pose_human(&mut self, body_pos: Vec3, yaw_deg: f32, speed: f32, dt: f32, scale: Vec3) {
        if speed > 0.0 {
            self.walk_phase += dt * speed * (WALK_CYCLES_PER_SEC_AT_WALK_SPEED / self.body.walk_speed) * std::f32::consts::TAU;
        }

        let (spine_x, l_hip_x, r_hip_x, l_knee, r_knee, l_sh_x, r_sh_x) = if speed > 0.0 {
            let ph = self.walk_phase;
            (
                3.0 * (ph * 2.0).sin(),
                HIP_SWING_DEG * ph.sin(),
                -HIP_SWING_DEG * ph.sin(),
                KNEE_REST_DEG + (KNEE_LIFT_DEG * (-ph).sin()).max(0.0),
                KNEE_REST_DEG + (KNEE_LIFT_DEG * ph.sin()).max(0.0),
                -SHOULDER_SWING_DEG * ph.sin(),
                SHOULDER_SWING_DEG * ph.sin(),
            )
        } else {
            let idle_t = self.start.elapsed().as_secs_f32();
            (IDLE_SWAY_DEG * (idle_t * 1.1).sin(), 0.0, 0.0, KNEE_REST_DEG, KNEE_REST_DEG, 0.0, 0.0)
        };

        // The right arm additionally blends toward the bat's windup/strike pose during a
        // swing — `r_sh_x` (this frame's walk-cycle value) is the blend's idle endpoint, so a
        // swing mid-stride recovers back into whatever the walk cycle is doing by then rather
        // than snapping to a fixed rest angle.
        let carrying = self.carrying();
        let aiming = !carrying && self.shown_weapon() == Weapon::Revolver;
        let kick = self.recoil_kick();
        // Carrying: both arms forward, elbows bent, holding the prop out in front of the chest.
        // Aiming the revolver: the gun arm out level (following where the player looks), kicking on a shot.
        let aim_shoulder = (AIM_SHOULDER_X - self.camera.pitch.to_degrees() + 14.0 * kick).clamp(-175.0, -20.0);
        let l_sh_x_final = if carrying {
            CARRY_SHOULDER_X
        } else if aiming {
            aim_shoulder
        } else {
            self.swing_blend(l_sh_x, ARM_WINDUP_SHOULDER_X, ARM_STRIKE_SHOULDER_X)
        };
        let l_elbow_final = if carrying {
            CARRY_ELBOW_DEG
        } else if aiming {
            AIM_ELBOW_DEG + 22.0 * kick
        } else {
            self.swing_blend(ARM_IDLE_ELBOW_DEG, ARM_WINDUP_ELBOW_DEG, ARM_STRIKE_ELBOW_DEG)
        };
        let r_sh_x = if carrying { CARRY_SHOULDER_X } else { r_sh_x };
        let r_elbow = if carrying { CARRY_ELBOW_DEG } else { KNEE_REST_DEG };

        let third_person = self.view_mode == ViewMode::ThirdPerson && !carrying;
        let l_shoulder = Vec3::new(l_sh_x_final, 0.0, -6.0);
        let r_shoulder = Vec3::new(r_sh_x, 0.0, 6.0);
        if let ObjectKind::Humanoid(h) = &mut self.scene.objects[self.player_object_index].kind {
            h.pose.spine = Track::constant(Vec3::new(spine_x, 0.0, 0.0));
            h.pose.l_hip = Track::constant(Vec3::new(l_hip_x, 0.0, 0.0));
            h.pose.r_hip = Track::constant(Vec3::new(r_hip_x, 0.0, 0.0));
            h.pose.l_knee = Track::constant(l_knee);
            h.pose.r_knee = Track::constant(r_knee);
            h.pose.l_shoulder = Track::constant(l_shoulder);
            h.pose.r_shoulder = Track::constant(r_shoulder);
            h.pose.l_elbow = Track::constant(l_elbow_final);
            h.pose.r_elbow = Track::constant(r_elbow);
        }

        // Weld the third-person bat to the right forearm bone: run the same forward-kinematic
        // solve the renderer uses (`skeleton::pose_to_parts`) with this frame's exact pose, take
        // the forearm bone's world transform, and place the grip at its far (wrist) end. The
        // forearm's own rotation (from `capsule_between`'s `Quat::from_rotation_arc(Y, dir)`) only
        // pins its local Y axis to the elbow-to-wrist direction — roll around that axis is
        // otherwise arbitrary, so `HAND_GRIP_ROLL_DEG` is a fixed fudge rather than a derived
        // value; it reads fine in practice since the arm doesn't twist much in this rig.
        self.hand_prop_transform = if third_person {
            let rig = HumanoidRig::new(PLAYER_HEIGHT, PLAYER_BUILD);
            let pose = PoseSample {
                spine: Vec3::new(spine_x, 0.0, 0.0),
                head: Vec3::ZERO,
                l_shoulder,
                r_shoulder,
                l_elbow: l_elbow_final,
                r_elbow: KNEE_REST_DEG,
                l_hip: Vec3::new(l_hip_x, 0.0, 0.0),
                r_hip: Vec3::new(r_hip_x, 0.0, 0.0),
                l_knee,
                r_knee,
            };
            let forearm = &pose_to_parts(&rig, &pose)[BAT_FOREARM_PART];
            let body_world = Mat4::from_scale_rotation_translation(scale, Quat::from_rotation_y(yaw_deg.to_radians()), body_pos);
            let hand_bone_world = body_world
                * Mat4::from_rotation_translation(forearm.rotation, forearm.center)
                * Mat4::from_translation(Vec3::new(0.0, forearm.length * 0.5, 0.0));
            let wrist = hand_bone_world.transform_point3(Vec3::ZERO);
            let (fwd, right) = (self.camera.forward_flat(), self.camera.right_flat());
            let basis = Mat4::from_cols(right.extend(0.0), Vec3::Y.extend(0.0), fwd.extend(0.0), Vec4::new(0.0, 0.0, 0.0, 1.0));
            if aiming {
                // The revolver aims where the player looks (pitch tips the barrel, recoil kicks it up).
                Mat4::from_translation(wrist) * basis * Mat4::from_rotation_x((-self.camera.pitch) + (-0.30 * kick))
            } else {
                let pitch_deg = self.swing_blend(IDLE_PITCH_DEG, WINDUP_PITCH_DEG, STRIKE_PITCH_DEG);
                Mat4::from_translation(wrist) * basis * Mat4::from_rotation_z(IDLE_ROLL_DEG.to_radians()) * Mat4::from_rotation_x(pitch_deg.to_radians())
            }
        } else {
            Mat4::from_scale(Vec3::splat(HIDDEN_SCALE))
        };
    }
}
