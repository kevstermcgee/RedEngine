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

/// Overall level of everything played with [`Audio::play_at`]: several guns firing at once add up, and the speakers clip at 1.0.
const MASTER_GAIN: f32 = 0.75;

/// Fire-and-forget sound-effect player on the default output device.
pub struct Audio {
    // Must stay alive for `handle` to keep working — never read directly, just held.
    _stream: OutputStream,
    handle: OutputStreamHandle,
    /// The looping music, if any is playing (dropping the sink stops it).
    music: Option<rodio::Sink>,
}

impl Audio {
    /// Opens the default output device; `None` (never an error) when there isn't one, so audio can never take the game down.
    pub fn new() -> Option<Self> {
        let (stream, handle) = OutputStream::try_default().ok()?;
        Some(Audio { _stream: stream, handle, music: None })
    }

    /// Starts `samples` (interleaved stereo at [`SAMPLE_RATE`]) playing on repeat at `volume` (0 silent, 1 full), replacing any music already playing.
    pub fn start_music(&mut self, samples: Vec<f32>, volume: f32) {
        let Ok(sink) = rodio::Sink::try_new(&self.handle) else { return };
        sink.set_volume(volume);
        sink.append(rodio::buffer::SamplesBuffer::new(2, SAMPLE_RATE, samples).repeat_infinite());
        self.music = Some(sink);
    }

    /// Sets the music's volume (no effect when none is playing).
    pub fn set_music_volume(&self, volume: f32) {
        if let Some(sink) = &self.music {
            sink.set_volume(volume);
        }
    }

    /// Whether music has been started.
    pub fn has_music(&self) -> bool {
        self.music.is_some()
    }

    /// Plays a synthesized mono clip once, fire-and-forget (mixed in automatically alongside
    /// anything else currently playing — no `Sink` bookkeeping needed since nothing here is
    /// ever paused, stopped, or replayed mid-flight).
    pub fn play(&self, clip: &[f32]) {
        let source = rodio::buffer::SamplesBuffer::new(1, SAMPLE_RATE, clip.to_vec());
        let _ = self.handle.play_raw(source.convert_samples());
    }

    /// Plays a mono clip at `gain` (0..1, a sound's loudness) placed by `pan` (-1 left .. 1 right), fire-and-forget like [`play`](Self::play).
    /// Where a sound in the world lands is [`crate::sfx::spatial`]; this only delivers it to the two speakers.
    pub fn play_at(&self, clip: &[f32], gain: f32, pan: f32) {
        let (left, right) = crate::sfx::pan_gains(gain * MASTER_GAIN, pan);
        let mut stereo = Vec::with_capacity(clip.len() * 2);
        for s in clip {
            stereo.push(s * left);
            stereo.push(s * right);
        }
        let source = rodio::buffer::SamplesBuffer::new(2, SAMPLE_RATE, stereo);
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

/// A dry metallic click: the hammer falling on an empty magazine, or raising a firearm.
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
    fn the_weapon_click_is_short_and_audible() {
        let click = synth_weapon_click();
        assert!(click.len() < 3000, "a click, not a sound effect: {} samples", click.len());
        assert!(click.iter().fold(0.0f32, |m, s| m.max(s.abs())) > 0.3);
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
