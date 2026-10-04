//! Minimal sound-effect playback.
//!
//! Same philosophy as the engine's procedural meshes: no imported/licensed audio files to ship
//! or track down — short effects are synthesized directly as PCM samples in Rust and handed to
//! [`rodio`] to mix and play. `Audio::new` returns `None` rather than erroring if no output
//! device is available (a missing sound card shouldn't take the game down with it), so callers
//! always go through `Option<Audio>` and simply skip playback when it's `None`.

use crate::mixer::{Bus, Cue, Levels, Mixer, Verdict};
use rodio::{OutputStream, OutputStreamHandle, Source};
use std::sync::Mutex;
use std::time::Instant;

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
    /// The mixing desk (`crate::mixer`): bus levels, the voice limit, ducking. Behind a lock because sounds are started through `&self`.
    mixer: Mutex<Mixer>,
    /// The volume the single music loop was set to, before ducking.
    music_base: Mutex<f32>,
    /// The sounds playing now, by the mixer's voice id, so one can be stolen.
    voices: Mutex<Vec<(u64, rodio::Sink)>>,
    /// Time zero for the mixer.
    epoch: Instant,
}

impl Audio {
    /// Opens the default output device; `None` (never an error) when there isn't one, so audio can never take the game down.
    pub fn new() -> Option<Self> {
        let (stream, handle) = OutputStream::try_default().ok()?;
        Some(Audio {
            _stream: stream,
            handle,
            music: None,
            layers: Vec::new(),
            music_base: Mutex::new(0.0),
            mixer: Mutex::new(Mixer::new(Levels::default())),
            voices: Mutex::new(Vec::new()),
            epoch: Instant::now(),
        })
    }

    /// Turns sound effects on or off (music is separate: see [`start_music`](Self::start_music)/[`set_music_volume`](Self::set_music_volume)).
    pub fn set_sfx_enabled(&mut self, on: bool) {
        // The player's sound switch is the level of every bus but the music.
        let mut mixer = self.mixer.lock().unwrap_or_else(|e| e.into_inner());
        let mut levels = mixer.levels();
        for bus in [Bus::Sfx, Bus::Ui, Bus::Ambience] {
            levels.set(bus, if on { 1.0 } else { 0.0 });
        }
        mixer.set_levels(levels);
    }

    /// Sets the master volume, 0 to 1.
    pub fn set_master_volume(&self, volume: f32) {
        let mut mixer = self.mixer.lock().unwrap_or_else(|e| e.into_inner());
        let mut levels = mixer.levels();
        levels.master = volume.clamp(0.0, 1.0);
        mixer.set_levels(levels);
    }

    fn now(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64()
    }

    /// What the music should be multiplied by right now because something important is playing over it (1 when nothing is).
    pub fn music_duck(&self) -> f32 {
        self.mixer.lock().unwrap_or_else(|e| e.into_inner()).duck_multiplier(self.now())
    }

    /// The mixer's counters (sounds started, stolen, thinned, dropped).
    pub fn mixer_stats(&self) -> crate::mixer::Stats {
        self.mixer.lock().unwrap_or_else(|e| e.into_inner()).stats()
    }

    /// Housekeeping, once a frame: forgets finished sounds and keeps the old single music loop under the duck.
    pub fn tick(&self) {
        self.voices.lock().unwrap_or_else(|e| e.into_inner()).retain(|(_, sink)| !sink.empty());
        if let Some(sink) = &self.music {
            sink.set_volume(*self.music_base.lock().unwrap_or_else(|e| e.into_inner()) * self.music_duck());
        }
    }

    /// Ducks the music now for `hold` seconds (a scene's `audio.duck` on a game event).
    pub fn duck_music(&self, duck: crate::mixer::Duck, hold: f32) {
        self.mixer.lock().unwrap_or_else(|e| e.into_inner()).duck_for(self.now(), duck, hold as f64);
    }

    /// Starts a stereo clip (interleaved) through the mixer: it may be dropped (a muted bus, a burst, too many voices), may cut another off, and may duck the music.
    pub fn play_stereo_cue(&self, stereo: Vec<f32>, cue: Cue) {
        let verdict = self.mixer.lock().unwrap_or_else(|e| e.into_inner()).request(self.now(), &cue);
        let Verdict::Play { id, gain, pan, stolen } = verdict else { return };
        let mut voices = self.voices.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(victim) = stolen {
            if let Some(i) = voices.iter().position(|(v, _)| *v == victim) {
                voices.remove(i).1.stop();
            }
        }
        // Balance: constant power, unity at the centre.
        let (left, right) = crate::sfx::pan_gains(gain * std::f32::consts::SQRT_2, pan);
        let Ok(sink) = rodio::Sink::try_new(&self.handle) else { return };
        sink.set_volume(1.0);
        sink.append(rodio::buffer::SamplesBuffer::new(
            2,
            SAMPLE_RATE,
            stereo.chunks(2).flat_map(|f| [f[0] * left, f.get(1).copied().unwrap_or(f[0]) * right]).collect::<Vec<f32>>(),
        ));
        voices.push((id, sink));
    }

    /// Starts a mono clip through the mixer, panned.
    pub fn play_cue(&self, clip: &[f32], cue: Cue) {
        let verdict = self.mixer.lock().unwrap_or_else(|e| e.into_inner()).request(self.now(), &cue);
        let Verdict::Play { id, gain, pan, stolen } = verdict else { return };
        let mut voices = self.voices.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(victim) = stolen {
            if let Some(i) = voices.iter().position(|(v, _)| *v == victim) {
                voices.remove(i).1.stop();
            }
        }
        let (left, right) = crate::sfx::pan_gains(gain, pan);
        let mut stereo = Vec::with_capacity(clip.len() * 2);
        for s in clip {
            stereo.push(s * left);
            stereo.push(s * right);
        }
        let Ok(sink) = rodio::Sink::try_new(&self.handle) else { return };
        sink.append(rodio::buffer::SamplesBuffer::new(2, SAMPLE_RATE, stereo));
        voices.push((id, sink));
    }

    fn key_of(clip: &[f32]) -> u64 {
        // Which sound this is: where the clip lives and how long it is (the bank builds each sound once).
        (clip.as_ptr() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ clip.len() as u64
    }

    /// Starts `samples` (interleaved stereo at [`SAMPLE_RATE`]) playing on repeat at `volume` (0 silent, 1 full), replacing any music already playing.
    pub fn start_music(&mut self, samples: Vec<f32>, volume: f32) {
        let Ok(sink) = rodio::Sink::try_new(&self.handle) else { return };
        sink.set_volume(volume);
        sink.append(rodio::buffer::SamplesBuffer::new(2, SAMPLE_RATE, samples).repeat_infinite());
        *self.music_base.lock().unwrap_or_else(|e| e.into_inner()) = volume;
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
        *self.music_base.lock().unwrap_or_else(|e| e.into_inner()) = volume;
        if let Some(sink) = &self.music {
            sink.set_volume(volume * self.music_duck());
        }
    }

    /// Whether music has been started.
    pub fn has_music(&self) -> bool {
        self.music.is_some()
    }

    /// Plays a synthesized mono clip once through the mixer's sound-effects bus (see [`Audio::play_cue`]).
    pub fn play(&self, clip: &[f32]) {
        // Centred at full level in both ears, as it always was.
        let stereo: Vec<f32> = clip.iter().flat_map(|s| [*s, *s]).collect();
        self.play_stereo_cue(stereo, Cue::sfx(1.0, 0.0, Self::key_of(clip), clip.len() as f32 / SAMPLE_RATE as f32));
    }

    /// Plays a mono clip at `gain` (0..1, a sound's loudness) placed by `pan` (-1 left .. 1 right) through the sound-effects bus. Where a sound in the world lands is
    /// [`crate::sfx::spatial`]; this only delivers it to the two speakers.
    pub fn play_at(&self, clip: &[f32], gain: f32, pan: f32) {
        self.play_cue(clip, Cue::sfx(gain * MASTER_GAIN, pan, Self::key_of(clip), clip.len() as f32 / SAMPLE_RATE as f32));
    }
}
