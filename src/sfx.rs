//! The game's sounds, designed in code (ADR 0008: nothing imported): one gun voice per firearm, the feedback cues that make shooting feel
//! like something (hit tick, kill ding, level-up jingle, hurt, death, respawn), movement and match stingers, and the maths that places a
//! sound in the stereo field.
//!
//! Every clip is a mono `Vec<f32>` at [`SAMPLE_RATE`], generated deterministically, finite, within -1..1 and fully decayed at its end (tests
//! check all of that, since a sound cannot be looked at). [`SoundBank`] builds them all once at start-up.
//!
//! **A gun** is four layers: a *crack* (high-passed noise, a few milliseconds), a *boom* (a sine sweeping down: the body of the shot), a
//! *body* (low-passed noise) and a *tail* (the room). Small automatic weapons have a thin crack, a little boom and almost no tail, so ten
//! shots a second do not smear; a shotgun is all boom and body; the scout is a hard crack and a long echo.

use crate::audio::SAMPLE_RATE;
use crate::feel::Cue;
use crate::weapons::Weapon;
use std::f32::consts::{PI, TAU};

/// A tiny xorshift so a noise burst needs no dependency and is the same every run.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

fn samples(seconds: f32) -> usize {
    (SAMPLE_RATE as f32 * seconds) as usize
}

fn time(i: usize) -> f32 {
    i as f32 / SAMPLE_RATE as f32
}

/// Exponential decay `e^(-t * rate)`.
fn decay(t: f32, rate: f32) -> f32 {
    (-t * rate).exp()
}

/// A click-free start: ramps in over about a third of a millisecond.
fn attack(t: f32) -> f32 {
    1.0 - decay(t, 9000.0)
}

/// Pushes a clip through a soft clipper and scales its peak to `level` (never above 0.98).
fn finish(mut clip: Vec<f32>, level: f32) -> Vec<f32> {
    for s in clip.iter_mut() {
        *s = (*s * 1.25).tanh();
    }
    let peak = clip.iter().fold(0.0f32, |m, s| m.max(s.abs())).max(1e-6);
    let k = level.min(0.98) / peak;
    for s in clip.iter_mut() {
        *s *= k;
    }
    // A 2 ms fade-out so nothing ends on a click.
    let fade = samples(0.002).min(clip.len());
    let n = clip.len();
    for j in 0..fade {
        clip[n - 1 - j] *= j as f32 / fade as f32;
    }
    clip
}

/// The numbers that make one firearm sound like itself.
#[derive(Debug, Clone, Copy)]
struct GunVoice {
    crack: f32,
    crack_rate: f32,
    boom_from: f32,
    boom_to: f32,
    boom_rate: f32,
    boom: f32,
    body: f32,
    body_lp: f32,
    body_rate: f32,
    tail: f32,
    tail_rate: f32,
    seconds: f32,
    level: f32,
    pump: bool,
}

fn voice(weapon: Weapon) -> Option<GunVoice> {
    let v = |crack, crack_rate, boom_from, boom_to, boom_rate, boom, body, body_lp, body_rate, tail, tail_rate, seconds, level| GunVoice {
        crack,
        crack_rate,
        boom_from,
        boom_to,
        boom_rate,
        boom,
        body,
        body_lp,
        body_rate,
        tail,
        tail_rate,
        seconds,
        level,
        pump: false,
    };
    Some(match weapon {
        Weapon::Pistol => v(0.9, 420.0, 190.0, 120.0, 22.0, 0.55, 0.5, 0.22, 45.0, 0.20, 9.0, 0.45, 0.62),
        Weapon::MachinePistol => v(1.0, 520.0, 210.0, 150.0, 40.0, 0.35, 0.45, 0.30, 70.0, 0.10, 18.0, 0.20, 0.42),
        Weapon::Smg => v(0.95, 480.0, 175.0, 110.0, 34.0, 0.45, 0.5, 0.25, 60.0, 0.12, 16.0, 0.22, 0.44),
        Weapon::Carbine => v(1.0, 380.0, 150.0, 85.0, 20.0, 0.7, 0.55, 0.20, 38.0, 0.30, 8.0, 0.50, 0.52),
        Weapon::Rifle => v(1.0, 330.0, 130.0, 70.0, 16.0, 0.85, 0.6, 0.18, 30.0, 0.40, 6.5, 0.60, 0.55),
        Weapon::Bullpup => v(1.0, 400.0, 145.0, 90.0, 19.0, 0.75, 0.55, 0.21, 36.0, 0.32, 8.0, 0.55, 0.50),
        Weapon::Marksman => v(1.0, 260.0, 110.0, 55.0, 11.0, 0.95, 0.65, 0.16, 24.0, 0.55, 4.5, 0.85, 0.60),
        Weapon::Shotgun => GunVoice { pump: true, ..v(0.85, 200.0, 95.0, 45.0, 9.0, 1.1, 0.9, 0.14, 16.0, 0.60, 4.0, 0.95, 0.68) },
        Weapon::Lmg => v(0.95, 340.0, 100.0, 60.0, 17.0, 0.85, 0.6, 0.17, 28.0, 0.25, 9.0, 0.40, 0.50),
        Weapon::Scout => v(1.0, 220.0, 90.0, 40.0, 8.0, 1.0, 0.6, 0.15, 20.0, 0.80, 3.2, 1.20, 0.62),
        // The bat swings and the revolver keeps the engine's original tuned shot; neither is built from a voice.
        Weapon::Bat | Weapon::Revolver => return None,
    })
}

/// The report of `weapon` being fired: its own voice, except the revolver (the engine's original synth) and the bat (whose "shot" is the
/// swing, see [`bat_swing`] and [`bat_hit`]).
pub fn gun_shot(weapon: Weapon) -> Vec<f32> {
    let Some(g) = voice(weapon) else {
        return if weapon == Weapon::Bat { bat_swing() } else { crate::audio::synth_revolver_shot() };
    };
    let n = samples(g.seconds);
    let mut noise = Noise(0x9E37_79B9 ^ (weapon.wire() as u32 + 1).wrapping_mul(0x85EB_CA6B));
    let (mut lp_body, mut lp_tail, mut hp_lp) = (0.0f32, 0.0f32, 0.0f32);
    let mut clip = Vec::with_capacity(n);
    for i in 0..n {
        let t = time(i);
        let white = noise.next();
        hp_lp += 0.5 * (white - hp_lp);
        let crack = (white - hp_lp) * decay(t, g.crack_rate) * g.crack;
        // The boom sweeps quickly down from `boom_from` to `boom_to`: the integral of the frequency is the phase.
        let sweep = (t * 7.0).min(1.0);
        let freq = g.boom_from + (g.boom_to - g.boom_from) * sweep;
        let boom = (TAU * freq * t).sin() * decay(t, g.boom_rate) * g.boom;
        lp_body += g.body_lp * (white - lp_body);
        let body = lp_body * decay(t, g.body_rate) * g.body * 2.2;
        lp_tail += 0.045 * (white - lp_tail);
        let tail = lp_tail * decay(t, g.tail_rate) * g.tail * 3.0 * (1.0 - decay(t, 70.0));
        let mut s = (crack + boom + body + tail) * attack(t);
        if g.pump {
            // Racking the pump: two dry clacks after the blast.
            for start in [0.46, 0.55] {
                let u = t - start;
                if u >= 0.0 {
                    s += (noise.next() * 0.5 + (TAU * 1800.0 * u).sin() * 0.5) * decay(u, 260.0) * 0.28;
                }
            }
        }
        clip.push(s);
    }
    finish(clip, g.level)
}

/// The bat coming round: a soft rush of filtered noise that opens up and closes again.
pub fn bat_swing() -> Vec<f32> {
    let n = samples(0.24);
    let mut noise = Noise(0x0BAD_5EED);
    let mut lp = 0.0f32;
    let clip = (0..n)
        .map(|i| {
            let t = time(i);
            let open = (PI * t / 0.24).sin().powi(2);
            lp += (0.05 + 0.5 * open) * (noise.next() - lp);
            lp * open * 2.0
        })
        .collect();
    finish(clip, 0.32)
}

/// The bat landing on a person: a low knock with a dry ring (louder and duller than hitting a prop).
pub fn bat_hit() -> Vec<f32> {
    let n = samples(0.24);
    let mut noise = Noise(0xB47);
    let mut lp = 0.0f32;
    let clip = (0..n)
        .map(|i| {
            let t = time(i);
            lp += 0.3 * (noise.next() - lp);
            let thump = (TAU * (85.0 - 25.0 * t) * t).sin() * decay(t, 20.0);
            let ring = (TAU * 420.0 * t).sin() * decay(t, 40.0) * 0.4;
            (thump + ring + lp * decay(t, 90.0)) * attack(t)
        })
        .collect();
    finish(clip, 0.8)
}

/// A sine partial with an exponential decay, the unit a bell is built from.
fn partial(freq: f32, t: f32, rate: f32) -> f32 {
    (TAU * freq * t).sin() * decay(t, rate)
}

/// The tick of a shot that landed: short, bright, unmistakable, and quiet enough to hear a hundred of.
pub fn hit_tick() -> Vec<f32> {
    let n = samples(0.07);
    let mut noise = Noise(0x71C6);
    let clip = (0..n)
        .map(|i| {
            let t = time(i);
            (partial(2100.0, t, 60.0) * 0.6 + partial(3300.0, t, 90.0) * 0.4 + noise.next() * decay(t, 400.0) * 0.25) * attack(t)
        })
        .collect();
    finish(clip, 0.45)
}

/// A kill: a chest thump under a two-note bell.
pub fn kill_ding() -> Vec<f32> {
    let n = samples(0.75);
    let clip = (0..n)
        .map(|i| {
            let t = time(i);
            let thump = partial(95.0, t, 28.0) * 0.9;
            let bell = partial(1046.5, t, 7.0) * 0.55 + partial(1568.0, t, 9.0) * 0.35 + partial(2093.0 * 1.0, t, 14.0) * 0.18;
            let second = if t > 0.09 { partial(1318.5, t - 0.09, 7.0) * 0.5 } else { 0.0 };
            (thump + bell + second) * attack(t)
        })
        .collect();
    finish(clip, 0.6)
}

/// Up a rung: a quick rising arpeggio with a shimmer. `final_rung` adds a fifth note and a longer ring (the last weapon).
pub fn level_up(final_rung: bool) -> Vec<f32> {
    let notes: &[f32] = if final_rung { &[523.25, 659.25, 783.99, 1046.5, 1318.5] } else { &[523.25, 659.25, 783.99, 1046.5] };
    let step = 0.065;
    let n = samples(step * notes.len() as f32 + if final_rung { 0.9 } else { 0.5 });
    let clip = (0..n)
        .map(|i| {
            let t = time(i);
            notes
                .iter()
                .enumerate()
                .map(|(k, f)| {
                    let u = t - k as f32 * step;
                    if u < 0.0 {
                        0.0
                    } else {
                        let tri = (TAU * f * u).sin() * 0.7 + (TAU * f * 2.0 * u).sin() * 0.2;
                        tri * decay(u, if final_rung { 4.0 } else { 7.0 }) * (1.0 - decay(u, 400.0))
                    }
                })
                .sum::<f32>()
        })
        .collect();
    finish(clip, if final_rung { 0.62 } else { 0.5 })
}

/// Taking damage: a dull thud with a rasp, low enough not to be mistaken for anything you did.
pub fn hurt() -> Vec<f32> {
    let n = samples(0.28);
    let mut noise = Noise(0x4172);
    let mut lp = 0.0f32;
    let clip = (0..n)
        .map(|i| {
            let t = time(i);
            lp += 0.12 * (noise.next() - lp);
            (partial(70.0, t, 16.0) * 0.9 + lp * decay(t, 30.0) * 1.4 + partial(190.0, t, 30.0) * 0.3) * attack(t)
        })
        .collect();
    finish(clip, 0.66)
}

/// Dying: a falling tone and a rumble.
pub fn death() -> Vec<f32> {
    let n = samples(1.1);
    let mut noise = Noise(0xDEAD);
    let mut lp = 0.0f32;
    let mut phase = 0.0f32;
    let clip = (0..n)
        .map(|i| {
            let t = time(i);
            let f = 320.0 * decay(t, 2.4) + 45.0;
            phase += TAU * f / SAMPLE_RATE as f32;
            lp += 0.03 * (noise.next() - lp);
            (phase.sin() * 0.7 * decay(t, 4.5) + lp * 3.0 * decay(t, 5.0)) * attack(t)
        })
        .collect();
    finish(clip, 0.6)
}

/// Coming back: a rising whoosh.
pub fn respawn() -> Vec<f32> {
    let n = samples(0.45);
    let mut noise = Noise(0x5EED5);
    let mut lp = 0.0f32;
    let mut phase = 0.0f32;
    let clip = (0..n)
        .map(|i| {
            let t = time(i);
            let rise = t / 0.45;
            phase += TAU * (180.0 + 900.0 * rise * rise) / SAMPLE_RATE as f32;
            lp += (0.02 + 0.5 * rise) * (noise.next() - lp);
            (phase.sin() * 0.35 + lp * 1.2) * (PI * rise).sin().powf(1.5)
        })
        .collect();
    finish(clip, 0.5)
}

/// A jump pad: a springy upward chirp.
pub fn pad_launch() -> Vec<f32> {
    let n = samples(0.45);
    let mut phase = 0.0f32;
    let mut noise = Noise(0x0BAD);
    let clip = (0..n)
        .map(|i| {
            let t = time(i);
            phase += TAU * (160.0 + 1500.0 * (1.0 - decay(t, 9.0))) / SAMPLE_RATE as f32;
            (phase.sin() * 0.8 + noise.next() * 0.12 * decay(t, 20.0)) * decay(t, 11.0) * attack(t)
        })
        .collect();
    finish(clip, 0.5)
}

/// A siren: three swells alternating between two tones, for "someone is one kill from winning".
pub fn alert() -> Vec<f32> {
    let n = samples(1.0);
    let clip = (0..n)
        .map(|i| {
            let t = time(i);
            let pulse = (t / 0.32).floor();
            let u = t - pulse * 0.32;
            let f = if pulse as i32 % 2 == 0 { 660.0 } else { 880.0 };
            let env = (u / 0.02).min(1.0) * decay(u, 3.0) * if t < 0.96 { 1.0 } else { (1.0 - t) / 0.04 };
            ((TAU * f * t).sin() * 0.6 + (TAU * f * 2.0 * t).sin() * 0.15) * env
        })
        .collect();
    finish(clip, 0.5)
}

/// A short beep of `freq` Hz lasting `seconds` (the countdown's 3, 2, 1).
pub fn beep(freq: f32, seconds: f32) -> Vec<f32> {
    let n = samples(seconds);
    let clip = (0..n)
        .map(|i| {
            let t = time(i);
            let release = ((seconds - t) / (0.35 * seconds)).clamp(0.0, 1.0);
            (TAU * freq * t).sin() * (t / 0.004).min(1.0) * release
        })
        .collect();
    finish(clip, 0.42)
}

/// The end of the countdown: a higher, longer note.
pub fn go() -> Vec<f32> {
    beep(1320.0, 0.42)
}

/// A win: a bright rising fanfare.
pub fn victory() -> Vec<f32> {
    chord_run(&[523.25, 659.25, 783.99, 1046.5, 1318.5, 1568.0], 0.11, 1.7, 4.5, 0.62)
}

/// A loss: a slow falling minor line.
pub fn defeat() -> Vec<f32> {
    chord_run(&[440.0, 392.0, 349.23, 293.66], 0.2, 1.6, 4.5, 0.5)
}

fn chord_run(notes: &[f32], step: f32, seconds: f32, rate: f32, level: f32) -> Vec<f32> {
    let n = samples(seconds);
    let clip = (0..n)
        .map(|i| {
            let t = time(i);
            notes
                .iter()
                .enumerate()
                .map(|(k, f)| {
                    let u = t - k as f32 * step;
                    if u < 0.0 {
                        0.0
                    } else {
                        ((TAU * f * u).sin() * 0.6 + (TAU * f * 2.0 * u).sin() * 0.15 + (TAU * f * 0.5 * u).sin() * 0.2)
                            * decay(u, rate)
                            * (1.0 - decay(u, 300.0))
                    }
                })
                .sum::<f32>()
        })
        .collect();
    finish(clip, level)
}

/// One heartbeat ("lub-dub"), for low health.
pub fn heartbeat() -> Vec<f32> {
    let n = samples(0.55);
    let clip = (0..n)
        .map(|i| {
            let t = time(i);
            let lub = partial(58.0, t, 22.0);
            let dub = if t > 0.17 { partial(52.0, t - 0.17, 26.0) * 0.7 } else { 0.0 };
            (lub + dub) * attack(t)
        })
        .collect();
    finish(clip, 0.55)
}

/// A footstep: a soft thud and a scuff of noise. `variant` picks between two slightly different feet.
pub fn footstep(variant: u32) -> Vec<f32> {
    let n = samples(0.1);
    let mut noise = Noise(0xF007 + variant * 7919);
    let mut lp = 0.0f32;
    let clip = (0..n)
        .map(|i| {
            let t = time(i);
            lp += (0.16 + 0.03 * variant as f32) * (noise.next() - lp);
            (partial(72.0 + 9.0 * variant as f32, t, 48.0) * 0.55 + lp * decay(t, 42.0) * 1.6) * attack(t)
        })
        .collect();
    finish(clip, 0.5)
}

/// Landing: a heavier thud than a step.
pub fn landing() -> Vec<f32> {
    let n = samples(0.2);
    let mut noise = Noise(0x1A2D);
    let mut lp = 0.0f32;
    let clip = (0..n)
        .map(|i| {
            let t = time(i);
            lp += 0.1 * (noise.next() - lp);
            (partial(58.0, t, 20.0) * 1.0 + lp * decay(t, 24.0) * 2.0) * attack(t)
        })
        .collect();
    finish(clip, 0.7)
}

/// Pushing off: a short breath of air.
pub fn jump() -> Vec<f32> {
    let n = samples(0.16);
    let mut noise = Noise(0x0A1B);
    let mut lp = 0.0f32;
    let clip = (0..n)
        .map(|i| {
            let t = time(i);
            let open = (PI * t / 0.16).sin().powi(2);
            lp += (0.08 + 0.3 * open) * (noise.next() - lp);
            lp * open * 2.5
        })
        .collect();
    finish(clip, 0.3)
}

/// Where a sound at `source` (x, y, z) lands in the stereo field for a listener at `listener` looking along `yaw` (the engine's yaw: 0 is -Z,
/// clockwise from above): `(gain, pan)` with `pan` from -1 (hard left) to 1 (hard right). Sounds fall off with distance and are a little duller
/// behind you, so a shot from behind is quieter than the same shot in front.
pub fn spatial(listener: [f32; 3], yaw: f32, source: [f32; 3]) -> (f32, f32) {
    let d = [source[0] - listener[0], source[1] - listener[1], source[2] - listener[2]];
    let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    let gain = (1.0 / (1.0 + (dist / 9.0).powf(1.6))).max(0.04);
    if dist < 0.5 {
        return (1.0, 0.0);
    }
    let bearing = d[0].atan2(-d[2]);
    let mut rel = (bearing - yaw) % TAU;
    if rel > PI {
        rel -= TAU;
    } else if rel < -PI {
        rel += TAU;
    }
    let behind = 0.65 + 0.35 * (rel / 2.0).cos().max(0.0);
    (gain * behind, rel.sin())
}

/// The `(left, right)` channel gains for a `gain` and a `pan` in -1..1 (constant power: hard left or right is full level in one ear,
/// centre is about 0.7 in both).
pub fn pan_gains(gain: f32, pan: f32) -> (f32, f32) {
    let a = (pan.clamp(-1.0, 1.0) + 1.0) * PI / 4.0;
    (gain * a.cos(), gain * a.sin())
}

/// Every sound the client plays, built once.
pub struct SoundBank {
    /// The shot of each weapon, indexed by [`Weapon::wire`] (the bat's slot holds its swing).
    pub guns: Vec<Vec<f32>>,
    /// The bat landing on a person.
    pub bat_hit: Vec<f32>,
    /// A shot of ours landed.
    pub hit_tick: Vec<f32>,
    /// We killed someone.
    pub kill: Vec<f32>,
    /// We moved up the ladder.
    pub level_up: Vec<f32>,
    /// We reached the last rung.
    pub level_final: Vec<f32>,
    /// We took damage.
    pub hurt: Vec<f32>,
    /// We died.
    pub death: Vec<f32>,
    /// We are back.
    pub respawn: Vec<f32>,
    /// A jump pad fired.
    pub pad: Vec<f32>,
    /// A countdown tick.
    pub beep: Vec<f32>,
    /// The countdown reached zero.
    pub go: Vec<f32>,
    /// We won the round.
    pub victory: Vec<f32>,
    /// We lost it.
    pub defeat: Vec<f32>,
    /// A heartbeat, played while health is low.
    pub heartbeat: Vec<f32>,
    /// Two footsteps, alternated.
    pub steps: [Vec<f32>; 2],
    /// A landing.
    pub land: Vec<f32>,
    /// A jump.
    pub jump: Vec<f32>,
    /// A weapon being raised.
    pub draw: Vec<f32>,
    /// Someone is one kill from winning.
    pub alert: Vec<f32>,
}

impl SoundBank {
    /// Builds every clip (a few milliseconds of arithmetic).
    pub fn new() -> SoundBank {
        let guns = Weapon::ALL.iter().map(|w| gun_shot(*w)).collect();
        SoundBank {
            guns,
            bat_hit: bat_hit(),
            hit_tick: hit_tick(),
            kill: kill_ding(),
            level_up: level_up(false),
            level_final: level_up(true),
            hurt: hurt(),
            death: death(),
            respawn: respawn(),
            pad: pad_launch(),
            beep: beep(880.0, 0.11),
            go: go(),
            victory: victory(),
            defeat: defeat(),
            heartbeat: heartbeat(),
            steps: [footstep(0), footstep(1)],
            land: landing(),
            jump: jump(),
            draw: crate::audio::synth_weapon_click(),
            alert: alert(),
        }
    }

    /// The shot sound of the weapon with wire id `weapon`.
    pub fn gun(&self, weapon: u8) -> &[f32] {
        self.guns.get(weapon as usize).map_or(&self.guns[0][..], Vec::as_slice)
    }

    /// The sound for `cue` as heard by `listener`: which clip, how loud, and where in the stereo field. `variant` picks between alternates
    /// (which foot), so a caller counts steps. Guns fired by others fall off with distance and pan to their side; everything about us is
    /// centred.
    pub fn play(&self, cue: &Cue, listener: Listener, variant: u32) -> Played<'_> {
        fn centred(clip: &[f32], gain: f32) -> Played<'_> {
            Played { clip, gain, pan: 0.0 }
        }
        match *cue {
            Cue::Shot { weapon, own: true, .. } => centred(self.gun(weapon), 0.95),
            Cue::Shot { weapon, at, own: false, .. } => {
                let (gain, pan) = spatial(listener.eye, listener.yaw, [at.x, at.y + 1.4, at.z]);
                Played { clip: self.gun(weapon), gain: gain * 0.95, pan }
            }
            Cue::Swing => centred(&self.guns[0], 0.6),
            Cue::Hit { bat: false } => centred(&self.hit_tick, 0.7),
            Cue::Hit { bat: true } => centred(&self.bat_hit, 0.85),
            Cue::Kill => centred(&self.kill, 0.8),
            Cue::LevelUp { last: false } => centred(&self.level_up, 0.7),
            Cue::LevelUp { last: true } => centred(&self.level_final, 0.85),
            Cue::Hurt => centred(&self.hurt, 0.9),
            Cue::Death => centred(&self.death, 0.9),
            Cue::Respawn => centred(&self.respawn, 0.7),
            Cue::Pad => centred(&self.pad, 0.7),
            Cue::Jump => centred(&self.jump, 0.5),
            Cue::Land(k) => centred(&self.land, 0.2 + 0.7 * k.clamp(0.0, 1.0)),
            Cue::Step => centred(&self.steps[(variant % 2) as usize], 0.32),
            Cue::Beep => centred(&self.beep, 0.6),
            Cue::Go => centred(&self.go, 0.7),
            Cue::Victory => centred(&self.victory, 0.8),
            Cue::Defeat => centred(&self.defeat, 0.7),
            Cue::Heartbeat => centred(&self.heartbeat, 0.6),
            Cue::Draw => centred(&self.draw, 0.5),
            Cue::Alert => centred(&self.alert, 0.75),
        }
    }
}

/// Where the listener stands: the eye position (x, y, z) and the yaw it looks along.
#[derive(Debug, Clone, Copy)]
pub struct Listener {
    /// Eye position.
    pub eye: [f32; 3],
    /// Look yaw, radians (0 looks along -Z).
    pub yaw: f32,
}

/// A sound chosen for a cue: the clip, its loudness (0..1) and where it sits in the stereo field (-1 left .. 1 right).
#[derive(Debug, Clone, Copy)]
pub struct Played<'a> {
    /// Mono samples at [`SAMPLE_RATE`].
    pub clip: &'a [f32],
    /// Loudness.
    pub gain: f32,
    /// Stereo position.
    pub pan: f32,
}

impl Default for SoundBank {
    fn default() -> Self {
        SoundBank::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak(c: &[f32]) -> f32 {
        c.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    fn secs(c: &[f32]) -> f32 {
        c.len() as f32 / SAMPLE_RATE as f32
    }

    #[test]
    fn every_sound_is_finite_audible_bounded_and_ends_in_silence() {
        let bank = SoundBank::new();
        let mut all: Vec<(String, &Vec<f32>)> = bank.guns.iter().enumerate().map(|(i, c)| (format!("gun {i}"), c)).collect();
        all.extend([
            ("bat_hit".to_string(), &bank.bat_hit),
            ("hit_tick".to_string(), &bank.hit_tick),
            ("kill".to_string(), &bank.kill),
            ("level_up".to_string(), &bank.level_up),
            ("level_final".to_string(), &bank.level_final),
            ("hurt".to_string(), &bank.hurt),
            ("death".to_string(), &bank.death),
            ("respawn".to_string(), &bank.respawn),
            ("pad".to_string(), &bank.pad),
            ("beep".to_string(), &bank.beep),
            ("go".to_string(), &bank.go),
            ("victory".to_string(), &bank.victory),
            ("defeat".to_string(), &bank.defeat),
            ("heartbeat".to_string(), &bank.heartbeat),
            ("step 0".to_string(), &bank.steps[0]),
            ("step 1".to_string(), &bank.steps[1]),
            ("land".to_string(), &bank.land),
            ("jump".to_string(), &bank.jump),
            ("draw".to_string(), &bank.draw),
            ("alert".to_string(), &bank.alert),
        ]);
        for (name, clip) in all {
            assert!(clip.iter().all(|s| s.is_finite() && s.abs() <= 1.0), "{name}: samples must be finite and within -1..1");
            let p = peak(clip);
            assert!((0.2..=0.99).contains(&p), "{name}: peak {p} should be audible but not clipping");
            let tail = peak(&clip[clip.len().saturating_sub(64)..]);
            assert!(tail < 0.02, "{name}: ends in silence, not a click: {tail}");
            assert!(secs(clip) >= 0.04 && secs(clip) < 2.0, "{name}: {} s", secs(clip));
        }
    }

    #[test]
    fn a_shotgun_and_a_scout_ring_out_longer_than_an_smg_and_the_smg_does_not_smear() {
        let (smg, shotgun, scout, mp) = (gun_shot(Weapon::Smg), gun_shot(Weapon::Shotgun), gun_shot(Weapon::Scout), gun_shot(Weapon::MachinePistol));
        assert!(secs(&shotgun) > secs(&smg) * 3.0 && secs(&scout) > secs(&smg) * 4.0);
        // Automatic weapons fire ~10 shots a second: by 100 ms the previous shot must be nearly gone.
        for (name, clip) in [("smg", &smg), ("machine pistol", &mp)] {
            let late = peak(&clip[samples(0.1).min(clip.len() - 1)..]);
            assert!(late < 0.12, "{name}: {late} still audible 100 ms after the shot");
        }
        // The sound of a shotgun carries more energy than an SMG round even after both are peak-normalised.
        let energy = |c: &[f32]| c.iter().map(|s| s * s).sum::<f32>();
        assert!(energy(&shotgun) > energy(&smg) * 3.0);
    }

    #[test]
    fn sounds_are_deterministic_and_each_weapon_has_its_own() {
        assert_eq!(gun_shot(Weapon::Rifle), gun_shot(Weapon::Rifle));
        let a = gun_shot(Weapon::Pistol);
        let b = gun_shot(Weapon::Carbine);
        assert_ne!(a.len(), b.len(), "different weapons are different sounds");
        let bank = SoundBank::new();
        assert_eq!(bank.guns.len(), Weapon::ALL.len());
        assert_eq!(bank.gun(Weapon::Shotgun.wire()), &bank.guns[Weapon::Shotgun.wire() as usize][..]);
        assert_eq!(bank.gun(200), &bank.guns[0][..], "an unknown weapon id falls back to the bat");
    }

    #[test]
    fn the_last_rung_rings_longer_than_an_ordinary_level_up() {
        assert!(secs(&level_up(true)) > secs(&level_up(false)) + 0.3);
    }

    #[test]
    fn spatial_sound_falls_off_pans_to_the_right_side_and_is_quieter_behind() {
        let here = [0.0, 1.7, 0.0];
        let (near, _) = spatial(here, 0.0, [0.0, 1.7, -3.0]);
        let (far, _) = spatial(here, 0.0, [0.0, 1.7, -40.0]);
        assert!(near > far * 3.0 && far >= 0.04, "{near} vs {far}");
        // Facing -Z (yaw 0): a source to the +X side is on the right, to -X on the left, straight ahead in the middle.
        assert!(spatial(here, 0.0, [10.0, 1.7, 0.0]).1 > 0.9);
        assert!(spatial(here, 0.0, [-10.0, 1.7, 0.0]).1 < -0.9);
        assert!(spatial(here, 0.0, [0.0, 1.7, -10.0]).1.abs() < 0.01);
        // Turn to face +X (yaw 90 degrees): the same +X source is now straight ahead, and a -Z source is on the left.
        assert!(spatial(here, PI / 2.0, [10.0, 1.7, 0.0]).1.abs() < 0.01);
        assert!(spatial(here, PI / 2.0, [0.0, 1.7, -10.0]).1 < -0.9);
        let (front, _) = spatial(here, 0.0, [0.0, 1.7, -10.0]);
        let (back, _) = spatial(here, 0.0, [0.0, 1.7, 10.0]);
        assert!(back < front * 0.75 && back > front * 0.5, "behind is quieter, not silent: {back} vs {front}");
        assert_eq!(spatial(here, 0.0, here), (1.0, 0.0), "a sound at the listener is centred and full");
    }

    #[test]
    fn every_cue_has_a_sound_and_others_shots_are_placed_while_ours_are_centred() {
        use glam::Vec3;
        let bank = SoundBank::new();
        let me = Listener { eye: [0.0, 1.7, 0.0], yaw: 0.0 };
        let cues = [
            Cue::Swing,
            Cue::Hit { bat: false },
            Cue::Hit { bat: true },
            Cue::Kill,
            Cue::LevelUp { last: false },
            Cue::LevelUp { last: true },
            Cue::Hurt,
            Cue::Death,
            Cue::Respawn,
            Cue::Pad,
            Cue::Jump,
            Cue::Land(0.5),
            Cue::Step,
            Cue::Beep,
            Cue::Go,
            Cue::Victory,
            Cue::Defeat,
            Cue::Heartbeat,
            Cue::Draw,
            Cue::Alert,
        ];
        for cue in cues {
            let p = bank.play(&cue, me, 0);
            assert!(!p.clip.is_empty() && (0.1..=1.0).contains(&p.gain) && p.pan == 0.0, "{cue:?}: gain {} pan {}", p.gain, p.pan);
        }
        let own = bank.play(&Cue::Shot { weapon: 4, at: Vec3::new(30.0, 0.0, 0.0), yaw: 0.0, pitch: 0.0, shooter: 0, own: true }, me, 0);
        assert_eq!((own.pan, own.gain), (0.0, 0.95), "our own gun is centred and full, wherever the cue says it was");
        let near = bank.play(&Cue::Shot { weapon: 4, at: Vec3::new(5.0, 0.0, 0.0), yaw: 0.0, pitch: 0.0, shooter: 1, own: false }, me, 0);
        let far = bank.play(&Cue::Shot { weapon: 4, at: Vec3::new(40.0, 0.0, 0.0), yaw: 0.0, pitch: 0.0, shooter: 1, own: false }, me, 0);
        assert!(near.pan > 0.9 && far.pan > 0.9, "a gun to the east is on the right");
        assert!(near.gain > far.gain * 2.0, "and quieter when far: {} vs {}", near.gain, far.gain);
        assert_ne!(bank.play(&Cue::Step, me, 0).clip, bank.play(&Cue::Step, me, 1).clip, "two alternating feet");
        assert!(bank.play(&Cue::Land(1.0), me, 0).gain > bank.play(&Cue::Land(0.0), me, 0).gain * 2.0, "harder landings are louder");
    }

    #[test]
    fn panning_is_constant_power_and_hard_pan_is_one_sided() {
        let (l, r) = pan_gains(1.0, -1.0);
        assert!((l - 1.0).abs() < 1e-5 && r.abs() < 1e-5);
        let (l, r) = pan_gains(1.0, 1.0);
        assert!(l.abs() < 1e-5 && (r - 1.0).abs() < 1e-5);
        for pan in [-0.7f32, -0.2, 0.0, 0.4, 0.9] {
            let (l, r) = pan_gains(0.8, pan);
            assert!(((l * l + r * r).sqrt() - 0.8).abs() < 1e-4, "power is constant at pan {pan}");
        }
    }
}
