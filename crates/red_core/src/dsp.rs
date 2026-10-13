//! The sound kit: the few small pieces every sound in the engine is made of, in one place.
//!
//! A sound is a **[`Voice`]**: a handful of **[`Layer`]s** that are summed, given a click-free attack and normalised by [`finish`]. A layer is a **[`Src`]** (a
//! sine with harmonics, a pitch glide, or noise through a one-pole filter) shaped by an **[`Env`]** (an exponential decay, an optional fade-in, an optional
//! delay) and a gain. That is enough to describe a gunshot (a crack, a boom, a body and a tail), a bell (a few decaying partials) or an arpeggio (the same
//! notes delayed), and because a voice is plain data it can be described in JSON, measured with `red_engine2 audio`, and compared with a golden.
//!
//! Everything is pure and deterministic: the same voice renders the same samples. The primitives ([`Noise`], [`OnePole`], [`saw`], [`decay`], [`attack`],
//! [`partial`], [`finish`]) are also public, for the sounds (music, ambience, sweeps) that are not layers yet.

use crate::synth::SAMPLE_RATE;
use std::f32::consts::TAU;

/// Samples in `seconds`.
pub fn samples(seconds: f32) -> usize {
    (SAMPLE_RATE as f32 * seconds) as usize
}

/// The time in seconds of sample `i`.
pub fn time(i: usize) -> f32 {
    i as f32 / SAMPLE_RATE as f32
}

/// A tiny xorshift noise source so a noise burst needs no dependency and is the same every run: white noise in -1..1.
#[derive(Debug, Clone)]
pub struct Noise(pub u32);

impl Noise {
    /// The next white-noise sample, -1..1.
    pub fn white(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// Exponential decay `e^(-t * rate)`: 1 at `t = 0`, falling by 8.7 dB per `1/rate` seconds.
pub fn decay(t: f32, rate: f32) -> f32 {
    (-t * rate).exp()
}

/// A click-free start: ramps in over about a third of a millisecond.
pub fn attack(t: f32) -> f32 {
    1.0 - decay(t, 9000.0)
}

/// A sine partial with an exponential decay, the unit a bell is built from.
pub fn partial(freq: f32, t: f32, rate: f32) -> f32 {
    (TAU * freq * t).sin() * decay(t, rate)
}

/// A band-limited saw (PolyBLEP) for a phase in 0..1 and a phase step per sample `dt`.
pub fn saw(phase: f32, dt: f32) -> f32 {
    let mut s = 2.0 * phase - 1.0;
    if phase < dt {
        let t = phase / dt;
        s -= t + t - t * t - 1.0;
    } else if phase > 1.0 - dt {
        let t = (phase - 1.0) / dt;
        s -= t * t + t + t + 1.0;
    }
    s
}

/// Frequency of MIDI note `n` (69 is A4, 440 Hz).
pub fn hz(n: i32) -> f32 {
    440.0 * 2f32.powf((n - 69) as f32 / 12.0)
}

/// A one-pole filter: `k` near 0 is a heavy low-pass (a rumble), near 1 barely filters. `high` of the same coefficient is its complement.
#[derive(Debug, Clone, Copy, Default)]
pub struct OnePole(pub f32);

impl OnePole {
    /// Low-pass: moves toward `x` by the fraction `k` and returns where it is.
    pub fn low(&mut self, k: f32, x: f32) -> f32 {
        self.0 += k * (x - self.0);
        self.0
    }

    /// High-pass: what the low-pass of the same `k` leaves out.
    pub fn high(&mut self, k: f32, x: f32) -> f32 {
        x - self.low(k, x)
    }
}

/// Pushes a clip through a soft clipper and scales its peak to `level` (never above 0.98), with a 2 ms fade-out so nothing ends on a click.
pub fn finish(mut clip: Vec<f32>, level: f32) -> Vec<f32> {
    for s in clip.iter_mut() {
        *s = (*s * 1.25).tanh();
    }
    let peak = clip.iter().fold(0.0f32, |m, s| m.max(s.abs())).max(1e-6);
    let k = level.min(0.98) / peak;
    for s in clip.iter_mut() {
        *s *= k;
    }
    let fade = samples(0.002).min(clip.len());
    let n = clip.len();
    for j in 0..fade {
        clip[n - 1 - j] *= j as f32 / fade as f32;
    }
    clip
}

/// What a noise layer does to the voice's white noise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Filter {
    /// Straight white noise.
    None,
    /// One-pole low-pass with this coefficient (smaller is duller).
    Low(f32),
    /// The complement: everything the low-pass of this coefficient leaves out (a crack, a hiss).
    High(f32),
    /// A low-pass that opens with the layer's own swell: its coefficient goes from `closed` (silent edges) to `open` (the swell's peak). A whoosh.
    LowOpen {
        /// Coefficient while the layer is faded out.
        closed: f32,
        /// Coefficient at the top of the swell.
        open: f32,
    },
}

/// What a layer plays.
#[derive(Debug, Clone, PartialEq)]
pub enum Src {
    /// A sine at `hz` plus harmonics `(multiple of hz, amplitude)`, each with its own phase from the layer's own start.
    Tone {
        /// The fundamental.
        hz: f32,
        /// Extra partials relative to the fundamental.
        partials: Vec<(f32, f32)>,
    },
    /// A fast pitch glide: the frequency moves linearly from `from` to `to` over `1/rate` seconds and holds, played as `sin(2*pi*f(t)*t)`.
    Glide {
        /// Start frequency.
        from: f32,
        /// End frequency.
        to: f32,
        /// How fast it gets there (7 is a seventh of a second).
        rate: f32,
    },
    /// A sine whose pitch glides exponentially from `from` to `to` (rate per second), with the phase integrated so the glide is clean: a falling death
    /// tone, a springy launch chirp.
    Sweep {
        /// Frequency at the start.
        from: f32,
        /// Frequency it settles on.
        to: f32,
        /// How fast it gets there (`e^(-rate*t)` of the way remains).
        rate: f32,
    },
    /// The voice's noise through a filter. Layers share one noise stream, so a crack and a body of the same shot are correlated, as in a real blast.
    Noise(Filter),
}

/// The shape of a layer over time, measured from its own start.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Env {
    /// Exponential decay rate (`e^(-u*rate)`); 0 holds steady.
    pub decay: f32,
    /// Fade-in rate: multiplies by `1 - e^(-u*fade_in)`; 0 for none.
    pub fade_in: f32,
    /// Seconds into the voice before the layer starts.
    pub delay: f32,
    /// Seconds to swell in from silence (a smooth raised-cosine); 0 for none. A pad's slow bloom.
    pub attack: f32,
    /// Seconds to swell out to silence at the end of the voice (or note); 0 for none. With `attack` set to half the voice it is a single smooth bump.
    pub release: f32,
}

impl Env {
    /// A plain decay starting at once.
    pub fn decay(rate: f32) -> Env {
        Env { decay: rate, fade_in: 0.0, delay: 0.0, attack: 0.0, release: 0.0 }
    }

    /// The same, starting `seconds` into the voice.
    pub fn after(self, seconds: f32) -> Env {
        Env { delay: seconds, ..self }
    }

    /// The same, fading in at `rate`.
    pub fn fading_in(self, rate: f32) -> Env {
        Env { fade_in: rate, ..self }
    }

    /// The same, swelling in over `seconds` and out over `release`.
    pub fn swelling(self, attack: f32, release: f32) -> Env {
        Env { attack, release, ..self }
    }

    /// The layer's swell at `u` seconds in with `left` seconds to the end: 0 to 1, raised-cosine in and out (1 where neither is set).
    pub fn swell(&self, u: f32, left: f32) -> f32 {
        let ramp = |x: f32| 0.5 - 0.5 * (std::f32::consts::PI * x.clamp(0.0, 1.0)).cos();
        let up = if self.attack > 0.0 { ramp(u / self.attack) } else { 1.0 };
        let down = if self.release > 0.0 { ramp(left / self.release) } else { 1.0 };
        up * down
    }
}

/// One source, shaped and scaled.
#[derive(Debug, Clone)]
pub struct Layer {
    /// What it plays.
    pub src: Src,
    /// How it is shaped.
    pub env: Env,
    /// How loud, before the voice is normalised.
    pub gain: f32,
    filter: OnePole,
    phase: f32,
}

impl Layer {
    /// A layer of `src` shaped by `env` at `gain`.
    pub fn new(src: Src, env: Env, gain: f32) -> Layer {
        Layer { src, env, gain, filter: OnePole::default(), phase: 0.0 }
    }

    /// A sine at `hz` (no harmonics).
    pub fn sine(hz: f32, env: Env, gain: f32) -> Layer {
        Layer::new(Src::Tone { hz, partials: Vec::new() }, env, gain)
    }

    /// Noise through `filter`.
    pub fn noise(filter: Filter, env: Env, gain: f32) -> Layer {
        Layer::new(Src::Noise(filter), env, gain)
    }

    fn sample(&mut self, t: f32, white: f32, end: f32) -> f32 {
        let u = t - self.env.delay;
        let swell = self.env.swell(u, end - t);
        if u < 0.0 {
            // A noise filter must still see the noise that came before the layer starts, or its first samples would differ from a layer that was always running.
            if let Src::Noise(Filter::Low(k) | Filter::High(k)) = self.src {
                self.filter.low(k, white);
            }
            return 0.0;
        }
        let raw = match &self.src {
            Src::Tone { hz, partials } => (TAU * hz * u).sin() + partials.iter().map(|(m, a)| (TAU * hz * m * u).sin() * a).sum::<f32>(),
            Src::Glide { from, to, rate } => (TAU * (from + (to - from) * (u * rate).min(1.0)) * u).sin(),
            Src::Sweep { from, to, rate } => {
                self.phase += TAU * (to + (from - to) * decay(u, *rate)) / SAMPLE_RATE as f32;
                self.phase.sin()
            }
            Src::Noise(Filter::None) => white,
            Src::Noise(Filter::Low(k)) => self.filter.low(*k, white),
            Src::Noise(Filter::High(k)) => self.filter.high(*k, white),
            Src::Noise(Filter::LowOpen { closed, open }) => self.filter.low(closed + (open - closed) * swell, white),
        };
        let fade = if self.env.fade_in > 0.0 { 1.0 - decay(u, self.env.fade_in) } else { 1.0 };
        let shaped = raw * decay(u, self.env.decay) * fade;
        if self.env.attack > 0.0 || self.env.release > 0.0 {
            shaped * swell * self.gain
        } else {
            shaped * self.gain
        }
    }
}

/// A sound: layers summed, then given an attack and normalised. Plain data.
#[derive(Debug, Clone)]
pub struct Voice {
    /// Length in seconds.
    pub seconds: f32,
    /// The peak the finished clip is normalised to.
    pub level: f32,
    /// Seed of the noise the layers share.
    pub seed: u32,
    /// Ramp in over a third of a millisecond (nearly every effect; a tone that ramps itself in does not need it).
    pub attack: bool,
    /// An instrument: the `Tone` frequencies are multiples of the note it is played at (1 is the note, 2 the octave above), see [`Voice::render_note`].
    pub pitched: bool,
    /// The layers, summed in this order.
    pub layers: Vec<Layer>,
}

impl Voice {
    /// An empty voice of `seconds`, normalised to `level`, with the noise seeded by `seed`.
    pub fn new(seconds: f32, level: f32, seed: u32) -> Voice {
        Voice { seconds, level, seed, attack: true, pitched: false, layers: Vec::new() }
    }

    /// Adds a layer.
    pub fn with(mut self, layer: Layer) -> Voice {
        self.layers.push(layer);
        self
    }

    /// Turns the attack ramp off.
    pub fn without_attack(mut self) -> Voice {
        self.attack = false;
        self
    }

    /// Marks this voice as an instrument (its tones are multiples of the note).
    pub fn instrument(mut self) -> Voice {
        self.pitched = true;
        self
    }

    /// Plays this instrument at `hz`. With `hold` (seconds of gate) the note lasts the gate plus its release, at most the voice's own length; without,
    /// the voice's full length. `salt` varies the noise between notes so a repeated pluck is not a machine-gun.
    pub fn render_note(&self, hz: f32, hold: Option<f32>, salt: u32) -> Vec<f32> {
        let mut v = self.clone();
        for layer in &mut v.layers {
            if let Src::Tone { hz: h, .. } = &mut layer.src {
                *h *= hz;
            }
        }
        let release = v.layers.iter().map(|l| l.env.release).fold(0.0f32, f32::max);
        if let Some(h) = hold {
            v.seconds = (h + release.max(0.01)).min(self.seconds);
        }
        v.seed ^= salt.wrapping_mul(0x9E37_79B9);
        v.render()
    }

    /// Renders the mono clip.
    pub fn render(mut self) -> Vec<f32> {
        let mut noise = Noise(self.seed);
        let clip = (0..samples(self.seconds))
            .map(|i| {
                let t = time(i);
                let white = noise.white();
                let sum: f32 = self.layers.iter_mut().map(|l| l.sample(t, white, self.seconds)).sum();
                if self.attack {
                    sum * attack(t)
                } else {
                    sum
                }
            })
            .collect();
        finish(clip, self.level)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_analysis::{analyze, note_name, Clip};

    fn report(clip: &[f32]) -> crate::audio_analysis::Report {
        analyze(Clip { samples: clip, channels: 1, rate: SAMPLE_RATE })
    }

    #[test]
    fn the_noise_stream_is_the_engines_xorshift_and_never_changes() {
        // The first draws of a known seed: sounds are specified by these numbers, so a change here changes every noise burst in the game.
        let mut n = Noise(0x9E37_79B9);
        let first: Vec<f32> = (0..4).map(|_| n.white()).collect();
        let mut again = Noise(0x9E37_79B9);
        assert_eq!(first, (0..4).map(|_| again.white()).collect::<Vec<_>>());
        assert!(first.iter().all(|v| (-1.0..=1.0).contains(v)));
        let mut x = 0x9E37_79B9u32;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        assert_eq!(first[0], (x as f32 / u32::MAX as f32) * 2.0 - 1.0);
    }

    #[test]
    fn envelopes_decay_attack_and_notes_follow_the_documented_curves() {
        assert!((decay(0.0, 30.0) - 1.0).abs() < 1e-6 && (decay(0.1, 10.0) - (-1.0f32).exp()).abs() < 1e-6);
        assert!(attack(0.0) < 1e-6 && attack(0.001) > 0.99);
        assert!((hz(69) - 440.0).abs() < 1e-3 && (hz(57) - 220.0).abs() < 1e-3);
        assert!((partial(100.0, 0.0025, 0.0)).abs() > 0.99, "a quarter cycle of 100 Hz is the sine's peak");
    }

    #[test]
    fn the_one_pole_filters_split_the_spectrum_at_its_coefficient() {
        let (mut low, mut high) = (OnePole::default(), OnePole::default());
        let dc: Vec<f32> = (0..2000).map(|_| 1.0).collect();
        let l = dc.iter().map(|x| low.low(0.1, *x)).last().unwrap();
        let h = dc.iter().map(|x| high.high(0.1, *x)).last().unwrap();
        assert!(l > 0.99 && h.abs() < 0.01, "DC passes the low-pass and not the high-pass: {l} {h}");
        // A fast alternation is the opposite.
        let mut low = OnePole::default();
        let alt: f32 = (0..2000).map(|i| low.low(0.05, if i % 2 == 0 { 1.0 } else { -1.0 }).abs()).fold(0.0, f32::max);
        assert!(alt < 0.1, "a heavy low-pass flattens Nyquist: {alt}");
    }

    #[test]
    fn the_saw_is_band_limited_and_centred() {
        let dt = 440.0 / SAMPLE_RATE as f32;
        let mut phase = 0.0f32;
        let v: Vec<f32> = (0..44100)
            .map(|_| {
                phase = (phase + dt).fract();
                saw(phase, dt)
            })
            .collect();
        assert!((v.iter().sum::<f32>() / v.len() as f32).abs() < 0.01);
        assert!(v.iter().all(|s| s.abs() < 1.2));
    }

    #[test]
    fn a_voice_is_its_layers_summed_and_normalised_and_the_same_every_time() {
        let make = || Voice::new(0.5, 0.5, 7).with(Layer::sine(440.0, Env::decay(3.0), 1.0)).render();
        let clip = make();
        assert_eq!(clip, make(), "deterministic");
        let r = report(&clip);
        assert!(r.peak_dbfs > -6.3 && r.peak_dbfs < -5.7, "normalised to 0.5: {}", r.peak_dbfs);
        assert!(note_name(r.dominant_hz).starts_with("A4"), "{}", note_name(r.dominant_hz));
        assert_eq!(r.clipped, 0);
        assert!(r.seam < 1.0, "ends on zero, no jump: {}", r.seam);
    }

    #[test]
    fn delay_fade_in_and_harmonics_do_what_they_say() {
        let late = Voice::new(0.6, 0.5, 1).with(Layer::sine(500.0, Env::decay(2.0).after(0.3), 1.0)).render();
        let r = report(&late);
        assert!((r.lead_ms - 300.0).abs() < 8.0, "silent until the delay: {}", r.lead_ms);
        // A fade-in softens the start: the first millisecond is quieter than the same layer without it.
        let hard = Voice::new(0.1, 0.5, 1).with(Layer::sine(300.0, Env::decay(1.0), 1.0)).render();
        let soft = Voice::new(0.1, 0.5, 1).with(Layer::sine(300.0, Env::decay(1.0).fading_in(200.0), 1.0)).render();
        let rms = |c: &[f32]| (c[..441].iter().map(|v| v * v).sum::<f32>() / 441.0).sqrt();
        assert!(rms(&soft) < rms(&hard), "{} {}", rms(&soft), rms(&hard));
        // Harmonics add energy above the fundamental.
        let plain = report(&Voice::new(0.4, 0.5, 1).with(Layer::sine(200.0, Env::decay(1.0), 1.0)).render());
        let rich =
            report(&Voice::new(0.4, 0.5, 1).with(Layer::new(Src::Tone { hz: 200.0, partials: vec![(3.0, 0.6), (5.0, 0.4)] }, Env::decay(1.0), 1.0)).render());
        assert!(rich.centroid_hz > plain.centroid_hz * 1.4, "{} {}", rich.centroid_hz, plain.centroid_hz);
    }

    #[test]
    fn noise_layers_are_filtered_as_asked_and_share_one_stream() {
        let bright = report(&Voice::new(0.5, 0.5, 3).with(Layer::noise(Filter::High(0.5), Env::decay(4.0), 1.0)).render());
        let dull = report(&Voice::new(0.5, 0.5, 3).with(Layer::noise(Filter::Low(0.03), Env::decay(4.0), 1.0)).render());
        assert!(bright.centroid_hz > dull.centroid_hz * 5.0, "{} {}", bright.centroid_hz, dull.centroid_hz);
        // The same noise through a low and a high filter, summed, is the original noise: the stream is shared, not two independent ones.
        let split = Voice::new(0.3, 0.5, 9)
            .with(Layer::noise(Filter::Low(0.2), Env::decay(0.0), 1.0))
            .with(Layer::noise(Filter::High(0.2), Env::decay(0.0), 1.0))
            .without_attack()
            .render();
        let whole = Voice::new(0.3, 0.5, 9).with(Layer::noise(Filter::None, Env::decay(0.0), 1.0)).without_attack().render();
        assert!(split.iter().zip(&whole).all(|(a, b)| (a - b).abs() < 1e-4), "low + high = the whole");
    }

    #[test]
    fn a_glide_starts_at_the_first_pitch_and_settles_on_the_second() {
        let clip = Voice::new(0.6, 0.5, 1).with(Layer::new(Src::Glide { from: 400.0, to: 100.0, rate: 7.0 }, Env::decay(1.0), 1.0)).render();
        let tail = &clip[samples(0.3)..samples(0.55)];
        let r = analyze(Clip { samples: tail, channels: 1, rate: SAMPLE_RATE });
        assert!((r.dominant_hz - 100.0).abs() < 15.0, "{}", r.dominant_hz);
        let head = analyze(Clip { samples: &clip[..samples(0.03)], channels: 1, rate: SAMPLE_RATE });
        assert!(head.centroid_hz > r.centroid_hz, "it starts higher");
    }
}
