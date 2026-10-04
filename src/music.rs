//! The game's music, composed in code like its sounds (ADR 0008, 0056): one loop of driving synthwave, eight bars in A minor at 132 BPM, generated
//! at start-up and played on repeat under the sound effects. Four on the floor, a clap on two and four, offbeat hats, a rolling bass, an
//! arpeggio with a bouncing echo and a pad, everything ducking a little on each kick so the mix pumps. It is finished by wrapping its own tails
//! round the end, so the loop has no seam.
//!
//! Pure: [`loop_samples`] returns interleaved stereo `f32` at [`crate::synth::SAMPLE_RATE`]; tests check its length, level, seam, rhythm and that the
//! bass and the arpeggio play the notes the chords say (a piece of music cannot be looked at, but its pitches can be measured).

use crate::dsp::{saw, Noise};
use crate::synth::SAMPLE_RATE;
use std::f32::consts::TAU;

/// Tempo of the loop.
pub const BPM: f32 = 132.0;
/// Bars in the loop.
pub const BARS: usize = 8;

/// Chord roots as MIDI notes (bass octave) and whether the chord is minor: Am F C G, then Am F G E.
const PROGRESSION: [(i32, bool); BARS] = [(45, true), (41, false), (48, false), (43, false), (45, true), (41, false), (43, false), (40, false)];
/// Bass, one note per eighth: semitones above the chord root.
const BASS: [i32; 8] = [0, 0, 12, 0, 0, 12, 7, 0];
/// Arpeggio, one note per sixteenth: which of the chord's notes (root, third, fifth, octave).
const ARP: [usize; 16] = [0, 1, 2, 3, 2, 1, 0, 1, 2, 3, 2, 1, 0, 1, 2, 3];

/// Seconds in one beat / bar / the whole loop.
pub fn beat_secs() -> f32 {
    60.0 / BPM
}

/// Frames (stereo samples) in one bar.
pub fn bar_frames() -> usize {
    (4.0 * beat_secs() * SAMPLE_RATE as f32).round() as usize
}

/// Frames in the whole loop.
pub fn loop_frames() -> usize {
    bar_frames() * BARS
}

pub use crate::dsp::hz;

/// The three notes of a chord on `root` (MIDI), then its octave.
fn chord(root: i32, minor: bool) -> [i32; 4] {
    [root, root + if minor { 3 } else { 4 }, root + 7, root + 12]
}

/// One mono layer of the loop, rendered on its own so it can be tested and mixed.
#[derive(Debug, Clone)]
pub struct Layers {
    /// Kick and clap and hats.
    pub drums: Vec<f32>,
    /// The bass line.
    pub bass: Vec<f32>,
    /// The arpeggio (dry).
    pub arp: Vec<f32>,
    /// The chord pad.
    pub pad: Vec<f32>,
}

/// Adds `v` at frame `i`, wrapping past the end of the loop so tails carry into the start (a seamless loop).
fn add(buf: &mut [f32], i: usize, v: f32) {
    let n = buf.len();
    buf[i % n] += v;
}

/// A few milliseconds of fade-out at the end of a note of `len` frames, so it never ends on a click.
fn release(k: usize, len: usize) -> f32 {
    ((len - k) as f32 / (0.004 * SAMPLE_RATE as f32)).min(1.0)
}

/// How much a kick at the start of each beat pushes the other layers down `t` seconds after it (sidechain pumping).
fn duck(t: f32) -> f32 {
    1.0 - 0.55 * (-t / 0.11).exp()
}

/// Renders every layer of the loop.
pub fn layers() -> Layers {
    let n = loop_frames();
    let beat = beat_secs();
    let sr = SAMPLE_RATE as f32;
    let beat_frames = (beat * sr).round() as usize;
    let mut drums = vec![0.0f32; n];
    let mut bass = vec![0.0f32; n];
    let mut arp = vec![0.0f32; n];
    let mut pad = vec![0.0f32; n];
    let mut noise = Noise(0x5EED_1234);

    // Drums: a kick on every beat, a clap on two and four, a hat on every offbeat eighth.
    for b in 0..BARS * 4 {
        let start = b * beat_frames;
        let kick_len = (0.32 * sr) as usize;
        for k in 0..kick_len {
            let t = k as f32 / sr;
            let phase = TAU * (42.0 * t + 110.0 * (1.0 - (-t * 38.0).exp()) / 38.0);
            let click = if k < 60 { noise.next() * (1.0 - k as f32 / 60.0) * 0.25 } else { 0.0 };
            add(&mut drums, start + k, (phase.sin() * (-t * 9.0).exp() + click) * 0.95 * release(k, kick_len));
        }
        if b % 2 == 1 {
            let mut lp = 0.0f32;
            for k in 0..(0.2 * sr) as usize {
                let t = k as f32 / sr;
                lp += 0.35 * (noise.next() - lp);
                let body = (noise.next() - lp) * 0.7 + (TAU * 185.0 * t).sin() * 0.25;
                add(&mut drums, start + k, body * (-t * 22.0).exp() * 0.55);
            }
        }
        let half = beat_frames / 2;
        let mut hp = 0.0f32;
        for k in 0..(0.05 * sr) as usize {
            let t = k as f32 / sr;
            hp += 0.6 * (noise.next() - hp);
            add(&mut drums, start + half + k, (noise.next() - hp) * (-t * 90.0).exp() * 0.22);
        }
    }

    // Bass: rolling eighths on the chord root, a saw through a low-pass that closes on each note, ducked by the kick.
    let eighth = beat_frames / 2;
    for (bar, &(root, _)) in PROGRESSION.iter().enumerate() {
        for (i, &semis) in BASS.iter().enumerate() {
            let start = bar * bar_frames() + i * eighth;
            let f = hz(root - 12 + semis);
            let dt = f / sr;
            let (mut phase, mut lp) = (0.0f32, 0.0f32);
            let note_len = (eighth as f32 * 0.92) as usize;
            for k in 0..note_len {
                let t = k as f32 / sr;
                phase = (phase + dt).fract();
                let cutoff = 0.05 + 0.25 * (-t * 14.0).exp();
                lp += cutoff * (saw(phase, dt) - lp);
                let env = (1.0 - (-t * 400.0).exp()) * (-t * 3.0).exp();
                let since_kick = ((start + k) % beat_frames) as f32 / sr;
                add(&mut bass, start + k, lp * env * duck(since_kick) * 0.75 * release(k, note_len));
            }
        }
    }

    // Arpeggio: sixteenths through the chord, a bright pluck that decays fast.
    let sixteenth = beat_frames / 4;
    for (bar, &(root, minor)) in PROGRESSION.iter().enumerate() {
        let notes = chord(root + 24, minor);
        for (i, &which) in ARP.iter().enumerate() {
            let start = bar * bar_frames() + i * sixteenth;
            let f = hz(notes[which]);
            let dt = f / sr;
            let (mut phase, mut lp) = (0.0f32, 0.0f32);
            let note_len = (sixteenth as f32 * 2.2) as usize;
            for k in 0..note_len {
                let t = k as f32 / sr;
                phase = (phase + dt).fract();
                let cutoff = 0.04 + 0.4 * (-t * 25.0).exp();
                lp += cutoff * (saw(phase, dt) - lp);
                let env = (1.0 - (-t * 600.0).exp()) * (-t * 14.0).exp();
                let since_kick = ((start + k) % beat_frames) as f32 / sr;
                add(&mut arp, start + k, lp * env * duck(since_kick) * 0.42 * release(k, note_len));
            }
        }
    }

    // Pad: the chord held for the bar, two slightly detuned saws per note with a slow swell, soft and low.
    for (bar, &(root, minor)) in PROGRESSION.iter().enumerate() {
        let start = bar * bar_frames();
        let len = bar_frames() + (0.5 * sr) as usize; // it rings a little into the next bar
        for &note in chord(root + 12, minor).iter().take(3) {
            for detune in [-0.004f32, 0.004] {
                let f = hz(note) * (1.0 + detune);
                let dt = f / sr;
                let (mut phase, mut lp) = (0.0f32, 0.0f32);
                for k in 0..len {
                    let t = k as f32 / sr;
                    phase = (phase + dt).fract();
                    lp += 0.03 * (saw(phase, dt) - lp);
                    let swell = (1.0 - (-t * 5.0).exp()) * if k > bar_frames() { (1.0 - (k - bar_frames()) as f32 / (0.5 * sr)).max(0.0) } else { 1.0 };
                    let since_kick = ((start + k) % beat_frames) as f32 / sr;
                    add(&mut pad, start + k, lp * swell * duck(since_kick) * 0.16);
                }
            }
        }
    }
    Layers { drums, bass, arp, pad }
}

/// The whole loop: interleaved stereo, peak about 0.9.
pub fn loop_samples() -> Vec<f32> {
    let l = layers();
    let n = loop_frames();
    let delay = (beat_secs() * 0.75 * SAMPLE_RATE as f32) as usize; // a dotted eighth
    let mut out = vec![0.0f32; n * 2];
    for i in 0..n {
        let dry = l.drums[i] + l.bass[i] + l.pad[i] * 0.8;
        let mut left = dry + l.arp[i] * 0.8 + l.pad[i] * 0.2;
        let mut right = dry + l.arp[i] * 0.8 - l.pad[i] * 0.2;
        // a ping-pong echo of the arpeggio, wrapping round the loop
        for tap in 1..=4usize {
            let g = 0.5f32.powi(tap as i32);
            let j = (i + n - (tap * delay) % n) % n;
            if tap % 2 == 1 {
                right += l.arp[j] * g;
            } else {
                left += l.arp[j] * g;
            }
        }
        out[2 * i] = (left * 0.9).tanh();
        out[2 * i + 1] = (right * 0.9).tanh();
    }
    let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs())).max(1e-6);
    let k = 0.9 / peak;
    for s in out.iter_mut() {
        *s *= k;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Energy of `signal` at `freq` Hz (Goertzel): how strongly that note is present.
    fn power_at(signal: &[f32], freq: f32) -> f32 {
        let w = TAU * freq / SAMPLE_RATE as f32;
        let coeff = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        for &x in signal {
            let s = x + coeff * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        (s1 * s1 + s2 * s2 - coeff * s1 * s2) / signal.len() as f32
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    #[test]
    fn the_loop_is_eight_bars_of_finite_bounded_stereo() {
        let m = loop_samples();
        assert_eq!(m.len(), loop_frames() * 2);
        let secs = loop_frames() as f32 / SAMPLE_RATE as f32;
        assert!((secs - 8.0 * 4.0 * 60.0 / 132.0).abs() < 0.01, "eight bars at 132 BPM is {secs} s");
        assert!(m.iter().all(|s| s.is_finite() && s.abs() <= 0.91));
        let peak = m.iter().fold(0.0f32, |a, s| a.max(s.abs()));
        assert!((0.85..=0.91).contains(&peak), "peak {peak}");
        let level = rms(&m);
        assert!((0.08..0.5).contains(&level), "a steady, not overloaded level: rms {level}");
    }

    #[test]
    fn the_loop_has_no_seam_and_no_click_at_the_wrap() {
        // Every sustained layer (the drums have their attacks by design) is as smooth across the wrap as anywhere: tails run on into the start.
        let l = layers();
        for (name, layer) in [("bass", &l.bass), ("arp", &l.arp), ("pad", &l.pad)] {
            let n = layer.len();
            let wrap = (layer[0] - layer[n - 1]).abs();
            let typical = (1..n).step_by(9973).map(|i| (layer[i] - layer[i - 1]).abs()).fold(0.0f32, f32::max);
            assert!(wrap <= typical.max(0.01) * 3.0, "{name}: {} to {} at the wrap ({wrap}), typical step {typical}", layer[n - 1], layer[0]);
        }
        let m = loop_samples();
        assert!(m.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn a_kick_lands_on_every_beat_and_the_pattern_repeats_every_bar() {
        let l = layers();
        let beat = (beat_secs() * SAMPLE_RATE as f32) as usize;
        let window = |b: usize, off: usize, len: usize| rms(&l.drums[b * beat + off..b * beat + off + len]);
        for b in 0..8 {
            let on = window(b, 0, 2000);
            let off = window(b, beat / 2 + 2500, 2000); // between the kick's tail and the next beat, away from the hat
            assert!(on > off * 2.0, "beat {b}: {on} vs {off}");
        }
        let bar = bar_frames();
        let (a, b) = (rms(&l.drums[0..bar]), rms(&l.drums[bar..2 * bar]));
        assert!((a - b).abs() < a * 0.05, "every bar has the same drums: {a} vs {b}");
    }

    #[test]
    fn the_bass_plays_the_root_of_each_chord() {
        let l = layers();
        let bar = bar_frames();
        // Bars 0..8 are Am F C G Am F G E: the bass roots are A1 F1 C2 G1 A1 F1 G1 E1 (MIDI 33 29 36 31 33 29 31 28).
        for (i, midi) in [33, 29, 36, 31, 33, 29, 31, 28].into_iter().enumerate() {
            let seg = &l.bass[i * bar..(i + 1) * bar];
            let want = power_at(seg, hz(midi));
            // Against the other bars' roots (at least two semitones away, or the notes' widths would blur together).
            for other in [28, 29, 31, 33, 36].into_iter().filter(|o| (o - midi).abs() >= 2) {
                let wrong = power_at(seg, hz(other));
                assert!(want > wrong * 2.0, "bar {i}: the bass should sit on {midi}, but {other} is as strong ({want} vs {wrong})");
            }
        }
    }

    #[test]
    fn the_arpeggio_walks_the_notes_of_the_chord() {
        let l = layers();
        let bar = bar_frames();
        // Bar 0 is A minor: A4 C5 E5 (MIDI 69 72 76); bar 1 is F major: F4 A4 C5 (65 69 72); a note outside the chord is much weaker.
        for (i, chord_notes, outside) in [(0usize, [69, 72, 76], [70, 73, 75]), (1, [65, 69, 72], [66, 68, 71])] {
            let seg = &l.arp[i * bar..(i + 1) * bar];
            let weakest_in = chord_notes.iter().map(|&m| power_at(seg, hz(m))).fold(f32::MAX, f32::min);
            let strongest_out = outside.iter().map(|&m| power_at(seg, hz(m))).fold(0.0f32, f32::max);
            assert!(weakest_in > strongest_out * 3.0, "bar {i}: chord tones {weakest_in} vs strangers {strongest_out}");
        }
    }

    #[test]
    fn frequencies_follow_the_concert_pitch() {
        assert!((hz(69) - 440.0).abs() < 1e-3 && (hz(57) - 220.0).abs() < 1e-3 && (hz(33) - 55.0).abs() < 1e-3);
        assert_eq!(chord(45, true), [45, 48, 52, 57]);
        assert_eq!(chord(40, false), [40, 44, 47, 52]);
    }
}
