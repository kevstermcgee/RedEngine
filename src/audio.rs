//! Minimal sound-effect playback.
//!
//! Same philosophy as the engine's procedural meshes: no imported/licensed audio files to ship
//! or track down — short effects are synthesized directly as PCM samples in Rust and handed to
//! [`rodio`] to mix and play. `Audio::new` returns `None` rather than erroring if no output
//! device is available (a missing sound card shouldn't take the game down with it), so callers
//! always go through `Option<Audio>` and simply skip playback when it's `None`.

use rodio::{OutputStream, OutputStreamHandle, Source};

pub use crate::synth::{synth_bat_hit, synth_weapon_click, wav_bytes_i16, write_wav_i16, SAMPLE_RATE};

/// Overall level of everything played with [`Audio::play_at`]: several guns firing at once add up, and the speakers clip at 1.0.
const MASTER_GAIN: f32 = 0.75;

/// Fire-and-forget sound-effect player on the default output device.
pub struct Audio {
    // Must stay alive for `handle` to keep working — never read directly, just held.
    _stream: OutputStream,
    handle: OutputStreamHandle,
    /// The looping music, if any is playing (dropping the sink stops it).
    music: Option<rodio::Sink>,
    /// Looping layers (the ambience beds and the mood scores), each with a volume the game sets as the hour changes.
    layers: Vec<rodio::Sink>,
    /// Whether [`play`](Self::play)/[`play_at`](Self::play_at) are allowed to make sound (the settings SFX toggle).
    sfx_on: bool,
}

impl Audio {
    /// Opens the default output device; `None` (never an error) when there isn't one, so audio can never take the game down.
    pub fn new() -> Option<Self> {
        let (stream, handle) = OutputStream::try_default().ok()?;
        Some(Audio { _stream: stream, handle, music: None, layers: Vec::new(), sfx_on: true })
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

    /// Starts `samples` (interleaved stereo at [`SAMPLE_RATE`]) looping silently as a new layer and returns its id; [`set_layer_volume`](Self::set_layer_volume) brings it in.
    pub fn add_layer(&mut self, samples: Vec<f32>) -> Option<usize> {
        let sink = rodio::Sink::try_new(&self.handle).ok()?;
        sink.set_volume(0.0);
        sink.append(rodio::buffer::SamplesBuffer::new(2, SAMPLE_RATE, samples).repeat_infinite());
        self.layers.push(sink);
        Some(self.layers.len() - 1)
    }

    /// Sets a layer's volume (0 silent, 1 full).
    pub fn set_layer_volume(&self, id: usize, volume: f32) {
        if let Some(sink) = self.layers.get(id) {
            sink.set_volume(volume.clamp(0.0, 1.0));
        }
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
