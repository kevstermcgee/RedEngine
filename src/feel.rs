//! Game feel: what the player *perceives* of a fight, as a pure state machine (no window, no audio device, no GPU).
//!
//! The network says what happened ([`Happenings`]: shots heard, hits landed, damage taken, kills) and what state we are in ([`Own`]:
//! hit points, dead or alive, weapon). [`Feel`] turns that into two things a client presents:
//! * **[`Cue`]s to hear**, once each (a hit tick, a kill ding, a level-up jingle a beat after it, hurt, death, respawn, the countdown's
//!   beeps, a heartbeat while nearly dead, a pad's launch, footsteps);
//! * **[`FxParams`] to see**, every frame: a red vignette that swells with damage and pulses with low health, an arc on the screen pointing
//!   at whoever hurt us (it swings as we turn), a hit marker that grows and fades, screen flashes for a level-up or a respawn.
//!
//! It is time-driven ([`Feel::tick`]) and deterministic, so its timing is unit-tested here; `sfx` turns cues into sound and `fx` draws the
//! parameters. Nothing in it affects the simulation.

use crate::net::happenings::Happenings;
use crate::sim::flow::Phase;
use glam::Vec3;

/// Health at or below which the screen edge pulses red and a heartbeat plays.
pub const LOW_HEALTH: u32 = 35;
/// How long a hit marker takes to fade, seconds (a kill's lasts [`KILL_MARKER_SECS`]).
pub const HIT_MARKER_SECS: f32 = 0.28;
/// How long a kill marker takes to fade, seconds.
pub const KILL_MARKER_SECS: f32 = 0.6;
/// How long the damage vignette and arc take to fade, seconds.
pub const HURT_SECS: f32 = 1.1;
/// Delay between the kill sound and the level-up jingle, so they do not land on top of each other.
pub const LEVEL_UP_DELAY_SECS: f32 = 0.16;
/// Metres walked between footsteps (a stride).
pub const STRIDE_M: f32 = 1.9;
/// Another player's shot is heard this long after the snapshot that reports it, because their avatar is drawn that far in the past (the
/// interpolation delay): the sound then lands with the muzzle flash.
pub const REMOTE_SOUND_DELAY_SECS: f32 = crate::net::interp::INTERP_DELAY as f32;

/// Something to hear.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cue {
    /// A firearm was fired: `weapon` is `Weapon::wire`, `at` where the shooter (player `shooter`) stands and `yaw` / `pitch` where they aimed; `own` when it
    /// was us (played centred, not placed).
    Shot { weapon: u8, at: Vec3, yaw: f32, pitch: f32, shooter: u8, own: bool },
    /// The bat swung.
    Swing,
    /// Our attack landed on a player (`bat`: the deeper knock).
    Hit { bat: bool },
    /// We killed someone.
    Kill,
    /// We climbed the weapon ladder (`last`: onto the final rung).
    LevelUp { last: bool },
    /// We took damage.
    Hurt,
    /// We died.
    Death,
    /// We are back.
    Respawn,
    /// A jump pad threw us.
    Pad,
    /// A jump.
    Jump,
    /// We landed; `0..1` is how hard.
    Land(f32),
    /// A footstep.
    Step,
    /// A countdown second.
    Beep,
    /// The countdown reached zero.
    Go,
    /// We won the round.
    Victory,
    /// We lost it.
    Defeat,
    /// A heartbeat (low health).
    Heartbeat,
    /// A weapon was raised.
    Draw,
    /// Someone else reached the last rung of the weapon ladder: one more kill and they win.
    Alert,
}

/// Our own state as the server last told us.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Own {
    /// Hit points.
    pub hp: u32,
    /// Waiting to respawn.
    pub dead: bool,
    /// Spawn protection is active.
    pub protected: bool,
    /// The weapon in hand (`Weapon::wire`).
    pub weapon: u8,
    /// Whether the weapon in hand is the last rung of a weapon ladder.
    pub last_rung: bool,
    /// Whether the game has a weapon ladder at all.
    pub ladder: bool,
}

/// What to draw over the finished frame this frame (see `shaders/fx.wgsl`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FxParams {
    /// Screen-edge tint: rgb and strength (0..1).
    pub vignette: [f32; 4],
    /// Whole-screen tint: rgb and alpha (0..1).
    pub flash: [f32; 4],
    /// Direction of whoever hurt us, radians relative to where we look (0 ahead, positive to the right).
    pub hurt_angle: f32,
    /// How strongly the damage arc shows (0 = not at all).
    pub hurt_strength: f32,
    /// Hit marker age, 0 (just hit) to 1 (gone); values of 1 or more draw nothing.
    pub marker_age: f32,
    /// Hit marker kind: 0 a hit, 1 a kill.
    pub marker_kind: f32,
}

impl Default for FxParams {
    /// Nothing to show (the marker's age is past its end).
    fn default() -> Self {
        FxParams { vignette: [0.0; 4], flash: [0.0; 4], hurt_angle: 0.0, hurt_strength: 0.0, marker_age: 1.0, marker_kind: 0.0 }
    }
}

impl FxParams {
    /// Whether anything at all needs drawing.
    pub fn active(&self) -> bool {
        self.vignette[3] > 0.001 || self.flash[3] > 0.001 || self.hurt_strength > 0.001 || self.marker_age < 1.0
    }
}

#[derive(Debug, Clone, Copy)]
struct Marker {
    age: f32,
    kill: bool,
}

#[derive(Debug, Clone, Copy)]
struct Flash {
    rgb: [f32; 3],
    alpha: f32,
    secs: f32,
    age: f32,
}

/// The presentation state of a fight. See the module docs.
#[derive(Debug, Clone, Default)]
pub struct Feel {
    cues: Vec<Cue>,
    later: Vec<(f32, Cue)>,
    marker: Option<Marker>,
    hurt: f32,
    hurt_bearing: f32,
    flash: Option<Flash>,
    own: Option<Own>,
    heartbeat_in: f32,
    time: f32,
    phase: Option<(Phase, Option<u32>)>,
    last_vy: f32,
    was_airborne: bool,
    fall_speed: f32,
    walked: f32,
    threat: Option<u8>,
}

impl Feel {
    /// A fresh state: nothing to hear or see.
    pub fn new() -> Feel {
        Feel::default()
    }

    /// Forgets our state (joined, resumed, a new round): the next observation is a baseline, and no old effect lingers.
    pub fn reset(&mut self) {
        *self = Feel::default();
    }

    fn cue(&mut self, cue: Cue) {
        self.cues.push(cue);
    }

    fn cue_later(&mut self, secs: f32, cue: Cue) {
        self.later.push((secs, cue));
    }

    fn flash(&mut self, rgb: [f32; 3], alpha: f32, secs: f32) {
        self.flash = Some(Flash { rgb, alpha, secs, age: 0.0 });
    }

    /// Applies what one snapshot reported. `own` is our state at the same moment.
    pub fn on_happened(&mut self, h: &Happenings, own: &Own) {
        for s in &h.shots {
            self.cue_later(REMOTE_SOUND_DELAY_SECS, Cue::Shot { weapon: s.weapon, at: s.pos, yaw: s.yaw, pitch: s.pitch, shooter: s.id, own: false });
        }
        if h.hits > 0 {
            self.cue(Cue::Hit { bat: own.weapon == 0 });
            if !self.marker.is_some_and(|m| m.kill && m.age < 0.15) {
                self.marker = Some(Marker { age: 0.0, kill: false });
            }
        }
        if h.kills > 0 {
            self.cue(Cue::Kill);
            self.marker = Some(Marker { age: 0.0, kill: true });
            if own.ladder {
                self.cue_later(LEVEL_UP_DELAY_SECS, Cue::LevelUp { last: own.last_rung });
                self.flash([1.0, 0.82, 0.25], 0.30, 0.45);
            }
        }
        if h.hurt > 0 {
            self.cue(Cue::Hurt);
            self.hurt = 1.0;
            self.hurt_bearing = h.bearing.unwrap_or(self.hurt_bearing);
        }
    }

    /// Notes our state each frame: dying, coming back, raising another weapon.
    pub fn observe_own(&mut self, own: &Own) {
        let Some(prev) = self.own.replace(*own) else { return };
        if own.dead && !prev.dead {
            self.cue(Cue::Death);
        } else if !own.dead && prev.dead {
            self.cue(Cue::Respawn);
            self.flash([0.55, 0.85, 1.0], 0.40, 0.55);
            self.hurt = 0.0;
        } else if !own.dead && own.weapon != prev.weapon {
            self.cue(Cue::Draw);
        }
        if own.hp <= LOW_HEALTH && !own.dead && prev.hp > LOW_HEALTH {
            self.heartbeat_in = 0.0;
        }
    }

    /// Notes the match: countdown beeps once a second, a note at "go", and a sting when the round ends (`won`: whether we won).
    pub fn observe_match(&mut self, phase: Phase, secs_left: Option<u32>, won: bool) {
        let now = (phase, secs_left);
        let prev = self.phase.replace(now);
        if phase == Phase::Countdown && secs_left.is_some_and(|s| s > 0) && prev != Some(now) {
            self.cue(Cue::Beep);
        }
        if let Some((was, _)) = prev {
            if was == Phase::Countdown && phase == Phase::Playing {
                self.cue(Cue::Go);
            }
            if was != Phase::Results && phase == Phase::Results {
                self.cue(if won { Cue::Victory } else { Cue::Defeat });
            }
        }
    }

    /// Notes the local player's movement each simulation tick: footsteps by distance walked, a jump, a landing (as hard as the fall),
    /// and a pad's launch (`pad_launch`: the smallest vertical speed a jump pad gives, if the map has any).
    pub fn observe_motion(&mut self, speed: f32, dt: f32, vy: f32, grounded: bool, pad_launch: Option<f32>, sprinting: bool) {
        if grounded {
            if self.was_airborne {
                self.cue(Cue::Land(((-self.fall_speed - 4.0) / 16.0).clamp(0.0, 1.0)));
                self.walked = 0.0;
            }
            self.walked += speed.max(0.0) * dt;
            let stride = if sprinting { STRIDE_M * 1.25 } else { STRIDE_M };
            if self.walked >= stride {
                self.walked -= stride;
                self.cue(Cue::Step);
            }
            self.fall_speed = 0.0;
        } else {
            self.fall_speed = self.fall_speed.min(vy);
            if !self.was_airborne {
                match pad_launch {
                    Some(min) if vy >= min * 0.85 && self.last_vy < min * 0.5 => self.cue(Cue::Pad),
                    _ if vy > 1.0 => self.cue(Cue::Jump),
                    _ => {}
                }
            }
        }
        self.was_airborne = !grounded;
        self.last_vy = vy;
    }

    /// We pulled the trigger (predicted locally: the sound must not wait for the server).
    pub fn own_shot(&mut self, weapon: u8, at: Vec3) {
        self.cue(Cue::Shot { weapon, at, yaw: 0.0, pitch: 0.0, shooter: 0, own: true });
    }

    /// We swung the bat.
    pub fn own_swing(&mut self) {
        self.cue(Cue::Swing);
    }

    /// Notes who, if anyone but us, is on the last rung of the weapon ladder (one kill from winning): a siren the moment someone gets there.
    pub fn observe_threat(&mut self, threat: Option<u8>) {
        if threat.is_some() && threat != self.threat {
            self.cue(Cue::Alert);
        }
        self.threat = threat;
    }

    /// Advances the timers by `dt` seconds.
    pub fn tick(&mut self, dt: f32) {
        self.time += dt;
        if let Some(m) = &mut self.marker {
            m.age += dt;
            if m.age >= if m.kill { KILL_MARKER_SECS } else { HIT_MARKER_SECS } {
                self.marker = None;
            }
        }
        self.hurt = (self.hurt - dt / HURT_SECS).max(0.0);
        if let Some(f) = &mut self.flash {
            f.age += dt;
            if f.age >= f.secs {
                self.flash = None;
            }
        }
        let mut due = Vec::new();
        self.later.retain_mut(|(left, cue)| {
            *left -= dt;
            if *left <= 0.0 {
                due.push(*cue);
                false
            } else {
                true
            }
        });
        self.cues.extend(due);
        if let Some(own) = self.own.filter(|o| o.hp <= LOW_HEALTH && !o.dead && o.hp > 0) {
            self.heartbeat_in -= dt;
            if self.heartbeat_in <= 0.0 {
                self.cue(Cue::Heartbeat);
                // Faster as health falls: about 0.95 s at the threshold, 0.55 s near death.
                self.heartbeat_in = 0.55 + 0.4 * own.hp as f32 / LOW_HEALTH as f32;
            }
        }
    }

    /// Screenshot and demo aid: holds one effect on screen every frame it is called. `what` is `hit`, `kill`, `hurt`, `dead`, `low`,
    /// `protected` or `flash` (anything else does nothing); unknown words are ignored.
    pub fn hold(&mut self, what: &str) {
        const ALIVE: Own = Own { hp: 100, dead: false, protected: false, weapon: 4, last_rung: false, ladder: true };
        match what {
            "hit" => self.marker = Some(Marker { age: HIT_MARKER_SECS * 0.25, kill: false }),
            "kill" => self.marker = Some(Marker { age: KILL_MARKER_SECS * 0.2, kill: true }),
            "hurt" => {
                self.hurt = 0.9;
                self.hurt_bearing = std::f32::consts::FRAC_PI_4;
            }
            "dead" => self.own = Some(Own { hp: 0, dead: true, ..ALIVE }),
            "low" => self.own = Some(Own { hp: 12, ..ALIVE }),
            "protected" => self.own = Some(Own { protected: true, ..ALIVE }),
            "flash" => self.flash = Some(Flash { rgb: [1.0, 0.82, 0.25], alpha: 0.30, secs: 1.0, age: 0.05 }),
            _ => {}
        }
    }

    /// The cues raised since the last call, in order.
    pub fn take_cues(&mut self) -> Vec<Cue> {
        std::mem::take(&mut self.cues)
    }

    /// The effects to draw this frame for a view looking along `view_yaw` (the damage arc is relative to it, so it swings as we turn).
    pub fn fx(&self, view_yaw: f32) -> FxParams {
        let mut fx = FxParams::default();
        let own = self.own;
        let dead = own.is_some_and(|o| o.dead);
        let hp = own.map_or(100, |o| o.hp);
        let low = if !dead && hp > 0 && hp <= LOW_HEALTH {
            let need = 1.0 - hp as f32 / LOW_HEALTH as f32;
            let beat = 0.5 + 0.5 * (self.time * std::f32::consts::TAU / (0.55 + 0.4 * hp as f32 / LOW_HEALTH as f32)).sin();
            (0.18 + 0.3 * need) * (0.55 + 0.45 * beat)
        } else {
            0.0
        };
        let red = (self.hurt * 0.62).max(low).max(if dead { 0.9 } else { 0.0 });
        if red > 0.0 {
            fx.vignette = [0.85, 0.04, 0.03, red.min(1.0)];
        } else if own.is_some_and(|o| o.protected && !o.dead) {
            fx.vignette = [0.15, 0.75, 1.0, 0.22 + 0.05 * (self.time * 6.0).sin()];
        }
        if let Some(f) = self.flash {
            let left = 1.0 - f.age / f.secs;
            fx.flash = [f.rgb[0], f.rgb[1], f.rgb[2], f.alpha * left * left];
        }
        if dead {
            fx.flash = [0.2, 0.0, 0.0, fx.flash[3].max(0.38)];
        }
        if self.hurt > 0.0 {
            fx.hurt_strength = self.hurt;
            fx.hurt_angle = wrap_pi(self.hurt_bearing - view_yaw);
        }
        if let Some(m) = self.marker {
            fx.marker_age = (m.age / if m.kill { KILL_MARKER_SECS } else { HIT_MARKER_SECS }).clamp(0.0, 1.0);
            fx.marker_kind = if m.kill { 1.0 } else { 0.0 };
        }
        fx
    }
}

/// `a` wrapped into `-pi..pi`.
pub fn wrap_pi(a: f32) -> f32 {
    let t = std::f32::consts::TAU;
    a - t * ((a + std::f32::consts::PI) / t).floor()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::happenings::ShotHeard;

    const ALIVE: Own = Own { hp: 100, dead: false, protected: false, weapon: 4, last_rung: false, ladder: true };

    fn happened(hits: u8, hurt: u8, kills: u8) -> Happenings {
        Happenings { hits, hurt, kills, bearing: (hurt > 0).then_some(std::f32::consts::FRAC_PI_2), ..Default::default() }
    }

    #[test]
    fn a_hit_ticks_and_shows_a_marker_that_fades_out() {
        let mut f = Feel::new();
        f.on_happened(&happened(1, 0, 0), &ALIVE);
        assert_eq!(f.take_cues(), vec![Cue::Hit { bat: false }]);
        assert_eq!(f.fx(0.0).marker_age, 0.0);
        f.tick(HIT_MARKER_SECS * 0.5);
        let mid = f.fx(0.0).marker_age;
        assert!((0.4..0.6).contains(&mid), "half way through: {mid}");
        f.tick(HIT_MARKER_SECS);
        assert!(f.fx(0.0).marker_age >= 1.0 && !f.fx(0.0).active(), "gone");
        f.on_happened(&happened(1, 0, 0), &Own { weapon: 0, ..ALIVE });
        assert_eq!(f.take_cues(), vec![Cue::Hit { bat: true }], "the bat has its own knock");
    }

    #[test]
    fn a_kill_dings_then_the_level_up_jingle_follows_a_beat_later() {
        let mut f = Feel::new();
        f.on_happened(&happened(1, 0, 1), &ALIVE);
        let now = f.take_cues();
        assert!(now.contains(&Cue::Kill) && !now.iter().any(|c| matches!(c, Cue::LevelUp { .. })), "{now:?}");
        assert_eq!(f.fx(0.0).marker_kind, 1.0, "a kill marker");
        f.tick(LEVEL_UP_DELAY_SECS * 0.5);
        assert!(f.take_cues().is_empty(), "not yet");
        f.tick(LEVEL_UP_DELAY_SECS);
        assert_eq!(f.take_cues(), vec![Cue::LevelUp { last: false }]);
        assert!(f.fx(0.0).flash[3] > 0.0, "a gold flash marks the level-up");
        let mut f = Feel::new();
        f.on_happened(&happened(1, 0, 1), &Own { last_rung: true, ..ALIVE });
        f.tick(1.0);
        assert!(f.take_cues().contains(&Cue::LevelUp { last: true }), "the final rung has its own fanfare");
        // Without a ladder a kill is just a kill.
        let mut f = Feel::new();
        f.on_happened(&happened(0, 0, 1), &Own { ladder: false, ..ALIVE });
        f.tick(1.0);
        assert_eq!(f.take_cues(), vec![Cue::Kill]);
    }

    #[test]
    fn damage_points_at_the_attacker_relative_to_where_we_look_and_fades() {
        let mut f = Feel::new();
        f.on_happened(&happened(0, 1, 0), &ALIVE);
        assert_eq!(f.take_cues(), vec![Cue::Hurt]);
        // The attacker is due east (bearing 90 degrees). Facing north he is on our right; facing east he is ahead; facing south, on the left.
        let a = f.fx(0.0);
        assert!((a.hurt_angle - std::f32::consts::FRAC_PI_2).abs() < 1e-5 && a.hurt_strength > 0.9, "{a:?}");
        assert!(f.fx(std::f32::consts::FRAC_PI_2).hurt_angle.abs() < 1e-5);
        assert!((f.fx(std::f32::consts::PI).hurt_angle + std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        assert!(a.vignette[3] > 0.4, "the screen edge reddens");
        f.tick(HURT_SECS * 0.5);
        assert!(f.fx(0.0).hurt_strength < 0.6 && f.fx(0.0).hurt_strength > 0.4);
        f.tick(HURT_SECS);
        assert!(!f.fx(0.0).active(), "everything faded");
    }

    #[test]
    fn low_health_pulses_red_and_beats_faster_as_health_falls() {
        let mut f = Feel::new();
        f.observe_own(&ALIVE);
        f.observe_own(&Own { hp: 30, ..ALIVE });
        let mut beats = 0;
        for _ in 0..600 {
            f.tick(1.0 / 60.0);
            beats += f.take_cues().iter().filter(|c| **c == Cue::Heartbeat).count();
        }
        assert!((9..=12).contains(&beats), "about one beat a second at 30 hp over ten seconds: {beats}");
        assert!(f.fx(0.0).vignette[3] > 0.05, "the edge is red");
        let mut g = Feel::new();
        g.observe_own(&ALIVE);
        g.observe_own(&Own { hp: 5, ..ALIVE });
        let mut fast = 0;
        for _ in 0..600 {
            g.tick(1.0 / 60.0);
            fast += g.take_cues().iter().filter(|c| **c == Cue::Heartbeat).count();
        }
        assert!(fast > beats + 2, "{fast} beats at 5 hp vs {beats} at 30");
        let mut healthy = Feel::new();
        healthy.observe_own(&ALIVE);
        healthy.tick(5.0);
        assert!(healthy.take_cues().is_empty() && !healthy.fx(0.0).active(), "a healthy player hears and sees nothing");
    }

    #[test]
    fn dying_and_coming_back_are_announced_once_and_tint_the_screen_while_dead() {
        let mut f = Feel::new();
        f.observe_own(&ALIVE);
        f.observe_own(&Own { hp: 0, dead: true, ..ALIVE });
        assert_eq!(f.take_cues(), vec![Cue::Death]);
        f.observe_own(&Own { hp: 0, dead: true, ..ALIVE });
        assert!(f.take_cues().is_empty(), "once");
        let dead = f.fx(0.0);
        assert!(dead.flash[3] >= 0.38 && dead.vignette[3] >= 0.85, "{dead:?}");
        f.observe_own(&ALIVE);
        assert_eq!(f.take_cues(), vec![Cue::Respawn]);
        assert!(f.fx(0.0).flash[3] > 0.3, "a flash of light on return");
        f.tick(1.0);
        assert!(!f.fx(0.0).active());
        // Raising a different weapon clicks, but not when the change is the spawn restoring our rung.
        f.observe_own(&Own { weapon: 7, ..ALIVE });
        assert_eq!(f.take_cues(), vec![Cue::Draw]);
    }

    #[test]
    fn the_countdown_beeps_each_second_then_goes_and_the_round_ends_with_a_sting() {
        let mut f = Feel::new();
        f.observe_match(Phase::Waiting, None, false);
        assert!(f.take_cues().is_empty());
        for s in [3, 3, 2, 2, 1] {
            f.observe_match(Phase::Countdown, Some(s), false);
        }
        assert_eq!(f.take_cues(), vec![Cue::Beep, Cue::Beep, Cue::Beep], "one beep per new second, not per frame");
        f.observe_match(Phase::Playing, Some(90), false);
        assert_eq!(f.take_cues(), vec![Cue::Go]);
        f.observe_match(Phase::Playing, Some(89), false);
        assert!(f.take_cues().is_empty());
        f.observe_match(Phase::Results, Some(20), true);
        assert_eq!(f.take_cues(), vec![Cue::Victory]);
        f.observe_match(Phase::Results, Some(19), true);
        assert!(f.take_cues().is_empty(), "the sting plays once");
        let mut g = Feel::new();
        g.observe_match(Phase::Playing, None, false);
        g.observe_match(Phase::Results, None, false);
        assert_eq!(g.take_cues(), vec![Cue::Defeat]);
    }

    #[test]
    fn movement_makes_footsteps_by_distance_jumps_landings_and_pad_launches() {
        let mut f = Feel::new();
        // Walking 5 m/s for two seconds on the ground: a stride is 1.9 m, so five steps.
        for _ in 0..120 {
            f.observe_motion(5.0, 1.0 / 60.0, 0.0, true, None, false);
        }
        let steps = f.take_cues().iter().filter(|c| **c == Cue::Step).count();
        assert_eq!(steps, 5, "10 m / 1.9 m");
        // Standing still: silence.
        for _ in 0..120 {
            f.observe_motion(0.0, 1.0 / 60.0, 0.0, true, None, false);
        }
        assert!(f.take_cues().is_empty());
        // A jump, then a hard landing.
        f.observe_motion(0.0, 1.0 / 60.0, 7.0, false, Some(15.0), false);
        assert_eq!(f.take_cues(), vec![Cue::Jump]);
        f.observe_motion(0.0, 1.0 / 60.0, -14.0, false, Some(15.0), false);
        f.observe_motion(0.0, 1.0 / 60.0, 0.0, true, Some(15.0), false);
        match f.take_cues().as_slice() {
            [Cue::Land(k)] => assert!(*k > 0.4, "a hard landing: {k}"),
            other => panic!("{other:?}"),
        }
        // A pad's launch is a pad, not a jump.
        let mut g = Feel::new();
        g.observe_motion(0.0, 1.0 / 60.0, 0.0, true, Some(15.0), false);
        g.observe_motion(0.0, 1.0 / 60.0, 17.0, false, Some(15.0), false);
        assert_eq!(g.take_cues(), vec![Cue::Pad]);
    }

    #[test]
    fn shots_heard_become_positioned_cues_and_our_own_are_flagged() {
        let mut f = Feel::new();
        let shot = ShotHeard { id: 3, weapon: 9, shots: 1, pos: Vec3::new(4.0, 0.0, -6.0), yaw: 0.0, pitch: 0.0 };
        f.on_happened(&Happenings { shots: vec![shot], ..Default::default() }, &ALIVE);
        f.own_shot(4, Vec3::ZERO);
        f.own_swing();
        // Ours play at once; another player's is heard when their avatar (drawn 100 ms late) fires.
        assert_eq!(f.take_cues(), vec![Cue::Shot { weapon: 4, at: Vec3::ZERO, yaw: 0.0, pitch: 0.0, shooter: 0, own: true }, Cue::Swing]);
        f.tick(REMOTE_SOUND_DELAY_SECS * 0.5);
        assert!(f.take_cues().is_empty(), "not yet");
        f.tick(REMOTE_SOUND_DELAY_SECS);
        assert_eq!(f.take_cues(), vec![Cue::Shot { weapon: 9, at: Vec3::new(4.0, 0.0, -6.0), yaw: 0.0, pitch: 0.0, shooter: 3, own: false }]);
    }

    #[test]
    fn a_siren_sounds_once_when_someone_else_reaches_the_last_rung() {
        let mut f = Feel::new();
        f.observe_threat(None);
        assert!(f.take_cues().is_empty());
        f.observe_threat(Some(3));
        f.observe_threat(Some(3));
        assert_eq!(f.take_cues(), vec![Cue::Alert], "once, not every frame");
        f.observe_threat(None);
        f.observe_threat(Some(3));
        assert_eq!(f.take_cues(), vec![Cue::Alert], "again if they fall back and return");
        f.observe_threat(Some(5));
        assert_eq!(f.take_cues(), vec![Cue::Alert], "a different player is a new threat");
    }

    #[test]
    fn spawn_protection_tints_the_screen_edge_and_reset_forgets_everything() {
        let mut f = Feel::new();
        f.observe_own(&Own { protected: true, ..ALIVE });
        f.tick(0.1);
        let v = f.fx(0.0).vignette;
        assert!(v[3] > 0.1 && v[2] > v[0], "a cool tint: {v:?}");
        f.on_happened(&happened(1, 1, 1), &ALIVE);
        f.reset();
        assert!(f.take_cues().is_empty() && !f.fx(0.0).active());
    }

    #[test]
    fn angles_wrap_into_a_half_turn_either_way() {
        for (a, want) in [(0.0, 0.0), (3.5, 3.5 - std::f32::consts::TAU), (-3.5, -3.5 + std::f32::consts::TAU), (7.0, 7.0 - std::f32::consts::TAU)] {
            assert!((wrap_pi(a) - want).abs() < 1e-5, "{a} -> {} (want {want})", wrap_pi(a));
        }
    }
}
