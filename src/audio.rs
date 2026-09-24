//! Minimal sound-effect playback.
//!
//! Same philosophy as the engine's procedural meshes: no imported/licensed audio files to ship
//! or track down — short effects are synthesized directly as PCM samples in Rust and handed to
//! [`rodio`] to mix and play. `Audio::new` returns `None` rather than erroring if no output
//! device is available (a missing sound card shouldn't take the game down with it), so callers
//! always go through `Option<Audio>` and simply skip playback when it's `None`.

use rodio::{OutputStream, OutputStreamHandle, Source};

/// Output sample rate of the synthesized clips, in Hz.
pub const SAMPLE_RATE: u32 = 44100;

/// Fire-and-forget sound-effect player on the default output device.
pub struct Audio {
    // Must stay alive for `handle` to keep working — never read directly, just held.
    _stream: OutputStream,
    handle: OutputStreamHandle,
}

impl Audio {
    /// Opens the default output device; `None` (never an error) when there isn't one, so audio can never take the game down.
    pub fn new() -> Option<Self> {
        let (stream, handle) = OutputStream::try_default().ok()?;
        Some(Audio { _stream: stream, handle })
    }

    /// Plays a synthesized mono clip once, fire-and-forget (mixed in automatically alongside
    /// anything else currently playing — no `Sink` bookkeeping needed since nothing here is
    /// ever paused, stopped, or replayed mid-flight).
    pub fn play(&self, clip: &[f32]) {
        let source = rodio::buffer::SamplesBuffer::new(1, SAMPLE_RATE, clip.to_vec());
        let _ = self.handle.play_raw(source.convert_samples());
    }
}

/// A tiny xorshift PRNG so a one-off noise burst doesn't need a `rand` dependency.
struct Xorshift(u32);

impl Xorshift {
    fn next_f32(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// A short wooden "thock" for the bat connecting: a fast-decaying low body thump (the impact), a
/// dry woody resonance a little above it, and a brief noise crack at the very start. Purely
/// synthesized, same reasoning as the engine's procedural meshes: no sample file to import.
pub fn synth_bat_hit() -> Vec<f32> {
    let duration_s = 0.20_f32;
    let n = (SAMPLE_RATE as f32 * duration_s) as usize;
    let mut noise = Xorshift(0x9E3779B9);
    let mut lp = 0.0f32;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SAMPLE_RATE as f32;
        let thump = (2.0 * std::f32::consts::PI * (95.0 - 30.0 * t) * t).sin() * (-t * 22.0).exp();
        let wood = (2.0 * std::f32::consts::PI * 340.0 * t).sin() * (-t * 38.0).exp();
        let wood2 = (2.0 * std::f32::consts::PI * 610.0 * t).sin() * (-t * 55.0).exp();
        lp += 0.35 * (noise.next_f32() - lp); // crude low-pass: a dull crack, not a hiss
        let crack = lp * (-t * 90.0).exp();
        let sample = thump * 0.75 + wood * 0.45 + wood2 * 0.18 + crack * 0.9;
        out.push((sample * 0.9).clamp(-1.0, 1.0));
    }
    out
}

/// A revolver shot: a sharp noise crack, a chest-thumping low boom that sweeps down, a mid-range
/// body, and a short room tail. Purely synthesized (ADR 0008).
pub fn synth_revolver_shot() -> Vec<f32> {
    let duration_s = 0.85_f32;
    let n = (SAMPLE_RATE as f32 * duration_s) as usize;
    let mut noise = Xorshift(0x1234ABCD);
    let (mut lp_mid, mut lp_tail, mut hp_lp) = (0.0f32, 0.0f32, 0.0f32);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SAMPLE_RATE as f32;
        let white = noise.next_f32();
        // High-passed noise: the crack (white minus its own low-pass).
        hp_lp += 0.5 * (white - hp_lp);
        let crack = (white - hp_lp) * (-t * 110.0).exp();
        let boom = (2.0 * std::f32::consts::PI * (150.0 - 95.0 * (t * 6.0).min(1.0)) * t).sin() * (-t * 13.0).exp();
        lp_mid += 0.16 * (white - lp_mid);
        let body = lp_mid * (-t * 28.0).exp();
        lp_tail += 0.04 * (white - lp_tail);
        let tail = lp_tail * (-t * 4.5).exp() * (1.0 - (-t * 60.0).exp());
        let sample = crack * 0.75 + boom * 0.9 + body * 1.1 + tail * 2.2;
        out.push((sample * 1.1).tanh() * REVOLVER_LEVEL);
    }
    out
}

/// Overall level of a revolver shot, 0..1 (the raw synth peaks near 0.95; this is about a third lower, roughly 4 dB).
const REVOLVER_LEVEL: f32 = 0.6;

/// A dry metallic click: the hammer falling on an empty chamber, or drawing the revolver.
pub fn synth_weapon_click() -> Vec<f32> {
    let n = (SAMPLE_RATE as f32 * 0.05) as usize;
    let mut noise = Xorshift(0xC11C4B);
    (0..n)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            let ring = (2.0 * std::f32::consts::PI * 2400.0 * t).sin() * (-t * 140.0).exp();
            let tick = noise.next_f32() * (-t * 400.0).exp();
            ((ring * 0.5 + tick * 0.6) * 0.8).clamp(-1.0, 1.0)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revolver_shot_is_loud_longer_than_the_bat_and_decays() {
        let clip = synth_revolver_shot();
        assert!(clip.len() > synth_bat_hit().len() * 2, "a shot rings out longer than a thunk");
        let peak = clip.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.4 && peak < 0.7, "a shot is clearly audible but not overpowering: peak {peak}");
        let tail = clip[clip.len() - 300..].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(tail < 0.02, "should have decayed: {tail}");
        assert!(synth_weapon_click().len() < 3000);
    }

    #[test]
    fn bat_hit_is_short_audible_and_decays() {
        let clip = synth_bat_hit();
        assert!(clip.len() > 4000 && clip.len() < 12000);
        let peak = clip.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.3 && peak <= 1.0, "peak {peak}");
        let tail = clip[clip.len() - 500..].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(tail < 0.02, "clip should have decayed by the end: {tail}");
    }
}
