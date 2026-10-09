//! The soundscape as numbers: how loud every loop wants to be, and which calls to start, from the hour, the place, the player's switches and the scene's rules.
//!
//! Everything that decides *what is heard* and nothing that makes a sound: the desktop client ([`crate::mixer`] and its sound card) drives one [`Soundscape`] and turns its [`Mix`]
//! into sound, so the dawn chorus, the music's crossfades and a rule layer's fade are decided in one place that can be tested without a sound card.
//! The loops themselves are rendered by [`crate::nature::Bed::render`], [`crate::score::Score::render`] and [`call_clip`].

use crate::ambience::{layer_target, Ambience, AudioSpec, Context, Frame, Heard};
use crate::mixer::Duck;
use crate::nature::{Bed, Call};
use crate::procgen::Biome;

/// How fast the player's own music and sound switches fade, per second.
const GATE_SLEW: f32 = 1.2;

/// A call to start now, with what the sink needs to play it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CallCue {
    /// Who, as the index in [`Call::ALL`].
    pub index: usize,
    /// Which phrase.
    pub seed: u32,
    /// Level, 0 to 1, with the scene's ambience level and the player's sound switch applied.
    pub gain: f32,
    /// Left (-1) to right (1).
    pub pan: f32,
}

/// One step's answer: the level of every loop (the beds in [`Bed::ALL`] order, the moods in [`crate::ambience::Mood::ALL`] order, then the scene's rule layers), and the calls to start.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mix {
    /// Level of each bed, 0 to 1 (scene ambience level and the sound switch applied).
    pub beds: [f32; 5],
    /// Level of each mood's score.
    pub moods: [f32; 4],
    /// Level of each rule layer.
    pub layers: Vec<f32>,
    /// Calls that begin now (only when the scene has nature sound).
    pub calls: Vec<CallCue>,
    /// A game event the scene ducks the music on: how, and for how many seconds. The caller applies it to whatever holds the duck.
    pub duck: Option<(Duck, f32)>,
}

/// The running soundscape of a scene's `audio` block.
pub struct Soundscape {
    spec: AudioSpec,
    state: Ambience,
    /// Where each rule layer's fade has got to (0 to 1).
    layer_levels: Vec<f32>,
    /// The player's music and sound switches, eased so a toggle is a fade, not a click.
    music_gate: f32,
    sound_gate: f32,
    biome: Biome,
    biome_age: f32,
    /// The last step's raw answer (for a state dump).
    pub last: Frame,
    /// The last few calls heard, newest last.
    pub recent: Vec<Heard>,
    /// How many times the music was ducked.
    pub ducks: u32,
}

impl Soundscape {
    /// A soundscape for `spec`; the same `seed` gives the same sequence of calls.
    pub fn new(spec: AudioSpec, seed: u32) -> Soundscape {
        let mut state = Ambience::new(seed);
        state.birds_off = !spec.birds;
        Soundscape {
            layer_levels: vec![0.0; spec.layers.len()],
            spec,
            state,
            music_gate: 1.0,
            sound_gate: 1.0,
            biome: Biome::Meadow,
            biome_age: 99.0,
            last: Frame::default(),
            recent: Vec::new(),
            ducks: 0,
        }
    }

    /// The `audio` block this plays.
    pub fn spec(&self) -> &AudioSpec {
        &self.spec
    }

    /// Calls started so far, ever, per call in [`Call::ALL`] order.
    pub fn totals(&self) -> [u32; 7] {
        self.state.totals
    }

    /// Where each rule layer's fade has got to.
    pub fn layer_levels(&self) -> &[f32] {
        &self.layer_levels
    }

    /// Advances by `dt` seconds: the hour (`ctx`), the place (`biome_at`, asked about twice a second), the player's switches, the scene's variables and the events of the last step
    /// in; the levels out. `duck` is the multiplier the music is under now (1 = not ducked): the caller owns that state, because a sound effect may duck the music too.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        dt: f32,
        ctx: Context,
        biome_at: impl FnOnce() -> Biome,
        music_on: bool,
        sound_on: bool,
        vars: &[(&str, f64)],
        events: &[String],
        duck: f32,
    ) -> Mix {
        self.biome_age += dt;
        if self.biome_age > 0.5 {
            self.biome = biome_at();
            self.biome_age = 0.0;
        }
        let ctx = Context { biome: self.biome, ..ctx };
        let frame = self.state.step(dt, &ctx);
        let ease = |gate: &mut f32, on: bool| *gate += ((if on { 1.0 } else { 0.0 }) - *gate).clamp(-GATE_SLEW * dt, GATE_SLEW * dt);
        ease(&mut self.music_gate, music_on);
        ease(&mut self.sound_gate, sound_on);
        // Layers that follow a rule variable fade in while it is high and out when it is not; the music ducks on the events the scene names.
        for (i, spec) in self.spec.layers.iter().enumerate() {
            let value = vars.iter().find(|(n, _)| *n == spec.var).map_or(0.0, |(_, v)| *v);
            let step = dt / spec.fade.max(0.05);
            self.layer_levels[i] += (layer_target(value, spec.above) - self.layer_levels[i]).clamp(-step, step);
        }
        let duck_now = self.spec.duck.as_ref().filter(|d| events.iter().any(|e| d.events.contains(e))).map(|d| (d.duck, d.hold));
        self.ducks += duck_now.is_some() as u32;
        let mut mix = Mix { duck: duck_now, ..Mix::default() };
        for i in 0..5 {
            mix.beds[i] = frame.beds[i] * self.spec.ambience_volume * self.sound_gate;
        }
        for i in 0..4 {
            mix.moods[i] = frame.music[i] * self.spec.music_volume * self.music_gate * duck;
        }
        mix.layers = self.layer_levels.iter().zip(&self.spec.layers).map(|(level, l)| level * l.volume * self.music_gate * duck).collect();
        if self.spec.nature {
            mix.calls = frame
                .calls
                .iter()
                .map(|h| CallCue {
                    index: Call::ALL.iter().position(|c| *c == h.call).unwrap_or(0),
                    seed: h.seed,
                    gain: h.gain * self.spec.ambience_volume,
                    pan: h.pan,
                })
                .collect();
        }
        self.recent.extend(frame.calls.iter().copied());
        if self.recent.len() > 12 {
            let drop = self.recent.len() - 12;
            self.recent.drain(..drop);
        }
        self.last = frame;
        mix
    }
}

/// One call as the clip to play: interleaved stereo at [`crate::synth::SAMPLE_RATE`]. A scene with a room (`audio.reverb`) puts every call in it, once, here.
pub fn call_clip(index: usize, seed: u32, room: Option<crate::ambience::RoomSpec>) -> Vec<f32> {
    let dry = Call::ALL[index % Call::ALL.len()].render(seed);
    room.map_or_else(|| dry.iter().flat_map(|s| [*s, *s]).collect(), |r| crate::mixer::wet(&dry, &r.reverb()))
}

/// The loop of bed `index` in [`Bed::ALL`] order: interleaved stereo at [`crate::synth::SAMPLE_RATE`].
pub fn bed_loop(index: usize) -> Vec<f32> {
    Bed::ALL[index % Bed::ALL.len()].render()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ambience::Mood;

    fn spec() -> AudioSpec {
        AudioSpec { nature: true, birds: true, music: vec![], music_volume: 0.5, ambience_volume: 0.8, reverb: None, duck: None, layers: vec![] }
    }

    fn noon() -> Context {
        Context { sun_elev_deg: 50.0, rising: false, biome: Biome::Meadow, speed: 0.0 }
    }

    #[test]
    fn the_levels_follow_the_hour_and_the_players_switches() {
        let mut s = Soundscape::new(spec(), 7);
        let mut last = Mix::default();
        for _ in 0..(60 * 30) {
            last = s.update(1.0 / 60.0, noon(), || Biome::Meadow, true, true, &[], &[], 1.0);
        }
        let day = Mood::ALL.iter().position(|m| *m == Mood::Day).unwrap();
        assert!(last.moods[day] > 0.3 && last.moods.iter().enumerate().all(|(i, v)| i == day || *v < last.moods[day]), "{last:?}");
        assert!(last.beds.iter().sum::<f32>() > 0.0 && last.beds.iter().all(|b| *b <= 0.8 + 1e-6), "beds are under the scene's ambience level: {last:?}");
        // Switching the music off fades it out, and only it.
        for _ in 0..(60 * 3) {
            last = s.update(1.0 / 60.0, noon(), || Biome::Meadow, false, true, &[], &[], 1.0);
        }
        assert!(last.moods.iter().all(|v| *v == 0.0) && last.beds.iter().sum::<f32>() > 0.0, "{last:?}");
    }

    #[test]
    fn calls_come_with_the_scenes_level_and_not_at_all_without_nature() {
        let heard = |nature: bool| {
            let mut s = Soundscape::new(AudioSpec { nature, ..spec() }, 3);
            (0..(60 * 120)).flat_map(|_| s.update(1.0 / 60.0, noon(), || Biome::Meadow, true, true, &[], &[], 1.0).calls).collect::<Vec<_>>()
        };
        let with = heard(true);
        assert!(!with.is_empty() && with.iter().all(|c| c.gain <= 0.8 && c.pan.abs() <= 1.0), "{with:?}");
        assert!(heard(false).is_empty());
        assert_eq!(heard(true), with, "the same seed, the same calls");
    }

    #[test]
    fn a_call_is_a_stereo_clip_and_a_bed_a_loop() {
        let clip = call_clip(0, 1, None);
        assert!(clip.len() > 1000 && clip.len().is_multiple_of(2) && clip.iter().any(|s| *s != 0.0));
        assert!(bed_loop(0).len().is_multiple_of(2));
    }
}
