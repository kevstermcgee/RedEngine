//! Shooter feedback in the windowed client (ADR 0052): the sounds and screen effects for what the server reports, and, online, the local
//! cosmetics of our own attacks (recoil, muzzle flash, the bat's swing) that must not wait for a round trip.
//!
//! The decisions live in `red_engine2::feel` (a pure state machine, tested there) and the clips in `red_engine2::sfx`; this file only feeds
//! them what the client knows and plays what comes out.

use super::*;
use red_engine2::feel::{Cue, Own};
use red_engine2::net::protocol::{RosterEntry, FLAG_PROTECTED};
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
            }
        }
    }
}
