//! Shooter feedback in the windowed client (ADR 0052): the sounds and screen effects for what the server reports, and, online, the local
//! cosmetics of our own attacks (recoil, muzzle flash, the bat's swing) that must not wait for a round trip.
//!
//! The decisions live in `red_engine2::feel` (a pure state machine, tested there) and the clips in `red_engine2::sfx`; this file only feeds
//! them what the client knows and plays what comes out.

use super::*;
use red_engine2::feel::{Cue, Own};
use red_engine2::net::protocol::{RosterEntry, FLAG_PROTECTED};
use red_engine2::net::session::{nearest_body_on_ray, RemoteBody};
use red_engine2::sfx::Listener;
use red_engine2::ui::online::CombatView;

/// [`PlayerSnap::flags`](red_engine2::net::protocol::PlayerSnap) bit: the player is dead.
const FLAG_DEAD: u8 = 4;
/// How long the level-up line stays on screen, seconds.
const NOTICE_SECS: f32 = 1.8;
/// Eye height of the camera lying where we fell, metres.
pub(crate) const DEAD_EYE_HEIGHT: f32 = 0.3;

impl App {
    /// Whether the server says we are dead (waiting to respawn).
    pub(crate) fn own_dead(&self) -> bool {
        self.net.as_ref().and_then(|n| n.own).is_some_and(|o| o.flags & FLAG_DEAD != 0)
    }

    /// Our state in the shape `Feel` takes (`None` until the server has told us).
    fn own_for_feel(&self) -> Option<Own> {
        let own = self.net.as_ref()?.own?;
        let ladder = self.scene.weapons.ladder();
        Some(Own {
            hp: own.hp as u32,
            dead: own.flags & FLAG_DEAD != 0,
            protected: own.flags & FLAG_PROTECTED != 0,
            weapon: own.weapon,
            last_rung: ladder.last().is_some_and(|w| w.wire() == own.weapon),
            ladder: !ladder.is_empty(),
        })
    }

    /// Our rung on the weapon ladder (0-based): the one our kills earned if it is the weapon in hand, else where the weapon sits on it.
    fn rung_of(&self, weapon: Weapon, kills: u32) -> Option<usize> {
        let ladder = self.scene.weapons.ladder();
        let earned = (kills as usize).min(ladder.len().checked_sub(1)?);
        if ladder[earned] == weapon {
            Some(earned)
        } else {
            ladder.iter().position(|w| *w == weapon)
        }
    }

    /// What the shooter HUD shows about us (`None` before the server has told us anything).
    pub(crate) fn combat_view(&self, me: u8, roster: &[RosterEntry]) -> Option<CombatView> {
        let net = self.net.as_ref()?;
        let own = net.own?;
        let weapon = Weapon::from_wire(own.weapon);
        let ladder = self.scene.weapons.ladder();
        let kills = roster.iter().find(|e| e.id == me).map_or(0, |e| e.score as u32);
        let rung = self.rung_of(weapon, kills);
        Some(CombatView {
            hp: own.hp as u32,
            max_hp: red_engine2::weapons::PLAYER_MAX_HP,
            weapon: weapon.name().to_string(),
            rung: rung.map(|i| (i as u32 + 1, ladder.len() as u32)),
            next_weapon: rung.and_then(|i| ladder.get(i + 1)).map(|w| w.name().to_string()),
            dead: own.flags & FLAG_DEAD != 0,
            respawn_secs: net.client.respawn_in_secs().ceil() as u32,
            protected: own.flags & FLAG_PROTECTED != 0,
            notice: self.notice.as_ref().map(|(text, _)| text.clone()),
        })
    }

    /// Once per rendered frame: turns what the network reported into sounds and screen effects, and lets the effects age.
    pub(crate) fn feedback_frame(&mut self, dt: f32) {
        if let Some((_, left)) = &mut self.notice {
            *left -= dt;
            if *left <= 0.0 {
                self.notice = None;
            }
        }
        let happened = self.net.as_mut().map(|n| n.take_happened()).unwrap_or_default();
        if let Some(own) = self.own_for_feel().filter(|_| self.debug_feel.is_none()) {
            for h in &happened {
                self.feel.on_happened(h, &own);
            }
            self.feel.observe_own(&own);
            // Coming back to life: face the way the server placed us.
            if self.was_dead && !own.dead {
                if let Some(o) = self.net.as_ref().and_then(|n| n.own) {
                    self.camera.yaw = o.yaw;
                    self.camera.pitch = 0.0;
                }
            }
            self.was_dead = own.dead;
        }
        if let Some(view) = self.online_view() {
            self.feel.observe_match(view.phase, view.secs_left, view.winner == view.me);
            let rungs = self.scene.weapons.ladder().len() as u32;
            let rival =
                (rungs > 0 && view.phase == red_engine2::sim::flow::Phase::Playing).then(|| red_engine2::ui::online::final_rung_rival(&view, rungs)).flatten();
            self.feel.observe_threat(rival.map(|e| e.id));
        }
        if let Some(what) = &self.debug_feel {
            self.feel.hold(what);
        }
        self.feel.tick(dt);
        for cue in self.feel.take_cues() {
            self.play_cue(&cue);
        }
    }

    /// Plays one cue: the clip, at its loudness, placed in the stereo field.
    fn play_cue(&mut self, cue: &Cue) {
        if let Cue::LevelUp { .. } = cue {
            if let Some(view) = self.net.as_ref().and_then(|n| n.own) {
                let kills = self.online_view().and_then(|v| v.roster.iter().find(|e| e.id == v.me).map(|e| e.score as u32)).unwrap_or(0);
                let weapon = Weapon::from_wire(view.weapon);
                let rung = self.rung_of(weapon, kills).unwrap_or(kills as usize);
                self.notice = Some((format!("RUNG {} - {}", rung + 1, weapon.name().to_uppercase()), NOTICE_SECS));
            }
        }
        if let Cue::Shot { weapon, at, yaw, pitch, shooter, own: false } = *cue {
            self.draw_remote_shot(Weapon::from_wire(weapon), at, yaw, pitch, shooter);
        }
        if self.log_cues {
            println!("[cue {:7.2}s] {cue:?}", self.start.elapsed().as_secs_f32());
        }
        let Some(audio) = &self.audio else { return };
        let listener = Listener { eye: self.eye.to_array(), yaw: self.camera.yaw };
        let played = self.sounds.play(cue, listener, self.step_count);
        if matches!(cue, Cue::Step) {
            self.step_count = self.step_count.wrapping_add(1);
        }
        if played.gain > 0.01 {
            audio.play_at(played.clip, played.gain, played.pan);
        }
    }

    /// Online, the server runs the weapons, but the click has to feel instant: this runs the same rules on the same input (a button acts when
    /// it goes down, automatic weapons repeat while held, a firearm waits out its cooldown) only to drive the recoil, the muzzle flash, the
    /// bat's swing and their sounds. What is actually hit is the server's word, and comes back as feedback.
    pub(crate) fn predict_attack(&mut self, attack: bool) {
        let edge = attack && !self.pred_prev_attack;
        self.pred_prev_attack = attack;
        self.swing.tick();
        self.shot_cd.tick();
        self.switch.tick();
        if self.own_dead() || !(edge || (attack && self.weapon.automatic())) {
            return;
        }
        if !self.body.has_bat || self.carrying() || self.switch.is_active() {
            return;
        }
        match self.weapon {
            Weapon::Bat => {
                if self.swing.start() {
                    self.feel.own_swing();
                }
            }
            weapon => {
                let Some(spec) = weapon.firearm() else { return };
                if !self.shot_cd.ready() {
                    return;
                }
                self.shot_cd.start(spec.cooldown_ticks);
                self.since_shot = 0.0;
                self.flash_left = MUZZLE_FLASH_TIME;
                self.feel.own_shot(weapon.wire(), self.tick_eye());
                self.draw_own_shot(weapon);
            }
        }
    }

    /// Draws where our shot went: a tracer from just ahead of the gun to what the crosshair ray met, and a spark there.
    pub(crate) fn draw_own_shot(&mut self, weapon: Weapon) {
        let eye = self.tick_eye();
        let dir = self.camera.forward();
        let muzzle = eye + dir * 0.55 + self.camera.right() * 0.16 - self.camera.up() * 0.14;
        let me = self.net.as_ref().and_then(|n| n.client.my_id());
        self.draw_shots(weapon, muzzle, eye, dir, me, true);
    }

    /// Draws where another player's shot went, from where they stood and the way they aimed.
    fn draw_remote_shot(&mut self, weapon: Weapon, at: Vec3, yaw: f32, pitch: f32, shooter: u8) {
        let (sy, cy) = yaw.sin_cos();
        let (sp, cp) = pitch.sin_cos();
        let dir = Vec3::new(sy * cp, sp, -cy * cp);
        let eye = at + Vec3::Y * 1.65;
        let muzzle = at + Vec3::Y * 1.3 + dir * 0.7;
        self.draw_shots(weapon, muzzle, eye, dir, Some(shooter), false);
    }

    /// A tracer per pellet from `muzzle`, along `dir` from `eye`, to the first wall or player it meets (or a good way into the distance), with a
    /// spark where it landed: red on a person, warm on a wall. `shooter` cannot be hit by their own shot; when someone else fires we can be.
    fn draw_shots(&mut self, weapon: Weapon, muzzle: Vec3, eye: Vec3, dir: Vec3, shooter: Option<u8>, own: bool) {
        let Some(spec) = weapon.firearm() else { return };
        let reach = spec.range.min(60.0);
        let mut bodies: Vec<RemoteBody> = self.net.as_ref().map(|n| n.bodies().to_vec()).unwrap_or_default();
        if !own {
            if let Some(id) = self.net.as_ref().and_then(|n| n.client.my_id()) {
                bodies.push(RemoteBody {
                    id,
                    pos: Vec3::new(self.physics_pos.x, self.foot_y, self.physics_pos.y),
                    dead: self.own_dead(),
                    character: self.character,
                });
            }
        }
        bodies.retain(|b| Some(b.id) != shooter);
        let tracer = if own { Vec3::new(1.0, 0.95, 0.65) } else { Vec3::new(1.0, 0.6, 0.3) };
        let Some(streaks) = self.streaks.as_mut() else { return };
        for pellet in 0..weapon.pellets() {
            let d = weapon.shot_direction(dir, pellet);
            let wall = raycast_shapes(eye, d, reach, &self.hit_shapes).map(|h| h.distance);
            let body = nearest_body_on_ray(&bodies, eye, d, reach).map(|(_, dist)| dist);
            let (dist, spark) = match (wall, body) {
                (Some(w), Some(b)) if b < w => (b, Some(Vec3::new(1.0, 0.15, 0.1))),
                (Some(w), _) => (w, Some(Vec3::new(1.0, 0.85, 0.4))),
                (None, Some(b)) => (b, Some(Vec3::new(1.0, 0.15, 0.1))),
                (None, None) => (reach, None),
            };
            let end = eye + d * dist;
            streaks.tracer(muzzle, end, tracer);
            if let Some(color) = spark {
                streaks.spark(end, color);
            }
        }
    }
}
