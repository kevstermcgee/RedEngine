//! The sounds of the countryside, made from noise and sine waves: wind, rustling leaves, crickets, bees, birdsong, an owl.
//!
//! Beds ([`Bed`]) are seamless stereo loops, a few tens of seconds long, quiet and steady enough to sit under everything, which the runtime fades in and out
//! with the hour and the place ([`crate::ambience`]). Calls ([`Call`]) are short mono clips, each different by seed (a robin never sings the same phrase twice), played
//! one at a time at random moments by the same runtime. Everything is pure arithmetic and deterministic, so `audio report ambience.nature.wind` and
//! `audio picture bird.robin` show what they are without a sound card, and the beds are held to the loop standard (no seam, no clipping, no silence).
//!
//! These are suggestions of real sounds, not recordings: the wind is a low rumble plus a breathy band that swells in gusts, leaves are a bright crackle shaped by
//! the same gusts, crickets are trains of short bursts near 4.5 kHz, a robin's phrase is a string of thin falling sweeps, a blackbird's is a slower flute-like line.

use crate::audio_analysis::{lufs, Clip};
use crate::dsp::{samples, time, Noise, OnePole};
use crate::procgen::noise::Rng;
use crate::synth::SAMPLE_RATE;
use std::f32::consts::{PI, TAU};

/// A looping background texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bed {
    /// Wind in grass and open air: a low rumble and a breathy band that swells in gusts.
    Wind,
    /// Leaves rustling in the trees, bright and crackling, shaped by the gusts.
    Leaves,
    /// Crickets in the dusk and the night.
    Crickets,
    /// The still air of night: very low, with a faint high shimmer.
    NightAir,
    /// Bees working the flowers on a warm day.
    Bees,
}

impl Bed {
    /// Every bed.
    pub const ALL: [Bed; 5] = [Bed::Wind, Bed::Leaves, Bed::Crickets, Bed::NightAir, Bed::Bees];

    /// Short name (`wind`, `leaves` ...).
    pub fn name(self) -> &'static str {
        match self {
            Bed::Wind => "wind",
            Bed::Leaves => "leaves",
            Bed::Crickets => "crickets",
            Bed::NightAir => "night_air",
            Bed::Bees => "bees",
        }
    }

    /// The loudness this bed is rendered at (LUFS), so that a gain of 1 sits it right among the others.
    pub fn target_lufs(self) -> f32 {
        match self {
            Bed::Wind => -29.0,
            Bed::Leaves => -33.0,
            Bed::Crickets => -33.0,
            Bed::NightAir => -37.0,
            Bed::Bees => -38.0,
        }
    }

    /// Length of the loop in seconds.
    pub fn seconds(self) -> f32 {
        match self {
            Bed::Wind => 30.0,
            Bed::Leaves => 24.0,
            Bed::Crickets => 18.0,
            Bed::NightAir => 28.0,
            Bed::Bees => 16.0,
        }
    }

    /// The loop, interleaved stereo at [`SAMPLE_RATE`].
    pub fn render(self) -> Vec<f32> {
        let loop_len = samples(self.seconds());
        let fade = samples(3.0).min(loop_len / 3);
        let raw = match self {
            Bed::Wind => wind(loop_len + fade, 1.0, 11),
            Bed::Leaves => leaves(loop_len + fade),
            Bed::Crickets => return normalise(crickets(loop_len), self.target_lufs()),
            Bed::NightAir => night_air(loop_len + fade),
            Bed::Bees => bees(loop_len + fade),
        };
        normalise(seamless(raw, loop_len, fade), self.target_lufs())
    }
}

/// A smooth random wander between 0 and 1, `rate` new targets per second joined by cosine curves.
struct Wander {
    rng: Rng,
    rate: f32,
    from: f32,
    to: f32,
    phase: f32,
}

impl Wander {
    fn new(seed: u32, rate: f32) -> Wander {
        let mut rng = Rng::at(seed, 3, 9);
        let (from, to) = (rng.white(), rng.white());
        Wander { rng, rate, from, to, phase: 0.0 }
    }

    fn next(&mut self) -> f32 {
        self.phase += self.rate / SAMPLE_RATE as f32;
        while self.phase >= 1.0 {
            self.phase -= 1.0;
            self.from = self.to;
            self.to = self.rng.white();
        }
        let u = 0.5 - 0.5 * (PI * self.phase).cos();
        self.from + (self.to - self.from) * u
    }
}

/// Turns `raw` (stereo, `len + fade` frames) into a loop of `len` frames that has no seam: the last `fade` frames are blended, equal-power, into the first.
fn seamless(raw: Vec<f32>, len: usize, fade: usize) -> Vec<f32> {
    let mut out = raw[..len * 2].to_vec();
    for i in 0..fade {
        let u = i as f32 / fade as f32;
        let (head, tail) = ((0.5 * PI * u).sin(), (0.5 * PI * u).cos());
        for ch in 0..2 {
            out[i * 2 + ch] = raw[i * 2 + ch] * head + raw[(len + i) * 2 + ch] * tail;
        }
    }
    out
}

/// Scales a stereo clip to a loudness.
fn normalise(mut clip: Vec<f32>, target: f32) -> Vec<f32> {
    let now = lufs(Clip { samples: &clip, channels: 2, rate: SAMPLE_RATE });
    if now.is_finite() {
        let k = 10f32.powf((target - now) / 20.0);
        for s in clip.iter_mut() {
            *s *= k;
        }
    }
    clip
}

/// Wind: a rumble and a breathy band under a gust envelope that wanders.
fn wind(frames: usize, strength: f32, seed: u32) -> Vec<f32> {
    let mut out = vec![0.0f32; frames * 2];
    let mut noise = [Noise(0x51ED_u32 ^ seed), Noise(0x7A3B_u32 ^ seed.wrapping_mul(7))];
    let (mut rumble, mut body, mut hiss_lo, mut hiss_hi) = ([OnePole::default(); 2], [OnePole::default(); 2], [OnePole::default(); 2], [OnePole::default(); 2]);
    let mut gust = Wander::new(seed, 0.13);
    let mut swirl = [Wander::new(seed + 1, 0.4), Wander::new(seed + 2, 0.45)];
    for i in 0..frames {
        let g = gust.next();
        let env = (0.18 + 0.82 * g * g * (0.6 + 0.4 * strength)).min(1.4);
        for ch in 0..2 {
            let w = noise[ch].white();
            let r = rumble[ch].low(0.006, w) * 6.0;
            let b = body[ch].low(0.05, w) * 1.8;
            let hiss = hiss_hi[ch].low(0.34, w) - hiss_lo[ch].low(0.06, w);
            let local = 0.8 + 0.4 * swirl[ch].next();
            out[i * 2 + ch] = (r * 0.8 + b) * env * local + hiss * 0.9 * env * env * env;
        }
    }
    out
}

/// Leaves: bright noise cut into a crackle, louder in the gusts.
fn leaves(frames: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; frames * 2];
    let mut noise = [Noise(0x1EAF), Noise(0x1EB7)];
    let (mut hp, mut lp) = ([OnePole::default(); 2], [OnePole::default(); 2]);
    let mut gust = Wander::new(5, 0.17);
    let mut crackle = [Wander::new(6, 21.0), Wander::new(7, 24.0)];
    for i in 0..frames {
        let g = gust.next();
        let env = 0.1 + 0.9 * g * g;
        for ch in 0..2 {
            let w = noise[ch].white();
            let band = lp[ch].low(0.55, hp[ch].high(0.12, w));
            let c = crackle[ch].next();
            out[i * 2 + ch] = band * (c * c * c * 3.0).min(1.0) * env;
        }
    }
    out
}

/// Crickets: five of them, each a train of short bursts of a slightly different pitch and rhythm. Every rhythm divides the loop, so it needs no blending.
fn crickets(frames: usize) -> Vec<f32> {
    let secs = frames as f32 / SAMPLE_RATE as f32;
    let mut out = vec![0.0f32; frames * 2];
    // (pitch Hz, chirps per loop, pulses per chirp, pulse rate Hz, chirp length s, phase 0..1, pan -1..1, gain)
    let voices = [
        (4380.0, 30.0, 3, 32.0, 0.12, 0.00, -0.7, 1.0),
        (4620.0, 25.0, 4, 36.0, 0.14, 0.31, 0.5, 0.9),
        (4150.0, 20.0, 5, 30.0, 0.19, 0.62, -0.2, 0.8),
        (4880.0, 36.0, 2, 40.0, 0.07, 0.17, 0.85, 0.7),
        (4500.0, 40.0, 3, 34.0, 0.10, 0.80, 0.1, 0.6),
    ];
    for (hz, per_loop, pulses, rate, len, phase, pan, gain) in voices {
        let period = secs / per_loop;
        let (l, r) = (((1.0 - pan) * 0.5f32).sqrt(), ((1.0 + pan) * 0.5f32).sqrt());
        for i in 0..frames {
            let t = time(i);
            let u = (t / period + phase) % 1.0 * period;
            if u >= len {
                continue;
            }
            // Pulses: each a short sine-shaped burst, the train rising a little in the middle.
            let pulse_period = 1.0 / rate;
            let within = u % pulse_period;
            let n = (u / pulse_period) as u32;
            if n >= pulses {
                continue;
            }
            let burst = pulse_period * 0.6;
            if within >= burst {
                continue;
            }
            let shape = (PI * within / burst).sin();
            let swell = (PI * (n as f32 + 0.5) / pulses as f32).sin().sqrt();
            let s = (TAU * hz * t).sin() * shape * shape * swell * gain * 0.4;
            out[i * 2] += s * l;
            out[i * 2 + 1] += s * r;
        }
    }
    out
}

/// The night's own sound: a very slow, very low breath and a faint high shimmer.
fn night_air(frames: usize) -> Vec<f32> {
    let mut out = wind(frames, 0.25, 23);
    let mut shimmer = [Noise(0x5A17), Noise(0x5A29)];
    let (mut a, mut b) = ([OnePole::default(); 2], [OnePole::default(); 2]);
    let mut slow = Wander::new(31, 0.07);
    for i in 0..frames {
        let s = slow.next();
        for ch in 0..2 {
            let w = shimmer[ch].white();
            let band = a[ch].low(0.5, w) - b[ch].low(0.2, w);
            out[i * 2 + ch] = out[i * 2 + ch] * 0.55 + band * 0.05 * (0.3 + s);
        }
    }
    out
}

/// Bees: a buzz near 200 Hz with a wobbling pitch, a breathy band, drifting in and out.
fn bees(frames: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; frames * 2];
    let mut noise = [Noise(0xBEE5), Noise(0xBEE7)];
    let (mut a, mut b) = ([OnePole::default(); 2], [OnePole::default(); 2]);
    // Three bees, each coming near and going away at its own pace.
    let mut near = [Wander::new(41, 0.11), Wander::new(42, 0.09), Wander::new(43, 0.14)];
    let mut phase = [0.0f32; 3];
    let base = [186.0f32, 213.0, 238.0];
    let pan = [-0.6f32, 0.4, 0.0];
    for i in 0..frames {
        let t = time(i);
        let mut mono = 0.0;
        for k in 0..3 {
            let wobble = 1.0 + 0.04 * (TAU * (3.5 + k as f32) * t + k as f32).sin() + 0.015 * (TAU * 9.0 * t).sin();
            phase[k] = (phase[k] + base[k] * wobble / SAMPLE_RATE as f32) % 1.0;
            let p = phase[k] * TAU;
            let buzz = p.sin() + 0.5 * (2.0 * p).sin() + 0.33 * (3.0 * p).sin() + 0.2 * (5.0 * p).sin();
            let n = near[k].next();
            mono += buzz * n * n * n * 0.5;
        }
        for ch in 0..2 {
            let w = noise[ch].white();
            let breath = a[ch].low(0.25, w) - b[ch].low(0.04, w);
            let side = if ch == 0 { 1.0 - 0.4 * (pan[0] + pan[1]) } else { 1.0 + 0.4 * (pan[0] + pan[1]) };
            out[i * 2 + ch] = mono * side * 0.6 + breath * 0.12 * mono.abs().min(1.0);
        }
    }
    out
}

/// A short animal call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Call {
    /// A robin: thin falling sweeps and little trills.
    Robin,
    /// A blackbird: a slow, rich, flute-like phrase.
    Blackbird,
    /// A wren: a loud, fast trill.
    Wren,
    /// A chaffinch: a cascade falling down the scale and a flourish.
    Chaffinch,
    /// A cuckoo's two notes.
    Cuckoo,
    /// A wood pigeon's soft cooing.
    Pigeon,
    /// A tawny owl's hoot.
    Owl,
}

impl Call {
    /// Every call.
    pub const ALL: [Call; 7] = [Call::Robin, Call::Blackbird, Call::Wren, Call::Chaffinch, Call::Cuckoo, Call::Pigeon, Call::Owl];

    /// Short name (`robin` ...).
    pub fn name(self) -> &'static str {
        match self {
            Call::Robin => "robin",
            Call::Blackbird => "blackbird",
            Call::Wren => "wren",
            Call::Chaffinch => "chaffinch",
            Call::Cuckoo => "cuckoo",
            Call::Pigeon => "pigeon",
            Call::Owl => "owl",
        }
    }

    /// The loudness this call is rendered at (LUFS): a wren carries, a pigeon murmurs, and all of them sit above the beds' quiet at a gain of 1.
    pub fn target_lufs(self) -> f32 {
        match self {
            Call::Robin => -24.0,
            Call::Blackbird => -23.0,
            Call::Wren => -21.0,
            Call::Chaffinch => -24.0,
            Call::Cuckoo => -22.0,
            Call::Pigeon => -26.0,
            Call::Owl => -23.0,
        }
    }

    /// The call, mono at [`SAMPLE_RATE`]; a different `seed` is a different phrase.
    pub fn render(self, seed: u32) -> Vec<f32> {
        let mut rng = Rng::at(seed, self as i32, 71);
        let mut song = Song::default();
        match self {
            Call::Robin => {
                let n = 6 + (rng.white() * 4.0) as u32;
                for k in 0..n {
                    let high = rng.range(3200.0, 5600.0);
                    if rng.white() < 0.3 {
                        // A little trill of quick notes.
                        for _ in 0..4 {
                            song.note(high * rng.range(0.9, 1.05), high * rng.range(0.8, 1.0), 0.032, 0.012, 0.55, 0.3, 0.0, 0.0);
                        }
                    } else {
                        song.note(high, high * rng.range(0.55, 0.85), rng.range(0.07, 0.16), rng.range(0.03, 0.09), 0.7, 0.18, 0.0, 0.0);
                    }
                    if k == n / 2 {
                        song.gap(rng.range(0.08, 0.18));
                    }
                }
            }
            Call::Blackbird => {
                // A pentatonic-ish set of whistled pitches.
                let scale = [1250.0f32, 1400.0, 1670.0, 1870.0, 2100.0, 2500.0];
                let n = 5 + (rng.white() * 4.0) as u32;
                let mut at = rng.below(scale.len());
                for _ in 0..n {
                    at = ((at as i32 + (rng.white() * 5.0) as i32 - 2).clamp(0, scale.len() as i32 - 1)) as usize;
                    let f = scale[at];
                    song.note(f, f * rng.range(0.9, 1.12), rng.range(0.16, 0.42), rng.range(0.06, 0.2), 0.8, 0.07, 5.5, 0.012);
                }
                song.note(3400.0, 2900.0, 0.12, 0.0, 0.4, 0.1, 0.0, 0.0);
                song.note(3600.0, 3000.0, 0.1, 0.0, 0.3, 0.1, 0.0, 0.0);
            }
            Call::Wren => {
                song.note(4300.0, 5200.0, 0.1, 0.05, 0.7, 0.2, 0.0, 0.0);
                song.note(5000.0, 4400.0, 0.1, 0.04, 0.7, 0.2, 0.0, 0.0);
                let n = 26 + (rng.white() * 8.0) as u32;
                for k in 0..n {
                    let hi = k % 2 == 0;
                    let f = if hi { rng.range(6200.0, 7000.0) } else { rng.range(4400.0, 5000.0) };
                    song.note(f, f * 0.96, 0.03, 0.006, 0.85, 0.15, 0.0, 0.0);
                }
                song.note(5200.0, 3800.0, 0.14, 0.0, 0.5, 0.15, 0.0, 0.0);
            }
            Call::Chaffinch => {
                let n = 6 + (rng.white() * 3.0) as u32;
                for k in 0..n {
                    let f = 5300.0 - k as f32 * (2400.0 / n as f32) + rng.range(-80.0, 80.0);
                    song.note(f, f * 0.9, 0.075, 0.025, 0.7, 0.2, 0.0, 0.0);
                }
                song.gap(0.06);
                song.note(3000.0, 4500.0, 0.2, 0.05, 0.8, 0.2, 0.0, 0.0);
                song.note(4600.0, 3400.0, 0.12, 0.0, 0.5, 0.2, 0.0, 0.0);
            }
            Call::Cuckoo => {
                let f = rng.range(700.0, 780.0);
                song.note(f, f * 0.98, 0.3, 0.13, 0.9, 0.12, 0.0, 0.0);
                song.note(f * 0.83, f * 0.82, 0.42, 0.0, 0.9, 0.12, 0.0, 0.0);
            }
            Call::Pigeon => {
                let f = rng.range(470.0, 540.0);
                for (len, gap, up) in [(0.26, 0.1, 1.0), (0.36, 0.08, 1.12), (0.22, 0.07, 0.98), (0.2, 0.07, 0.98), (0.5, 0.0, 0.95)] {
                    song.note(f * up, f * up * 0.97, len, gap, 0.85, 0.25, 5.0, 0.035);
                }
            }
            Call::Owl => {
                let f = rng.range(360.0, 410.0);
                song.note(f * 1.04, f * 0.97, 0.5, 0.55, 0.8, 0.3, 16.0, 0.03);
                song.note(f * 1.05, f * 1.0, 0.24, 0.12, 0.85, 0.3, 14.0, 0.03);
                song.note(f * 1.0, f * 0.9, 0.9, 0.0, 0.85, 0.3, 18.0, 0.04);
            }
        }
        let clip = song.finish(0.5);
        let now = lufs(Clip { samples: &clip, channels: 1, rate: SAMPLE_RATE });
        let k = 10f32.powf((self.target_lufs() - now) / 20.0);
        clip.into_iter().map(|s| s * k).collect()
    }
}

/// Builds a call note by note.
#[derive(Default)]
struct Song {
    out: Vec<f32>,
}

impl Song {
    /// A note gliding from `f0` to `f1` hertz over `dur` seconds, then `gap` seconds of silence. `second` is the share of the octave above mixed in
    /// (a brighter, birdier tone), `vib` / `depth` a vibrato of that rate and relative depth.
    #[allow(clippy::too_many_arguments)]
    fn note(&mut self, f0: f32, f1: f32, dur: f32, gap: f32, amp: f32, second: f32, vib: f32, depth: f32) {
        let n = samples(dur);
        let mut phase = 0.0f32;
        for i in 0..n {
            let u = i as f32 / n as f32;
            let slide = 0.5 - 0.5 * (PI * u).cos();
            let t = i as f32 / SAMPLE_RATE as f32;
            let f = (f0 + (f1 - f0) * slide) * (1.0 + depth * (TAU * vib * t).sin());
            phase = (phase + f / SAMPLE_RATE as f32) % 1.0;
            let p = phase * TAU;
            // A raised-cosine swell with a quick start and a longer fall: a note is a breath.
            let env = (PI * u.powf(0.7)).sin().powf(0.8);
            self.out.push((p.sin() + second * (2.0 * p).sin()) * env * amp);
        }
        self.gap(gap);
    }

    fn gap(&mut self, secs: f32) {
        let n = samples(secs);
        self.out.extend(std::iter::repeat_n(0.0, n));
    }

    fn finish(mut self, level: f32) -> Vec<f32> {
        // A little air before and after, then the call at its level.
        let mut clip = vec![0.0; samples(0.02)];
        clip.append(&mut self.out);
        clip.extend(std::iter::repeat_n(0.0, samples(0.1)));
        crate::dsp::finish(clip, level)
    }
}

trait Below {
    fn below(&mut self, n: usize) -> usize;
}

impl Below for Rng {
    fn below(&mut self, n: usize) -> usize {
        ((self.white() * n as f32) as usize).min(n - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_analysis::{analyze, problems, Kind};

    #[test]
    fn every_bed_is_a_clean_seamless_loop_at_its_loudness() {
        for bed in Bed::ALL {
            let clip = bed.render();
            assert_eq!(clip.len(), samples(bed.seconds()) * 2, "{}", bed.name());
            let report = analyze(Clip { samples: &clip, channels: 2, rate: SAMPLE_RATE });
            let found = problems(&report, Kind::Loop);
            assert!(found.is_empty(), "{}: {found:?}", bed.name());
            assert!((report.lufs - bed.target_lufs()).abs() < 0.6, "{} is {:.1} LUFS, wanted {}", bed.name(), report.lufs, bed.target_lufs());
        }
    }

    #[test]
    fn beds_sound_like_what_they_are() {
        let report = |b: Bed| analyze(Clip { samples: &b.render(), channels: 2, rate: SAMPLE_RATE });
        let (wind, leaves, crickets, bees, night) = (report(Bed::Wind), report(Bed::Leaves), report(Bed::Crickets), report(Bed::Bees), report(Bed::NightAir));
        // Wind is low and breathy, leaves are bright, crickets sit near 4-5 kHz, bees near 200 Hz.
        assert!(wind.centroid_hz < 1500.0, "wind centroid {}", wind.centroid_hz);
        assert!(leaves.centroid_hz > 2000.0 && leaves.centroid_hz > 1.8 * wind.centroid_hz, "leaves {} wind {}", leaves.centroid_hz, wind.centroid_hz);
        assert!((3800.0..5500.0).contains(&crickets.centroid_hz), "crickets {}", crickets.centroid_hz);
        assert!(bees.centroid_hz < 1200.0, "bees {}", bees.centroid_hz);
        assert!(night.centroid_hz < 3000.0);
    }

    #[test]
    fn gusts_make_the_wind_swell_and_fall() {
        let clip = Bed::Wind.render();
        let window = samples(1.0) * 2;
        let rms: Vec<f32> = clip.chunks(window).filter(|c| c.len() == window).map(|c| (c.iter().map(|s| s * s).sum::<f32>() / c.len() as f32).sqrt()).collect();
        let (lo, hi) = (rms.iter().cloned().fold(f32::MAX, f32::min), rms.iter().cloned().fold(0.0f32, f32::max));
        assert!(hi > 2.0 * lo, "the wind should not be steady: {lo}..{hi}");
    }

    #[test]
    fn every_call_is_a_clean_short_clip_and_a_seed_makes_a_new_phrase() {
        for call in Call::ALL {
            let a = call.render(1);
            let report = analyze(Clip { samples: &a, channels: 1, rate: SAMPLE_RATE });
            let secs = a.len() as f32 / SAMPLE_RATE as f32;
            assert!((0.3..6.0).contains(&secs), "{}: {secs:.2}s", call.name());
            assert!(problems(&report, Kind::OneShot).is_empty(), "{}: {:?}", call.name(), problems(&report, Kind::OneShot));
            assert_eq!(a, call.render(1), "{} is not deterministic", call.name());
            if !matches!(call, Call::Cuckoo) {
                assert_ne!(a, call.render(2), "{} sings the same every time", call.name());
            }
        }
    }

    #[test]
    fn calls_sit_where_the_real_birds_do() {
        let pitch = |c: Call| analyze(Clip { samples: &c.render(3), channels: 1, rate: SAMPLE_RATE }).centroid_hz;
        // Owls and pigeons are low, cuckoos middling, wrens and robins high.
        assert!(pitch(Call::Owl) < 700.0, "owl {}", pitch(Call::Owl));
        assert!(pitch(Call::Pigeon) < 900.0, "pigeon {}", pitch(Call::Pigeon));
        assert!((500.0..1500.0).contains(&pitch(Call::Cuckoo)), "cuckoo {}", pitch(Call::Cuckoo));
        assert!(pitch(Call::Wren) > 3500.0, "wren {}", pitch(Call::Wren));
        assert!(pitch(Call::Robin) > 2500.0, "robin {}", pitch(Call::Robin));
        assert!(pitch(Call::Blackbird) > 1000.0 && pitch(Call::Blackbird) < pitch(Call::Robin));
    }
}
