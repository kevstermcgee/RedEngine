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

use crate::dsp::{attack, decay, finish, partial, samples, time, Env, Filter, Layer, Noise, Src, Voice};
use crate::feel::Cue;
use crate::synth::SAMPLE_RATE;
use crate::weapons::Weapon;
use std::f32::consts::{PI, TAU};

/// Every sound that is built as a [`Voice`], by catalog name (`gun.<weapon>`, `fx.<cue>`): what `audio export` prints and the JSON form round-trips.
pub fn voice_catalog() -> Vec<(String, Voice)> {
    let mut v = Vec::new();
    for w in Weapon::ROSTER {
        if let Some(g) = voice(w) {
            v.push((format!("gun.{}", w.name().to_lowercase().replace(' ', "_")), gun_voice(w, &g)));
        }
    }
    let fx: [(&str, Voice); 16] = [
        ("bat_swing", bat_swing_voice()),
        ("bat_hit", bat_hit_voice()),
        ("hit_tick", hit_tick_voice()),
        ("kill_ding", kill_ding_voice()),
        ("hurt", hurt_voice()),
        ("death", death_voice()),
        ("pad_launch", pad_launch_voice()),
        ("victory", victory_voice()),
        ("defeat", defeat_voice()),
        ("heartbeat", heartbeat_voice()),
        ("landing", landing_voice()),
        ("jump", jump_voice()),
        ("throw_whoosh", throw_whoosh_voice()),
        ("level_up", level_up_voice(false)),
        ("level_up_final", level_up_voice(true)),
        ("footstep", footstep_voice(0)),
    ];
    v.extend(fx.into_iter().map(|(n, voice)| (format!("fx.{n}"), voice)));
    v
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
        Weapon::Bulldog => v(0.95, 360.0, 160.0, 100.0, 20.0, 0.7, 0.55, 0.20, 40.0, 0.25, 8.0, 0.50, 0.60),
        Weapon::HandCannon => v(1.0, 240.0, 120.0, 60.0, 12.0, 1.0, 0.7, 0.15, 24.0, 0.60, 4.5, 0.90, 0.70),
        Weapon::Marshal => v(1.0, 260.0, 130.0, 65.0, 13.0, 0.95, 0.65, 0.16, 26.0, 0.50, 5.0, 0.80, 0.66),
        Weapon::Stinger => v(0.95, 440.0, 170.0, 105.0, 30.0, 0.5, 0.5, 0.25, 55.0, 0.14, 14.0, 0.25, 0.46),
        Weapon::Ranger => v(1.0, 600.0, 230.0, 170.0, 45.0, 0.3, 0.4, 0.35, 80.0, 0.10, 20.0, 0.18, 0.38),
        Weapon::Wasp => v(0.95, 520.0, 200.0, 140.0, 40.0, 0.35, 0.45, 0.30, 70.0, 0.10, 18.0, 0.18, 0.40),
        Weapon::Ironside => v(1.0, 280.0, 120.0, 60.0, 13.0, 0.95, 0.62, 0.17, 26.0, 0.50, 5.0, 0.70, 0.58),
        Weapon::Gale => v(1.0, 240.0, 100.0, 50.0, 10.0, 0.98, 0.65, 0.15, 22.0, 0.60, 4.2, 0.90, 0.60),
        Weapon::Sentinel => v(1.0, 200.0, 80.0, 35.0, 7.0, 1.2, 0.7, 0.13, 16.0, 0.90, 3.0, 1.40, 0.72),
        Weapon::Auto12 => v(0.85, 210.0, 100.0, 48.0, 10.0, 1.0, 0.85, 0.15, 17.0, 0.55, 4.2, 0.80, 0.64),
        Weapon::Coach => v(0.8, 180.0, 85.0, 38.0, 8.0, 1.3, 1.0, 0.12, 14.0, 0.70, 3.5, 1.00, 0.78),
        Weapon::Hammer => v(0.95, 320.0, 95.0, 55.0, 16.0, 0.85, 0.6, 0.17, 27.0, 0.30, 8.0, 0.42, 0.50),
        Weapon::Lancer => v(0.4, 120.0, 70.0, 32.0, 6.0, 1.2, 1.0, 0.10, 10.0, 0.90, 2.5, 1.50, 0.74),
        Weapon::Thumper => v(0.5, 160.0, 90.0, 50.0, 10.0, 1.0, 0.8, 0.13, 18.0, 0.60, 4.0, 0.60, 0.60),
        // The melee weapons swing and the grenades are thrown; neither is built from a gun voice.
        Weapon::Bat | Weapon::Knife | Weapon::Hatchet | Weapon::Frag | Weapon::Flash | Weapon::Smoke | Weapon::Incendiary => return None,
    })
}

/// The report of `weapon` being fired: its own voice, except the bat (whose "shot" is the swing, see [`bat_swing`] and [`bat_hit`]).
pub fn gun_shot(weapon: Weapon) -> Vec<f32> {
    let Some(g) = voice(weapon) else { return bat_swing() };
    gun_voice(weapon, &g).render()
}

/// A gun as four layers over one shared noise: a *crack* (the noise through a high-pass), a *boom* (a sine gliding down), a *body* and a *tail* (the
/// noise through two low-passes). A pump-action adds two dry clacks after the blast.
fn gun_voice(weapon: Weapon, g: &GunVoice) -> Voice {
    let seed = 0x9E37_79B9 ^ (weapon.wire() as u32 + 1).wrapping_mul(0x85EB_CA6B);
    let mut v = Voice::new(g.seconds, g.level, seed)
        .with(Layer::noise(Filter::High(0.5), Env::decay(g.crack_rate), g.crack))
        .with(Layer::new(Src::Glide { from: g.boom_from, to: g.boom_to, rate: 7.0 }, Env::decay(g.boom_rate), g.boom))
        .with(Layer::noise(Filter::Low(g.body_lp), Env::decay(g.body_rate), g.body * 2.2))
        .with(Layer::noise(Filter::Low(0.045), Env::decay(g.tail_rate).fading_in(70.0), g.tail * 3.0));
    if g.pump {
        // Racking the pump: two dry clacks after the blast.
        for start in [0.46, 0.55] {
            v = v.with(Layer::noise(Filter::None, Env::decay(260.0).after(start), 0.14)).with(Layer::sine(1800.0, Env::decay(260.0).after(start), 0.14));
        }
    }
    v
}
/// The bat coming round: a soft rush of filtered noise that opens up and closes again.
pub fn bat_swing() -> Vec<f32> {
    bat_swing_voice().render()
}

/// The bat coming round: a soft rush of filtered noise that opens up and closes again.
pub fn bat_swing_voice() -> Voice {
    Voice::new(0.24, 0.32, 0x0BAD_5EED).without_attack().with(Layer::noise(
        Filter::LowOpen { closed: 0.05, open: 0.55 },
        Env::decay(0.0).swelling(0.12, 0.12),
        2.0,
    ))
}
/// The bat landing on a person: a low knock with a dry ring (louder and duller than hitting a prop).
pub fn bat_hit_voice() -> Voice {
    Voice::new(0.24, 0.8, 0xB47)
        .with(Layer::new(Src::Glide { from: 85.0, to: 60.0, rate: 1.0 }, Env::decay(20.0), 1.0))
        .with(Layer::sine(420.0, Env::decay(40.0), 0.4))
        .with(Layer::noise(Filter::Low(0.3), Env::decay(90.0), 1.0))
}

pub fn bat_hit() -> Vec<f32> {
    bat_hit_voice().render()
}
/// The tick of a shot that landed: short, bright, unmistakable, and quiet enough to hear a hundred of.
pub fn hit_tick_voice() -> Voice {
    Voice::new(0.07, 0.45, 0x71C6).with(Layer::sine(2100.0, Env::decay(60.0), 0.6)).with(Layer::sine(3300.0, Env::decay(90.0), 0.4)).with(Layer::noise(
        Filter::None,
        Env::decay(400.0),
        0.25,
    ))
}

pub fn hit_tick() -> Vec<f32> {
    hit_tick_voice().render()
}
/// A kill: a chest thump under a two-note bell.
pub fn kill_ding_voice() -> Voice {
    Voice::new(0.75, 0.6, 0)
        .with(Layer::sine(95.0, Env::decay(28.0), 0.9))
        .with(Layer::sine(1046.5, Env::decay(7.0), 0.55))
        .with(Layer::sine(1568.0, Env::decay(9.0), 0.35))
        .with(Layer::sine(2093.0, Env::decay(14.0), 0.18))
        .with(Layer::sine(1318.5, Env::decay(7.0).after(0.09), 0.5))
}

pub fn kill_ding() -> Vec<f32> {
    kill_ding_voice().render()
}
/// Up a rung: a quick rising arpeggio with a shimmer. `final_rung` adds a fifth note and a longer ring (the last weapon).
pub fn level_up_voice(final_rung: bool) -> Voice {
    let notes: &[f32] = if final_rung { &[523.25, 659.25, 783.99, 1046.5, 1318.5] } else { &[523.25, 659.25, 783.99, 1046.5] };
    let step = 0.065;
    let rate = if final_rung { 4.0 } else { 7.0 };
    let mut v = Voice::new(step * notes.len() as f32 + if final_rung { 0.9 } else { 0.5 }, if final_rung { 0.62 } else { 0.5 }, 0).without_attack();
    for (k, f) in notes.iter().enumerate() {
        // Each note: its fundamental and an octave shimmer, ringing out, fading in over a couple of milliseconds.
        v = v.with(Layer::new(Src::Tone { hz: *f, partials: vec![(2.0, 0.2 / 0.7)] }, Env::decay(rate).fading_in(400.0).after(k as f32 * step), 0.7));
    }
    v
}

pub fn level_up(final_rung: bool) -> Vec<f32> {
    level_up_voice(final_rung).render()
}
/// Taking damage: a dull thud with a rasp, low enough not to be mistaken for anything you did.
pub fn hurt_voice() -> Voice {
    Voice::new(0.28, 0.66, 0x4172)
        .with(Layer::sine(70.0, Env::decay(16.0), 0.9))
        .with(Layer::noise(Filter::Low(0.12), Env::decay(30.0), 1.4))
        .with(Layer::sine(190.0, Env::decay(30.0), 0.3))
}

pub fn hurt() -> Vec<f32> {
    hurt_voice().render()
}
/// Dying: a falling tone and a rumble.
pub fn death() -> Vec<f32> {
    death_voice().render()
}

/// Dying: a falling tone and a rumble.
pub fn death_voice() -> Voice {
    Voice::new(1.1, 0.6, 0xDEAD).with(Layer::new(Src::Sweep { from: 365.0, to: 45.0, rate: 2.4 }, Env::decay(4.5), 0.7)).with(Layer::noise(
        Filter::Low(0.03),
        Env::decay(5.0),
        3.0,
    ))
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
            lp += (0.02 + 0.5 * rise) * (noise.white() - lp);
            (phase.sin() * 0.35 + lp * 1.2) * (PI * rise).sin().powf(1.5)
        })
        .collect();
    finish(clip, 0.5)
}

/// A jump pad: a springy upward chirp.
pub fn pad_launch() -> Vec<f32> {
    pad_launch_voice().render()
}

/// A jump pad: a springy upward chirp.
pub fn pad_launch_voice() -> Voice {
    Voice::new(0.45, 0.5, 0x0BAD).with(Layer::new(Src::Sweep { from: 160.0, to: 1660.0, rate: 9.0 }, Env::decay(11.0), 0.8)).with(Layer::noise(
        Filter::None,
        Env::decay(31.0),
        0.12,
    ))
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
pub fn victory_voice() -> Voice {
    chord_run_voice(&[523.25, 659.25, 783.99, 1046.5, 1318.5, 1568.0], 0.11, 1.7, 4.5, 0.62)
}

/// A win: a bright rising fanfare.
pub fn victory() -> Vec<f32> {
    victory_voice().render()
}

/// A loss: a slow falling minor line.
pub fn defeat_voice() -> Voice {
    chord_run_voice(&[440.0, 392.0, 349.23, 293.66], 0.2, 1.6, 4.5, 0.5)
}

/// A loss: a slow falling minor line.
pub fn defeat() -> Vec<f32> {
    defeat_voice().render()
}

fn chord_run_voice(notes: &[f32], step: f32, seconds: f32, rate: f32, level: f32) -> Voice {
    let mut v = Voice::new(seconds, level, 0).without_attack();
    for (k, f) in notes.iter().enumerate() {
        // The note, its octave above and the half below, ringing out and fading in over a couple of milliseconds.
        v = v.with(Layer::new(
            Src::Tone { hz: *f, partials: vec![(2.0, 0.15 / 0.6), (0.5, 0.2 / 0.6)] },
            Env::decay(rate).fading_in(300.0).after(k as f32 * step),
            0.6,
        ));
    }
    v
}
/// One heartbeat ("lub-dub"), for low health.
pub fn heartbeat_voice() -> Voice {
    Voice::new(0.55, 0.55, 0).with(Layer::sine(58.0, Env::decay(22.0), 1.0)).with(Layer::sine(52.0, Env::decay(26.0).after(0.17), 0.7))
}

pub fn heartbeat() -> Vec<f32> {
    heartbeat_voice().render()
}
/// A footstep: a soft thud and a scuff of noise. `variant` picks between two slightly different feet.
pub fn footstep_voice(variant: u32) -> Voice {
    let v = variant as f32;
    Voice::new(0.1, 0.5, 0xF007 + variant * 7919).with(Layer::sine(72.0 + 9.0 * v, Env::decay(48.0), 0.55)).with(Layer::noise(
        Filter::Low(0.16 + 0.03 * v),
        Env::decay(42.0),
        1.6,
    ))
}

pub fn footstep(variant: u32) -> Vec<f32> {
    footstep_voice(variant).render()
}
/// Landing: a heavier thud than a step.
pub fn landing_voice() -> Voice {
    Voice::new(0.2, 0.7, 0x1A2D).with(Layer::sine(58.0, Env::decay(20.0), 1.0)).with(Layer::noise(Filter::Low(0.1), Env::decay(24.0), 2.0))
}

pub fn landing() -> Vec<f32> {
    landing_voice().render()
}
/// Pushing off: a short breath of air.
pub fn jump() -> Vec<f32> {
    jump_voice().render()
}

/// Pushing off: a short breath of air.
pub fn jump_voice() -> Voice {
    Voice::new(0.16, 0.3, 0x0A1B).without_attack().with(Layer::noise(Filter::LowOpen { closed: 0.08, open: 0.38 }, Env::decay(0.0).swelling(0.08, 0.08), 2.5))
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
    /// Killchain's restrained feedback: dry impacts and low mechanical pulses in place of melodic rewards.
    pub fn new_tactical() -> SoundBank {
        let mut bank = Self::new();
        bank.hit_tick = tactical_signal(0.07, 760.0, 0x417, 0.26);
        bank.kill = tactical_signal(0.18, 185.0, 0xA111, 0.38);
        bank.respawn = tactical_signal(0.20, 125.0, 0x5A11, 0.30);
        bank.beep = tactical_signal(0.11, 440.0, 0xBEE, 0.28);
        bank.go = tactical_signal(0.25, 95.0, 0x601, 0.36);
        bank.victory = tactical_signal(0.65, 82.0, 0x71C, 0.42);
        bank.defeat = tactical_signal(0.55, 65.0, 0xDEF, 0.34);
        bank.jump = tactical_signal(0.10, 90.0, 0xC10, 0.12);
        bank.draw = reload_shell();
        bank
    }

    /// Builds every clip (a few milliseconds of arithmetic).
    pub fn new() -> SoundBank {
        let guns = Weapon::ROSTER.iter().map(|w| gun_shot(*w)).collect();
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
            draw: crate::synth::synth_weapon_click(),
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

// A single damped impact with inharmonic metal and filtered noise; no scale, chord or upward pitch sweep.
fn tactical_signal(seconds: f32, pitch: f32, seed: u32, level: f32) -> Vec<f32> {
    let mut noise = Noise(seed);
    let mut low = 0.0;
    let rate = 7.0 / seconds;
    let clip = (0..samples(seconds))
        .map(|i| {
            let t = time(i);
            low += 0.14 * (noise.white() - low);
            let metal = partial(pitch, t, rate) * 0.35 + partial(pitch * 2.73, t, rate * 2.5) * 0.12;
            (metal + low * decay(t, rate) * 1.4 + noise.white() * decay(t, rate * 7.0) * 0.18) * attack(t)
        })
        .collect();
    finish(clip, level)
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

// ---------------------------------------------------------------------------------------------------------------------------------
// the loadout shooter's sounds
// ---------------------------------------------------------------------------------------------------------------------------------

/// An explosion: a flat crack, a deep boom falling through the floor, a rolling rumble and debris.
pub fn explosion() -> Vec<f32> {
    let n = samples(1.9);
    let mut noise = Noise(0xB1A57);
    let (mut lp, mut lp2) = (0.0f32, 0.0f32);
    let clip = (0..n)
        .map(|i| {
            let t = time(i);
            lp += 0.25 * (noise.white() - lp);
            lp2 += 0.02 * (noise.white() - lp2);
            let crack = noise.white() * decay(t, 70.0) * 0.8;
            let boom = (TAU * (95.0 * decay(t, 2.2) + 28.0) * t).sin() * decay(t, 4.0) * 1.1;
            let rumble = lp2 * decay(t, 3.2) * 9.0;
            let debris = lp * decay(t, 7.0) * 0.5 * (0.6 + 0.4 * (TAU * 23.0 * t).sin());
            (crack + boom + rumble + debris) * attack(t)
        })
        .collect();
    finish(clip, 0.95)
}

/// A flashbang: a hard bang and a high ringing that lingers.
pub fn flash_pop() -> Vec<f32> {
    let n = samples(2.0);
    let mut noise = Noise(0xF1A5);
    let clip = (0..n)
        .map(|i| {
            let t = time(i);
            let bang = noise.white() * decay(t, 45.0) * 1.0 + partial(180.0, t, 14.0) * 0.6;
            let ring = (partial(4200.0, t, 1.6) * 0.5 + partial(6300.0, t, 2.2) * 0.25) * (1.0 - decay(t, 60.0));
            (bang + ring) * attack(t)
        })
        .collect();
    finish(clip, 0.9)
}

/// A smoke grenade popping and venting: a dull pop, then a hiss.
pub fn smoke_pop() -> Vec<f32> {
    let n = samples(1.5);
    let mut noise = Noise(0x5A0CE);
    let mut lp = 0.0f32;
    let clip = (0..n)
        .map(|i| {
            let t = time(i);
            lp += 0.35 * (noise.white() - lp);
            let pop = partial(110.0, t, 26.0) * 0.9 + noise.white() * decay(t, 120.0) * 0.4;
            let hiss = (noise.white() - lp) * 0.35 * (1.0 - decay(t, 20.0)) * decay(t, 2.2);
            (pop + hiss) * attack(t)
        })
        .collect();
    finish(clip, 0.6)
}

/// An incendiary bursting: a whump of air and a crackle.
pub fn fire_burst() -> Vec<f32> {
    let n = samples(1.4);
    let mut noise = Noise(0xF1BE);
    let (mut lp, mut crackle) = (0.0f32, 0.0f32);
    let clip = (0..n)
        .map(|i| {
            let t = time(i);
            lp += 0.08 * (noise.white() - lp);
            if noise.white() > 0.93 {
                crackle = noise.white().abs();
            }
            crackle *= 0.9;
            let whump = partial(70.0, t, 9.0) * 1.0 + lp * decay(t, 3.0) * 6.0;
            (whump + crackle * 0.35 * decay(t, 1.6)) * attack(t)
        })
        .collect();
    finish(clip, 0.75)
}

/// A grenade bouncing off something hard.
pub fn grenade_bounce() -> Vec<f32> {
    let n = samples(0.3);
    let mut noise = Noise(0xB0CE);
    let clip = (0..n)
        .map(|i| {
            let t = time(i);
            (partial(1250.0, t, 28.0) * 0.5 + partial(2010.0, t, 40.0) * 0.35 + partial(3330.0, t, 60.0) * 0.15 + noise.white() * decay(t, 400.0) * 0.4)
                * attack(t)
        })
        .collect();
    finish(clip, 0.5)
}

/// A magazine coming out and going in: two clicks and a clack.
pub fn reload_mag() -> Vec<f32> {
    let n = samples(0.95);
    let mut noise = Noise(0x3A6);
    let click = |t: f32, f: f32| partial(f, t, 160.0) * 0.6 + partial(f * 2.7, t, 240.0) * 0.25;
    let clip = (0..n)
        .map(|i| {
            let t = time(i);
            let mut v = 0.0;
            for (at, f, g) in [(0.0, 900.0, 0.7), (0.38, 640.0, 0.9), (0.78, 1300.0, 1.0)] {
                if t >= at {
                    let u = t - at;
                    v += (click(u, f) + noise.white() * decay(u, 500.0) * 0.5) * g;
                }
            }
            v * attack(t)
        })
        .collect();
    finish(clip, 0.5)
}

/// One shell going into a tube.
pub fn reload_shell() -> Vec<f32> {
    let n = samples(0.18);
    let mut noise = Noise(0x5E11);
    let clip = (0..n)
        .map(|i| {
            let t = time(i);
            (partial(520.0, t, 70.0) * 0.6 + partial(1500.0, t, 130.0) * 0.3 + noise.white() * decay(t, 600.0) * 0.4) * attack(t)
        })
        .collect();
    finish(clip, 0.42)
}

/// A bolt or pump working: clack, clack.
pub fn bolt_cycle() -> Vec<f32> {
    let n = samples(0.55);
    let mut noise = Noise(0xB017);
    let clip = (0..n)
        .map(|i| {
            let t = time(i);
            let mut v = 0.0;
            for (at, f) in [(0.0, 420.0), (0.28, 300.0)] {
                if t >= at {
                    let u = t - at;
                    v += partial(f, u, 55.0) * 0.7 + partial(f * 3.1, u, 120.0) * 0.3 + noise.white() * decay(u, 500.0) * 0.5;
                }
            }
            v * attack(t)
        })
        .collect();
    finish(clip, 0.5)
}

/// Picking something up: a short, bright double click.
pub fn pickup() -> Vec<f32> {
    let n = samples(0.22);
    let clip = (0..n)
        .map(|i| {
            let t = time(i);
            let second = if t > 0.06 { partial(1560.0, t - 0.06, 40.0) * 0.6 } else { 0.0 };
            (partial(1040.0, t, 45.0) * 0.7 + second) * attack(t)
        })
        .collect();
    finish(clip, 0.45)
}

/// A grenade leaving the hand: a short rush of air.
pub fn throw_whoosh() -> Vec<f32> {
    throw_whoosh_voice().render()
}

/// A grenade leaving the hand: a short rush of air.
pub fn throw_whoosh_voice() -> Voice {
    Voice::new(0.3, 0.4, 0x7A0).without_attack().with(Layer::noise(Filter::LowOpen { closed: 0.05, open: 0.30 }, Env::decay(0.0).swelling(0.15, 0.15), 2.2))
}
/// A knife or hatchet through the air: a thin, quick swish.
pub fn blade_swish() -> Vec<f32> {
    let n = samples(0.2);
    let mut noise = Noise(0x51BE);
    let (mut lp, mut hp) = (0.0f32, 0.0f32);
    let clip = (0..n)
        .map(|i| {
            let t = time(i);
            let open = (PI * t / 0.2).sin().powi(2);
            let x = noise.white();
            lp += 0.45 * (x - lp);
            hp += 0.15 * (lp - hp);
            (lp - hp) * open * 2.2
        })
        .collect();
    finish(clip, 0.36)
}

/// A blade cutting into a person: a wet-less, dull thud with a short metallic edge (no gore: a clipped knock).
pub fn blade_hit() -> Vec<f32> {
    let n = samples(0.18);
    let mut noise = Noise(0xB1AD);
    let mut lp = 0.0f32;
    let clip = (0..n)
        .map(|i| {
            let t = time(i);
            lp += 0.2 * (noise.white() - lp);
            (partial(130.0, t, 30.0) * 0.9 + partial(2300.0, t, 120.0) * 0.25 + lp * decay(t, 60.0)) * attack(t)
        })
        .collect();
    finish(clip, 0.6)
}

/// The sounds of the loadout shooter that the prototype's bank does not hold.
pub struct KitSounds {
    /// Explosions.
    pub explosion: Vec<f32>,
    /// A flashbang.
    pub flash_pop: Vec<f32>,
    /// A smoke grenade.
    pub smoke_pop: Vec<f32>,
    /// An incendiary.
    pub fire_burst: Vec<f32>,
    /// A grenade bouncing.
    pub bounce: Vec<f32>,
    /// A magazine reload.
    pub reload_mag: Vec<f32>,
    /// A shell loading.
    pub reload_shell: Vec<f32>,
    /// A bolt or pump.
    pub bolt: Vec<f32>,
    /// Picking something up.
    pub pickup: Vec<f32>,
    /// A throw.
    pub throw: Vec<f32>,
    /// A blade in the air.
    pub blade_swish: Vec<f32>,
    /// A blade landing.
    pub blade_hit: Vec<f32>,
}

impl KitSounds {
    /// Loadout feedback with a quiet handling click instead of the bright pickup double chime.
    pub fn new_tactical() -> KitSounds {
        let mut sounds = Self::new();
        sounds.pickup = tactical_signal(0.12, 210.0, 0xC11C, 0.22);
        sounds
    }
    /// Builds every clip.
    pub fn new() -> KitSounds {
        KitSounds {
            explosion: explosion(),
            flash_pop: flash_pop(),
            smoke_pop: smoke_pop(),
            fire_burst: fire_burst(),
            bounce: grenade_bounce(),
            reload_mag: reload_mag(),
            reload_shell: reload_shell(),
            bolt: bolt_cycle(),
            pickup: pickup(),
            throw: throw_whoosh(),
            blade_swish: blade_swish(),
            blade_hit: blade_hit(),
        }
    }
}

impl Default for KitSounds {
    fn default() -> Self {
        KitSounds::new()
    }
}

/// The natural sound of the map: wind that gusts, a far-off machine hum and now and then a distant metal clank, as interleaved stereo at
/// [`SAMPLE_RATE`]. It loops seamlessly (the end is cross-faded into the start). There is no music in the game; this is the only thing
/// playing when nobody is shooting.
pub fn ambience(seconds: f32) -> Vec<f32> {
    let fade = samples(2.0);
    let n = samples(seconds) + fade;
    let mut noise = [Noise(0xA1B1E), Noise(0xA1B2E)];
    let mut lp = [0.0f32; 2];
    let mut lp_slow = [0.0f32; 2];
    let mut buf = vec![0.0f32; n * 2];
    // Distant clanks at fixed times (deterministic), each a damped, inharmonic ring far away.
    let clanks = [(seconds * 0.17, 310.0, 0.5), (seconds * 0.46, 420.0, 0.35), (seconds * 0.81, 270.0, 0.45)];
    for i in 0..n {
        let t = time(i);
        let gust = 0.55 + 0.45 * ((TAU * t / seconds * 3.0).sin() * 0.6 + (TAU * t / seconds * 7.0 + 1.3).sin() * 0.4);
        for ch in 0..2 {
            let w = noise[ch].white();
            lp[ch] += 0.03 * (w - lp[ch]);
            lp_slow[ch] += 0.004 * (w - lp_slow[ch]);
            let wind = (lp[ch] * 1.4 + lp_slow[ch] * 5.0) * gust;
            let hum = ((TAU * 50.0 * t).sin() * 0.05 + (TAU * 100.5 * t).sin() * 0.03 + (TAU * 151.0 * t).sin() * 0.015)
                * (0.8 + 0.2 * (TAU * t / seconds * 5.0).sin());
            let mut clank = 0.0;
            for (k, (at, f, g)) in clanks.iter().enumerate() {
                let u = (t - at).rem_euclid(seconds.max(1.0));
                if u < 1.6 {
                    let side = if (k + ch) % 2 == 0 { 1.0 } else { 0.55 };
                    clank += (partial(*f, u, 4.0) * 0.5 + partial(f * 2.76, u, 6.0) * 0.3) * g * 0.05 * side;
                }
            }
            buf[i * 2 + ch] = wind * 0.55 + hum + clank;
        }
    }
    // Loop: cross-fade the tail (the extra `fade` samples) into the head.
    let len = n - fade;
    let mut out = buf[..len * 2].to_vec();
    for i in 0..fade {
        let k = i as f32 / fade as f32;
        for ch in 0..2 {
            out[i * 2 + ch] = buf[i * 2 + ch] * k + buf[(len + i) * 2 + ch] * (1.0 - k);
        }
    }
    let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs())).max(1e-6);
    for s in out.iter_mut() {
        *s *= 0.6 / peak;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tactical_feedback_is_short_quiet_bounded_and_decays_without_clicks() {
        let bank = SoundBank::new_tactical();
        let kit = KitSounds::new_tactical();
        for clip in [&bank.hit_tick, &bank.kill, &bank.respawn, &bank.beep, &bank.go, &bank.victory, &bank.defeat, &bank.jump, &kit.pickup] {
            assert!(clip.iter().all(|s| s.is_finite() && s.abs() <= 0.43));
            assert!(peak(clip) > 0.05);
            assert_eq!(*clip.last().unwrap(), 0.0);
            assert!(secs(clip) < 0.7);
        }
        assert_eq!(bank.guns, SoundBank::new().guns, "weapon reports keep their distinct voices");
        assert_ne!(bank.kill, kill_ding());
        assert_ne!(kit.pickup, pickup());
    }

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
        assert_eq!(bank.guns.len(), Weapon::ROSTER.len());
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

    #[test]
    fn the_loadout_sounds_are_finite_audible_and_end_in_silence() {
        let k = KitSounds::new();
        for (name, clip) in [
            ("explosion", &k.explosion),
            ("flash", &k.flash_pop),
            ("smoke", &k.smoke_pop),
            ("fire", &k.fire_burst),
            ("bounce", &k.bounce),
            ("reload mag", &k.reload_mag),
            ("reload shell", &k.reload_shell),
            ("bolt", &k.bolt),
            ("pickup", &k.pickup),
            ("throw", &k.throw),
            ("blade swish", &k.blade_swish),
            ("blade hit", &k.blade_hit),
        ] {
            assert!(clip.iter().all(|s| s.is_finite() && s.abs() <= 1.0), "{name}");
            assert!((0.2..=0.99).contains(&peak(clip)), "{name}: peak {}", peak(clip));
            assert!(peak(&clip[clip.len().saturating_sub(64)..]) < 0.02, "{name} ends in silence");
        }
        assert!(secs(&k.explosion) > secs(&k.bounce) * 4.0, "an explosion rolls on");
    }

    #[test]
    fn the_ambience_is_stereo_quiet_and_loops_without_a_click() {
        let a = ambience(12.0);
        assert_eq!(a.len() % 2, 0);
        assert!((secs(&a[..a.len() / 2]) - 12.0).abs() < 0.01 || (a.len() / 2) as f32 / SAMPLE_RATE as f32 > 11.9);
        assert!(a.iter().all(|s| s.is_finite() && s.abs() <= 0.7), "never loud enough to cover a footstep");
        // The join between the last and the first sample is no bigger than the largest step inside the clip.
        let step = |i: usize| (a[(i + 1) * 2] - a[i * 2]).abs();
        let worst_inside = (0..a.len() / 2 - 1).map(step).fold(0.0f32, f32::max);
        let seam = (a[0] - a[a.len() - 2]).abs();
        assert!(seam <= worst_inside * 1.5 + 0.01, "seam {seam} vs worst step {worst_inside}");
    }
}
