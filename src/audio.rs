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
    /// Whether [`play`](Self::play)/[`play_at`](Self::play_at) are allowed to make sound (the settings SFX toggle).
    sfx_on: bool,
}

impl Audio {
    /// Opens the default output device; `None` (never an error) when there isn't one, so audio can never take the game down.
    pub fn new() -> Option<Self> {
        let (stream, handle) = OutputStream::try_default().ok()?;
        Some(Audio { _stream: stream, handle, music: None, sfx_on: true })
    }

    /// Turns sound effects on or off (music is separate: see [`start_music`](Self::start_music)/[`set_music_volume`](Self::set_music_volume)).
    pub fn set_sfx_enabled(&mut self, on: bool) {
        self.sfx_on = on;
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
        if !self.sfx_on {
            return;
        }
        let source = rodio::buffer::SamplesBuffer::new(1, SAMPLE_RATE, clip.to_vec());
        let _ = self.handle.play_raw(source.convert_samples());
    }

    /// Plays a mono clip at `gain` (0..1, a sound's loudness) placed by `pan` (-1 left .. 1 right), fire-and-forget like [`play`](Self::play).
    /// Where a sound in the world lands is [`crate::sfx::spatial`]; this only delivers it to the two speakers.
    pub fn play_at(&self, clip: &[f32], gain: f32, pan: f32) {
        if !self.sfx_on {
            return;
        }
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

/// Encodes `interleaved` (`f32` in -1..1, `channels` per frame) as a 16-bit PCM WAV file in memory: a 44-byte
/// RIFF/WAVE header followed by little-endian `i16` samples. No new dependency for this — the format is this
/// simple. Used to let a player save the engine's generated music (`crate::music::loop_samples`) as a file.
pub fn wav_bytes_i16(interleaved: &[f32], sample_rate: u32, channels: u16) -> Vec<u8> {
    let bits_per_sample: u16 = 16;
    let block_align = channels * bits_per_sample / 8;
    let byte_rate = sample_rate * block_align as u32;
    let data_size = (interleaved.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_size as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_size).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM subchunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // audio format: 1 = PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits_per_sample.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_size.to_le_bytes());
    for s in interleaved {
        let sample = (s.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

/// [`wav_bytes_i16`], written straight to `path`.
pub fn write_wav_i16(path: &std::path::Path, interleaved: &[f32], sample_rate: u32, channels: u16) -> Result<(), String> {
    std::fs::write(path, wav_bytes_i16(interleaved, sample_rate, channels)).map_err(|e| format!("{}: {e}", path.display()))
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
    fn wav_bytes_round_trip_the_header_and_sample_count() {
        let samples = vec![0.0, 0.5, -1.0, 1.0, -0.25, 0.25]; // 3 stereo frames
        let bytes = wav_bytes_i16(&samples, SAMPLE_RATE, 2);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[12..16], b"fmt ");
        let channels = u16::from_le_bytes([bytes[22], bytes[23]]);
        let rate = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);
        let bits = u16::from_le_bytes([bytes[34], bytes[35]]);
        assert_eq!((channels, rate, bits), (2, SAMPLE_RATE, 16));
        assert_eq!(&bytes[36..40], b"data");
        let data_size = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]) as usize;
        assert_eq!(data_size, samples.len() * 2);
        assert_eq!(bytes.len(), 44 + data_size);
        // A full-scale sample must round-trip to the i16 extreme, not clip into noise or silence.
        let last_two = &bytes[bytes.len() - 2..];
        assert_eq!(i16::from_le_bytes([last_two[0], last_two[1]]), (0.25 * i16::MAX as f32).round() as i16);
    }

    #[test]
    fn write_wav_writes_exactly_the_encoded_bytes() {
        let dir = std::env::temp_dir().join(format!("re2_audio_wav_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out.wav");
        let samples = vec![0.1, -0.2, 0.3, -0.4];
        write_wav_i16(&path, &samples, SAMPLE_RATE, 2).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), wav_bytes_i16(&samples, SAMPLE_RATE, 2));
        let _ = std::fs::remove_dir_all(&dir);
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
