//! Weapons and interaction in the windowed game: the bat swing, the revolver, weapon switching, pick-up/drop (E), what the crosshair
//! probes, and (online) the action-button pulses sent to the server, which owns weapons there (ADR 0016).

use super::*;

impl App {
    /// The seeker's primary (and only) action on objects. Runs once per bat swing, at the start
    /// of the strike phase, and only when the swing's ray struck real geometry within
    /// `MELEE_REACH` (see `red_engine2::hit`): logs it and plays the impact thunk (the object itself
    /// does not change colour). A swing that touches nothing never gets here, so it is silent.
    pub(crate) fn hit_with(&mut self, object_index: usize) {
        let id = self.scene.objects[object_index].id.clone();
        println!("Hit '{id}' with the bat!");
        if let Some(audio) = &self.audio {
            audio.play(&self.hit_sound);
        }
        self.rules.inject(self.clock.ticks_run(), "hit", Some(0));
    }

    /// Blends between `idle`, `windup`, and `strike` values across the current swing's three
    /// phases — `idle` itself when not swinging. Shared by the first-person weapon's pitch, the
    /// third-person arm's shoulder/elbow pose, and the weapon's forward lunge, each of which
    /// just plugs in different endpoint values for the same windup/strike/recover curve.
    pub(crate) fn swing_blend(&self, idle: f32, windup: f32, strike: f32) -> f32 {
        match self.swing_timer {
            None => idle,
            Some(elapsed) if elapsed < SWING_WINDUP => {
                let f = Ease::Out.apply(elapsed / SWING_WINDUP);
                idle + (windup - idle) * f
            }
            Some(elapsed) if elapsed < SWING_WINDUP + SWING_STRIKE => {
                let f = Ease::In.apply((elapsed - SWING_WINDUP) / SWING_STRIKE);
                windup + (strike - windup) * f
            }
            Some(elapsed) => {
                let f = Ease::Out.apply(((elapsed - SWING_WINDUP - SWING_STRIKE) / SWING_RECOVER).min(1.0));
                strike + (idle - strike) * f
            }
        }
    }

    /// The bat viewmodel's current world transform: an idle held pose, or mid-swing pose
    /// interpolated by elapsed time through windup/strike/recover. Pitch is rotation about the
    /// camera's local right axis (tipping the bar up/down); roll (about the bar's own axis, so
    /// the shaft itself doesn't visibly change) stays fixed to keep the bat leaning across
    /// the view.
    pub(crate) fn weapon_transform(&self) -> Mat4 {
        // The bat viewmodel is camera-attached, not bound to the body rig's hand bone, so in
        // third person it would just float in front of the (now distant) camera. Shrinking it
        // away reuses the same "scale to near-nothing" hide trick as the player body's own
        // first-person visibility toggle rather than adding a second code path.
        if self.view_mode == ViewMode::ThirdPerson || !self.body.has_bat || self.carrying() {
            return Mat4::from_scale(Vec3::splat(HIDDEN_SCALE));
        }
        // Scrolling between weapons lowers the old one and raises the new one.
        let dip = self.switch_dip();
        if let Some(spec) = self.shown_weapon().firearm() {
            let k = self.recoil_kick() * spec.recoil;
            let ads = self.ads_blend;
            let local_rotation = Mat4::from_rotation_y(GUN_YAW_DEG.to_radians())
                * Mat4::from_rotation_x((GUN_IDLE_PITCH_DEG + GUN_RECOIL_PITCH_DEG * k + 25.0 * dip).to_radians());
            let hip = Vec3::new(GUN_RIGHT, -GUN_DOWN - 0.30 * dip, GUN_FORWARD - GUN_RECOIL_BACK * k);
            let aimed = Vec3::new(0.0, -0.035 - 0.30 * dip, GUN_FORWARD + 0.06 - GUN_RECOIL_BACK * k);
            let local_offset = hip.lerp(aimed, ads);
            return viewmodel_transform(&self.camera, local_offset, local_rotation);
        }
        let pitch_deg = self.swing_blend(IDLE_PITCH_DEG, WINDUP_PITCH_DEG, STRIKE_PITCH_DEG);
        let lunge = self.swing_blend(0.0, 0.0, STRIKE_LUNGE);
        let local_rotation = Mat4::from_rotation_z(IDLE_ROLL_DEG.to_radians()) * Mat4::from_rotation_x((pitch_deg + 30.0 * dip).to_radians());
        let local_offset = Vec3::new(VM_RIGHT, -VM_DOWN - 0.35 * dip, VM_FORWARD + lunge);
        viewmodel_transform(&self.camera, local_offset, local_rotation)
    }

    /// The weapon drawn right now: during a switch the old one until the halfway point, then the new.
    pub(crate) fn shown_weapon(&self) -> Weapon {
        match self.switching {
            Some((from, t)) if t < SWITCH_TIME * 0.5 => from,
            _ => self.weapon,
        }
    }

    /// 0..1..0 over a weapon switch (how far the viewmodel is lowered), 0 when not switching.
    pub(crate) fn switch_dip(&self) -> f32 {
        self.switching.map_or(0.0, |(_, t)| (std::f32::consts::PI * (t / SWITCH_TIME).clamp(0.0, 1.0)).sin())
    }

    /// Recoil kick right after a shot, 1 -> 0 (eased) over `RECOIL_TIME`.
    pub(crate) fn recoil_kick(&self) -> f32 {
        let f = (self.since_shot / RECOIL_TIME).clamp(0.0, 1.0);
        (1.0 - f) * (1.0 - f)
    }

    /// Scroll wheel: switch between the bat and the revolver (human only, not while carrying).
    pub(crate) fn on_scroll(&mut self, lines: f32) {
        if self.net.is_some() {
            // Online the server owns weapons: turn enough scrolling into one "switch" press.
            self.scroll_accum += lines;
            if self.scroll_accum.abs() >= SCROLL_LINES_PER_SWITCH {
                self.net_pulse[2] = NET_PULSE_TICKS;
                self.scroll_accum = 0.0;
            }
            return;
        }
        if !self.body.has_bat || self.carrying() || self.switch.is_active() || self.switch_queued.is_some() {
            return;
        }
        self.scroll_accum += lines;
        if self.scroll_accum.abs() >= SCROLL_LINES_PER_SWITCH {
            self.switch_queued = Some(if self.scroll_accum > 0.0 { 1 } else { -1 });
            self.scroll_accum = 0.0;
        }
    }

    /// Simulation tick: performs the queued weapon switch.
    pub(crate) fn begin_switch(&mut self, dir: i32) {
        if !self.body.has_bat || self.carrying() || self.switch.is_active() {
            return;
        }
        let to = self.weapon.cycle(dir);
        self.switch.start(self.weapon);
        self.weapon = to;
        self.swing.cancel();
        println!("Weapon: {}", to.name());
        if to.is_firearm() {
            if let Some(audio) = &self.audio {
                audio.play(&self.click_sound);
            }
        }
    }

    /// Simulation tick: one revolver shot at the crosshair (hitscan) from the tick's eye. Infinite ammo for now.
    pub(crate) fn fire_firearm(&mut self) {
        let Some(spec) = self.weapon.firearm() else { return };
        if !self.shot_cd.ready() || self.switch.is_active() || self.carrying() {
            return;
        }
        if !self.ammo.try_fire() {
            self.shot_cd.start(DRY_FIRE_COOLDOWN_TICKS);
            if let Some(audio) = &self.audio {
                audio.play(&self.click_sound);
            }
            return;
        }
        self.shot_cd.start(spec.cooldown_ticks);
        self.since_shot = 0.0;
        self.flash_left = MUZZLE_FLASH_TIME;
        if let Some(audio) = &self.audio {
            audio.play(&self.shot_sound);
        }
        let dir = self.camera.forward();
        let eye = self.tick_eye();
        if let Some((object_index, distance, loose)) = self.probe(eye, spec.range) {
            if let (Some(prop), Some(props)) = (loose, self.props.as_mut()) {
                props.strike_impulse(prop, dir, eye + dir * distance, spec.impulse);
            }
            println!("Shot '{}' at {:.1} m", self.scene.objects[object_index].id, distance);
        }
        self.rules.inject(self.clock.ticks_run(), "shot", Some(0));
    }

    /// Whether action button `i` is still held this tick (online pulses count down; offline they are never set).
    pub(crate) fn take_pulse(&mut self, i: usize) -> bool {
        let held = self.net_pulse[i] > 0;
        self.net_pulse[i] = self.net_pulse[i].saturating_sub(1);
        held
    }

    /// Weapon logic for one tick: advance the timers (an action queued on tick T first advances on
    /// tick T+1), resolve a landing bat strike, then perform the input queued since the last tick.
    pub(crate) fn fixed_step_combat(&mut self) {
        if self.net.is_some() {
            // Online the server runs the weapons; a click becomes an "attack" press on the next inputs.
            if std::mem::take(&mut self.attack_queued) {
                self.net_pulse[1] = NET_PULSE_TICKS;
            }
            self.switch_queued = None;
            return;
        }
        let strike = self.swing.tick();
        self.shot_cd.tick();
        self.switch.tick();

        if strike {
            let eye = self.tick_eye();
            if let Some((object_index, distance, loose)) = self.melee_probe(eye) {
                // A loose prop gets knocked, too: light things fly, heavy ones shuffle.
                if let (Some(prop), Some(props)) = (loose, self.props.as_mut()) {
                    let dir = self.camera.forward();
                    props.strike(prop, dir, eye + dir * distance);
                }
                self.hit_with(object_index);
            }
        }

        if let Some(dir) = self.switch_queued.take() {
            self.begin_switch(dir);
        }
        if std::mem::take(&mut self.attack_queued) && self.body.has_bat && !self.carrying() && !self.switch.is_active() {
            match self.weapon {
                Weapon::Bat => {
                    self.swing.start();
                }
                _ => self.fire_firearm(),
            }
        }
    }

    /// True while carrying a prop.
    pub(crate) fn carrying(&self) -> bool {
        self.props.as_ref().is_some_and(|p| p.held().is_some())
    }

    /// E: drop what is carried (it keeps the player's momentum), else pick up the prop under the crosshair.
    pub(crate) fn interact(&mut self) {
        if self.net.is_some() {
            self.net_pulse[0] = NET_PULSE_TICKS; // the server picks up / drops
            return;
        }
        let toss = self.camera.forward_flat() * 1.0;
        let Some(props) = self.props.as_mut() else { return };
        if props.held().is_some() {
            props.drop_held(self.player_vel + toss);
            self.rules.inject(self.clock.ticks_run(), "drop", Some(0));
        } else if let Some(p) = self.pickup_target {
            props.pick_up(p);
            self.swing.cancel();
            self.swing_timer = None;
            self.rules.inject(self.clock.ticks_run(), "pickup", Some(0));
        }
    }

    /// What a swing from `eye` along the camera would strike: fixed geometry (exact shapes) or a
    /// loose prop, whichever is nearer. `(scene object index, distance, loose-prop index)`.
    pub(crate) fn melee_probe(&self, eye: Vec3) -> Option<(usize, f32, Option<usize>)> {
        self.probe(eye, MELEE_REACH)
    }

    /// [`melee_probe`](Self::melee_probe) for any reach (a bullet travels 80 m).
    pub(crate) fn probe(&self, eye: Vec3, reach: f32) -> Option<(usize, f32, Option<usize>)> {
        let dir = self.camera.forward();
        let fixed = raycast_shapes(eye, dir, reach, &self.hit_shapes).map(|h| (h.object_index, h.distance, None));
        let loose = self.props.as_ref().and_then(|p| p.ray_props(eye, dir, reach).map(|(prop, d)| (p.props()[prop].object_index, d, Some(prop))));
        match (fixed, loose) {
            (Some(f), Some(l)) => Some(if l.1 < f.1 { l } else { f }),
            (f, l) => f.or(l),
        }
    }
}
