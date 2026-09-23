//! Minimal sound-effect playback.
//!
//! Same philosophy as the engine's procedural meshes: no imported/licensed audio files to ship
//! or track down — short effects are synthesized directly as PCM samples in Rust and handed to
//! [`rodio`] to mix and play. `Audio::new` returns `None` rather than erroring if no output
//! device is available (a missing sound card shouldn't take the game down with it), so callers
//! always go through `Option<Audio>` and simply skip playback when it's `None`.

use rodio::{OutputStream, OutputStreamHandle, Source};

pub const SAMPLE_RATE: u32 = 44100;

pub struct Audio {
    // Must stay alive for `handle` to keep working — never read directly, just held.
    _stream: OutputStream,
    handle: OutputStreamHandle,
}

impl Audio {
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

/// A short, percussive metal-on-something "clank" for the tire iron connecting with something:
/// a fast-decaying low "thud" (the impact itself) layered with a higher metallic "ring" and a
/// brief filtered-noise transient at the very start (the crack of contact). Purely synthesized,
/// same reasoning as the engine's procedural primitive meshes — no sample file to import.
pub fn synth_hit_clank() -> Vec<f32> {
    let duration_s = 0.22_f32;
    let n = (SAMPLE_RATE as f32 * duration_s) as usize;
    let mut noise = Xorshift(0x9E3779B9);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SAMPLE_RATE as f32;
        let thud = (2.0 * std::f32::consts::PI * 120.0 * t).sin() * (-t * 18.0).exp();
        let ring = (2.0 * std::f32::consts::PI * 850.0 * t).sin() * (-t * 14.0).exp();
        let transient = noise.next_f32() * (-t * 60.0).exp();
        let sample = thud * 0.5 + ring * 0.4 + transient * 0.5;
        out.push((sample * 0.8).clamp(-1.0, 1.0));
    }
    out
}
