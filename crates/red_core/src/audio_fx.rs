//! The two effects a score needs: a stereo feedback **delay** and a **reverb**. Pure, deterministic and cheap enough to run several passes over a
//! minute of audio, which is how a loop gets a tail that wraps (see `score`).
//!
//! The reverb is a Schroeder/Freeverb-style network: eight damped feedback combs per ear in parallel (their lengths a little different left and right, so the
//! tail is wide) followed by four allpass diffusers. Each comb's feedback is chosen from its length so the whole tail falls 60 dB in `decay` seconds, which
//! makes `decay` mean what it says, and the damping low-passes the feedback so high frequencies die first, as in a room. `mix` is the wet level relative to
//! what was sent in (RMS), not a magic gain, so "0.35" is always about -9 dB under the dry sound.

use crate::synth::SAMPLE_RATE;

/// A stereo signal: left and right.
pub type Stereo = (Vec<f32>, Vec<f32>);

/// A stereo feedback delay. `time` in seconds, `feedback` 0..0.95 (how much of each echo comes round again), `damp` 0..1 darkens each repeat. The
/// echoes ping-pong: the left echo feeds the right line and back.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Delay {
    /// Seconds between echoes.
    pub time: f32,
    /// Fraction of an echo fed back.
    pub feedback: f32,
    /// Darkening of each repeat, 0 (none) to 1.
    pub damp: f32,
    /// Wet level relative to the dry signal sent in (RMS).
    pub mix: f32,
}

/// A reverb. `decay` is the time the tail takes to fall 60 dB.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reverb {
    /// Seconds for the tail to fall 60 dB.
    pub decay: f32,
    /// Scales the room (the comb lengths): 1 is a hall, 0.5 a small room, 2 a cathedral.
    pub size: f32,
    /// High-frequency damping of the tail, 0 (bright) to 1 (dark).
    pub damp: f32,
    /// Silence before the tail starts, seconds.
    pub predelay: f32,
    /// Wet level relative to the dry signal sent in (RMS).
    pub mix: f32,
}

fn rms(v: &[f32]) -> f32 {
    (v.iter().map(|x| x * x).sum::<f32>() / v.len().max(1) as f32).sqrt()
}

/// Scales `wet` so its level is `mix` times the level of `sent` (stereo RMS), then returns it.
fn matched(mut wet: Stereo, sent: &Stereo, mix: f32) -> Stereo {
    let (rs, rw) = (rms(&sent.0).max(rms(&sent.1)), rms(&wet.0).max(rms(&wet.1)));
    let k = if rw > 1e-12 { mix * rs / rw } else { 0.0 };
    wet.0.iter_mut().for_each(|v| *v *= k);
    wet.1.iter_mut().for_each(|v| *v *= k);
    wet
}

impl Delay {
    /// The echoes of `input` alone (no dry), at `mix` relative to it.
    pub fn process(&self, input: &Stereo) -> Stereo {
        let n = input.0.len();
        let d = ((self.time * SAMPLE_RATE as f32) as usize).max(1);
        let (mut out_l, mut out_r) = (vec![0.0f32; n], vec![0.0f32; n]);
        let (mut lp_l, mut lp_r) = (0.0f32, 0.0f32);
        let k = 1.0 - self.damp.clamp(0.0, 0.99);
        for i in 0..n {
            // What arrives now from d samples ago: the left echo comes from the right line and the reverse (ping-pong).
            let (echo_l, echo_r) = if i >= d { (out_r[i - d], out_l[i - d]) } else { (0.0, 0.0) };
            lp_l += k * (echo_l - lp_l);
            lp_r += k * (echo_r - lp_r);
            // The first echo comes from the dry input; later ones from the feedback.
            let src_l = if i >= d { input.0[i - d] } else { 0.0 };
            let src_r = if i >= d { input.1[i - d] } else { 0.0 };
            out_l[i] = src_l + lp_l * self.feedback;
            out_r[i] = src_r + lp_r * self.feedback;
        }
        matched((out_l, out_r), input, self.mix)
    }
}

const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;

struct Comb {
    buf: Vec<f32>,
    at: usize,
    feedback: f32,
    damp: f32,
    state: f32,
}

impl Comb {
    fn new(len: usize, decay: f32, damp: f32) -> Comb {
        // Feedback so that after `decay` seconds (decay * rate / len passes round the loop) the level has fallen by 60 dB.
        let passes = (decay * SAMPLE_RATE as f32 / len as f32).max(1.0);
        Comb { buf: vec![0.0; len], at: 0, feedback: 0.001f32.powf(1.0 / passes), damp: damp.clamp(0.0, 0.98), state: 0.0 }
    }

    fn run(&mut self, x: f32) -> f32 {
        let out = self.buf[self.at];
        self.state = out * (1.0 - self.damp) + self.state * self.damp;
        self.buf[self.at] = x + self.state * self.feedback;
        self.at = (self.at + 1) % self.buf.len();
        out
    }
}

struct Allpass {
    buf: Vec<f32>,
    at: usize,
}

impl Allpass {
    fn run(&mut self, x: f32) -> f32 {
        let delayed = self.buf[self.at];
        let out = delayed - x;
        self.buf[self.at] = x + delayed * 0.5;
        self.at = (self.at + 1) % self.buf.len();
        out
    }
}

impl Reverb {
    /// The tail of `input` alone (no dry), at `mix` relative to it.
    pub fn process(&self, input: &Stereo) -> Stereo {
        let n = input.0.len();
        let lines = |extra: usize| -> (Vec<Comb>, Vec<Allpass>) {
            let combs = COMBS.iter().map(|c| Comb::new(((c + extra) as f32 * self.size).max(8.0) as usize, self.decay.max(0.05), self.damp)).collect();
            let all = ALLPASS.iter().map(|a| Allpass { buf: vec![0.0; ((a + extra) as f32 * self.size).max(4.0) as usize], at: 0 }).collect();
            (combs, all)
        };
        let ((mut cl, mut al), (mut cr, mut ar)) = (lines(0), lines(STEREO_SPREAD));
        let pre = (self.predelay * SAMPLE_RATE as f32) as usize;
        let (mut out_l, mut out_r) = (vec![0.0f32; n], vec![0.0f32; n]);
        for i in 0..n {
            if i < pre {
                continue;
            }
            // Both ears hear a little of both channels so a hard-panned note still blooms on both sides.
            let mono = (input.0[i - pre] + input.1[i - pre]) * 0.5 * 0.03;
            let (mut l, mut r) = (cl.iter_mut().map(|c| c.run(mono)).sum::<f32>(), cr.iter_mut().map(|c| c.run(mono)).sum::<f32>());
            for a in al.iter_mut() {
                l = a.run(l);
            }
            for a in ar.iter_mut() {
                r = a.run(r);
            }
            out_l[i] = l;
            out_r[i] = r;
        }
        matched((out_l, out_r), input, self.mix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn impulse(n: usize) -> Stereo {
        let mut l = vec![0.0f32; n];
        l[0] = 1.0;
        (l.clone(), l)
    }

    fn env_db(x: &[f32], from_s: f32, len_s: f32) -> f32 {
        let a = (from_s * SAMPLE_RATE as f32) as usize;
        let b = (a + (len_s * SAMPLE_RATE as f32) as usize).min(x.len());
        20.0 * (x[a..b].iter().map(|v| v * v).sum::<f32>() / (b - a) as f32).sqrt().max(1e-9).log10()
    }

    #[test]
    fn the_reverb_tail_falls_sixty_decibels_in_about_its_decay_time() {
        // mix 1 on an impulse so the wet level is the impulse's own level; the tail then follows the comb feedback.
        let r = Reverb { decay: 3.0, size: 1.0, damp: 0.0, predelay: 0.0, mix: 1.0 };
        let wet = r.process(&impulse(SAMPLE_RATE as usize * 8));
        let (early, late) = (env_db(&wet.0, 0.5, 0.25), env_db(&wet.0, 3.5, 0.25));
        // 3 s later is about 60 dB lower (the comb feedback is set for exactly that); allow a wide band, rooms are not exact.
        assert!(early - late > 45.0 && early - late < 85.0, "fell {} dB in 3 s", early - late);
        // A longer decay holds the tail up.
        let long = Reverb { decay: 8.0, ..r }.process(&impulse(SAMPLE_RATE as usize * 8));
        assert!(env_db(&long.0, 3.5, 0.25) > late + 15.0);
    }

    #[test]
    fn damping_makes_the_tail_darker_and_the_two_ears_differ() {
        let sig = {
            let mut x = 0x1234_5678u32;
            let n: Vec<f32> = (0..SAMPLE_RATE as usize / 4)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    (x as f32 / u32::MAX as f32) * 2.0 - 1.0
                })
                .chain(std::iter::repeat_n(0.0, SAMPLE_RATE as usize * 2))
                .collect();
            (n.clone(), n)
        };
        let bright = Reverb { decay: 2.0, size: 1.0, damp: 0.0, predelay: 0.0, mix: 1.0 }.process(&sig);
        let dark = Reverb { decay: 2.0, size: 1.0, damp: 0.9, predelay: 0.0, mix: 1.0 }.process(&sig);
        let centroid = |x: &[f32]| {
            let r = crate::audio_analysis::analyze(crate::audio_analysis::Clip { samples: x, channels: 1, rate: SAMPLE_RATE });
            r.centroid_hz
        };
        assert!(centroid(&dark.0[SAMPLE_RATE as usize / 2..]) < centroid(&bright.0[SAMPLE_RATE as usize / 2..]) * 0.7, "damping darkens the tail");
        assert!(bright.0 != bright.1, "left and right tails differ (a wide field)");
    }

    #[test]
    fn mix_is_the_wet_level_relative_to_what_was_sent_and_predelay_waits() {
        let sig = {
            let t: Vec<f32> = (0..SAMPLE_RATE as usize * 2).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
            (t.clone(), t)
        };
        let r = Reverb { decay: 1.5, size: 1.0, damp: 0.3, predelay: 0.0, mix: 0.5 }.process(&sig);
        let ratio = rms(&r.0).max(rms(&r.1)) / rms(&sig.0);
        assert!((ratio - 0.5).abs() < 0.01, "wet is half the sent level: {ratio}");
        let late = Reverb { predelay: 0.2, ..Reverb { decay: 1.5, size: 1.0, damp: 0.3, predelay: 0.0, mix: 1.0 } }.process(&impulse(SAMPLE_RATE as usize));
        assert!(late.0[..(0.19 * SAMPLE_RATE as f32) as usize].iter().all(|v| *v == 0.0), "nothing before the predelay");
        assert!(Reverb { decay: 1.0, size: 1.0, damp: 0.0, predelay: 0.0, mix: 0.0 }.process(&sig).0.iter().all(|v| *v == 0.0), "mix 0 is silent");
    }

    #[test]
    fn the_delay_repeats_at_its_time_fades_by_its_feedback_and_ping_pongs() {
        let d = Delay { time: 0.25, feedback: 0.5, damp: 0.0, mix: 1.0 };
        // A click in the left ear only: its echoes alternate ears.
        let mut left_only = impulse(SAMPLE_RATE as usize * 2);
        left_only.1 = vec![0.0; left_only.0.len()];
        let wet = d.process(&left_only);
        let at = |x: &[f32], s: f32| x[(s * SAMPLE_RATE as f32) as usize];
        let (first, second) = (at(&wet.0, 0.25), at(&wet.1, 0.5));
        assert!(first.abs() > 0.1, "an echo at 0.25 s on the left: {first}");
        assert!((second / first - 0.5).abs() < 0.02, "the next echo is half as loud: {second} {first}");
        assert!(at(&wet.0, 0.5).abs() < 1e-6 && at(&wet.1, 0.25).abs() < 1e-6, "the repeat moves to the other ear");
        let dark = Delay { damp: 0.8, ..d }.process(&left_only);
        assert!(at(&dark.1, 0.5).abs() < second.abs(), "damping softens the repeats");
    }

    #[test]
    fn effects_are_deterministic() {
        let sig = impulse(SAMPLE_RATE as usize);
        let r = Reverb { decay: 2.0, size: 1.0, damp: 0.4, predelay: 0.01, mix: 0.4 };
        assert_eq!(r.process(&sig), r.process(&sig));
    }
}
