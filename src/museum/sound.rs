//! The museum's own sounds, synthesized like every other sound in the engine (ADR 0008): soft footsteps that ring a little in a stone room, the shimmer of
//! stepping into a painting and the sigh of stepping out, the chime of a key, a door's slow swing, a knock on a locked one, and the voice of each kind of
//! sculpture when it is touched. Mono clips at the engine's sample rate, ready for `Audio::play_at`.

use super::sculpt::Family;
use crate::dsp::{finish, samples, Env, Filter, Layer, Src, Voice};

/// A tiny room reverb: a few feedback combs and an all-pass, enough to put a sound in a stone gallery. `wet` 0..1; the tail is added to the clip.
pub fn hall(dry: Vec<f32>, wet: f32, tail_secs: f32) -> Vec<f32> {
    let n = dry.len() + samples(tail_secs);
    let mut out = vec![0.0f32; n];
    let combs = [(1557usize, 0.78f32), (1617, 0.76), (1491, 0.79), (1422, 0.77)];
    for (len, fb) in combs {
        let mut buf = vec![0.0f32; len];
        let mut idx = 0;
        let mut lp = 0.0f32;
        for (i, o) in out.iter_mut().enumerate() {
            let x = dry.get(i).copied().unwrap_or(0.0);
            let y = buf[idx];
            lp = lp * 0.35 + y * 0.65;
            buf[idx] = x + lp * fb;
            idx = (idx + 1) % len;
            *o += y * 0.25;
        }
    }
    // One all-pass to smear the echoes.
    let mut ap = vec![0.0f32; 556];
    let mut j = 0;
    for o in out.iter_mut() {
        let b = ap[j];
        let y = -*o * 0.5 + b;
        ap[j] = *o + b * 0.5;
        j = (j + 1) % ap.len();
        *o = y;
    }
    for (i, o) in out.iter_mut().enumerate() {
        *o = dry.get(i).copied().unwrap_or(0.0) + *o * wet;
    }
    finish(out, 0.9)
}

/// A soft step on stone: a heel's tap and a scuff, ringing faintly in the room.
pub fn step(variant: u32) -> Vec<f32> {
    let v = variant as f32;
    let dry = Voice::new(0.14, 0.5, 0x57E9 + variant * 131)
        .with(Layer::sine(110.0 + 13.0 * v, Env::decay(55.0), 0.35))
        .with(Layer::noise(Filter::High(0.82), Env::decay(90.0), 0.35))
        .with(Layer::noise(Filter::Low(0.22 + 0.04 * v), Env::decay(45.0), 1.1))
        .render();
    hall(dry, 0.32, 0.9)
}

/// The flight into a painting: a rising shimmer of bells over a breath of air that opens like a door.
pub fn step_in() -> Vec<f32> {
    let mut v = Voice::new(3.6, 0.55, 0x51E9)
        .with(Layer::new(Src::Noise(Filter::LowOpen { closed: 0.02, open: 0.35 }), Env::decay(0.0).swelling(1.6, 1.6), 0.5))
        .with(Layer::new(Src::Sweep { from: 110.0, to: 220.0, rate: 0.9 }, Env::decay(0.0).swelling(1.2, 1.8), 0.25));
    // A rising run of bell tones (a major ninth chord, spread up two octaves).
    let notes = [523.25, 659.25, 783.99, 987.77, 1174.66, 1318.51, 1567.98, 1975.53];
    for (i, f) in notes.iter().enumerate() {
        let at = 0.25 + i as f32 * 0.17;
        v = v.with(Layer::new(Src::Tone { hz: *f, partials: vec![(2.76, 0.18), (5.4, 0.06)] }, Env::decay(1.6).after(at), 0.2));
    }
    hall(v.render(), 0.5, 2.5)
}

/// Stepping back out: the shimmer falls, and the room's air returns.
pub fn step_out() -> Vec<f32> {
    let mut v = Voice::new(3.0, 0.5, 0x0E7)
        .with(Layer::new(Src::Noise(Filter::LowOpen { closed: 0.02, open: 0.25 }), Env::decay(0.0).swelling(0.8, 1.8), 0.45))
        .with(Layer::new(Src::Sweep { from: 330.0, to: 165.0, rate: 1.2 }, Env::decay(0.0).swelling(0.4, 1.6), 0.2));
    let notes = [1567.98, 1318.51, 987.77, 783.99, 659.25, 523.25];
    for (i, f) in notes.iter().enumerate() {
        v = v.with(Layer::new(Src::Tone { hz: *f, partials: vec![(2.76, 0.15)] }, Env::decay(1.8).after(0.1 + i as f32 * 0.14), 0.18));
    }
    hall(v.render(), 0.45, 2.0)
}

/// A key found: a music box playing a bright little figure that rings on.
pub fn key_found() -> Vec<f32> {
    let mut v = Voice::new(4.0, 0.6, 0x4EF);
    let figure = [(0.0, 1046.5), (0.16, 1318.51), (0.32, 1567.98), (0.48, 2093.0), (0.72, 1567.98), (0.88, 2093.0), (1.1, 2637.02)];
    for (at, f) in figure {
        v = v.with(Layer::new(Src::Tone { hz: f, partials: vec![(3.0, 0.12), (4.2, 0.05)] }, Env::decay(2.2).after(at), 0.24));
    }
    // A warm chord underneath.
    for f in [261.63, 329.63, 392.0, 523.25] {
        v = v.with(Layer::new(Src::Tone { hz: f, partials: vec![(2.0, 0.25)] }, Env::decay(0.0).swelling(0.6, 2.4), 0.12));
    }
    hall(v.render(), 0.45, 2.5)
}

/// A door swinging open: a low wooden groan and the lock's soft bell.
pub fn door_open() -> Vec<f32> {
    let v = Voice::new(3.2, 0.5, 0xD00)
        .with(Layer::new(Src::Sweep { from: 95.0, to: 70.0, rate: 0.6 }, Env::decay(0.0).swelling(0.3, 2.2), 0.25))
        .with(Layer::new(Src::Noise(Filter::LowOpen { closed: 0.01, open: 0.08 }), Env::decay(0.0).swelling(0.5, 2.0), 0.6))
        .with(Layer::new(Src::Tone { hz: 392.0, partials: vec![(2.76, 0.2), (5.4, 0.08)] }, Env::decay(1.1), 0.22))
        .with(Layer::new(Src::Tone { hz: 587.33, partials: vec![(2.76, 0.15)] }, Env::decay(1.3).after(0.12), 0.16));
    hall(v.render(), 0.4, 1.8)
}

/// A locked door: two muffled knocks.
pub fn locked() -> Vec<f32> {
    let v = Voice::new(0.6, 0.55, 0x10C)
        .with(Layer::sine(82.0, Env::decay(26.0), 0.8))
        .with(Layer::noise(Filter::Low(0.12), Env::decay(30.0), 1.2))
        .with(Layer::sine(78.0, Env::decay(26.0).after(0.22), 0.7))
        .with(Layer::noise(Filter::Low(0.12), Env::decay(30.0).after(0.22), 1.0));
    hall(v.render(), 0.35, 0.8)
}

/// What a sculpture says when touched: chimes ring, a heart thumps, glass shivers, stone hums; `seed` picks its note.
pub fn touch(family: Family, seed: f32) -> Vec<f32> {
    let scale = [261.63, 293.66, 329.63, 392.0, 440.0, 523.25, 587.33, 659.25];
    let f = scale[(seed.abs() as usize) % scale.len()];
    let v = match family {
        Family::Chimes | Family::Mobile => {
            let mut v = Voice::new(4.0, 0.5, 0xC1);
            for (i, m) in [1.0, 1.25, 1.5, 2.0, 2.5].iter().enumerate() {
                v = v.with(Layer::new(Src::Tone { hz: f * 2.0 * m, partials: vec![(2.76, 0.2), (5.4, 0.07)] }, Env::decay(1.2).after(i as f32 * 0.11), 0.2));
            }
            v
        }
        Family::Heart => Voice::new(1.2, 0.6, 0x4E)
            .with(Layer::sine(55.0, Env::decay(9.0), 0.9))
            .with(Layer::noise(Filter::Low(0.06), Env::decay(14.0), 0.8))
            .with(Layer::sine(52.0, Env::decay(9.0).after(0.28), 0.7)),
        Family::Shards | Family::Rain | Family::Constellation => Voice::new(3.0, 0.45, 0x5A)
            .with(Layer::new(Src::Tone { hz: f * 4.0, partials: vec![(1.5, 0.4), (2.25, 0.2)] }, Env::decay(1.6), 0.2))
            .with(Layer::new(Src::Tone { hz: f * 4.0 * 1.006, partials: vec![] }, Env::decay(1.4).after(0.05), 0.15))
            .with(Layer::noise(Filter::High(0.9), Env::decay(6.0), 0.15)),
        _ => Voice::new(3.5, 0.45, 0x70)
            .with(Layer::new(Src::Tone { hz: f, partials: vec![(2.0, 0.35), (3.0, 0.12)] }, Env::decay(0.0).swelling(0.15, 2.8), 0.4))
            .with(Layer::new(Src::Tone { hz: f * 1.5, partials: vec![] }, Env::decay(0.0).swelling(0.4, 2.6), 0.15)),
    };
    hall(v.render(), 0.45, 1.8)
}

/// Every sound the museum makes, rendered once.
pub struct Sounds {
    pub steps: [Vec<f32>; 3],
    pub step_in: Vec<f32>,
    pub step_out: Vec<f32>,
    pub key: Vec<f32>,
    pub door: Vec<f32>,
    pub locked: Vec<f32>,
}

impl Sounds {
    pub fn render() -> Sounds {
        Sounds { steps: [step(0), step(1), step(2)], step_in: step_in(), step_out: step_out(), key: key_found(), door: door_open(), locked: locked() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sound_is_finite_audible_and_ends_quietly() {
        let s = Sounds::render();
        let mut all: Vec<&Vec<f32>> = vec![&s.step_in, &s.step_out, &s.key, &s.door, &s.locked];
        all.extend(s.steps.iter());
        let touches: Vec<Vec<f32>> = [Family::Chimes, Family::Heart, Family::Shards, Family::Vessel].iter().map(|f| touch(*f, 3.0)).collect();
        all.extend(touches.iter());
        for clip in all {
            assert!(clip.iter().all(|x| x.is_finite() && x.abs() <= 1.0));
            let peak = clip.iter().fold(0.0f32, |a, x| a.max(x.abs()));
            assert!(peak > 0.1, "audible");
            let tail = clip[clip.len() - 64..].iter().fold(0.0f32, |a, x| a.max(x.abs()));
            assert!(tail < 0.02, "ends without a click: {tail}");
        }
    }
}
