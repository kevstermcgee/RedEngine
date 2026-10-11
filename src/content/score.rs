//! A score (music as data: `assets/audio/*.json`, a scene's `audio/*.json`, a 2D game's `music`). Checker: `score::parse_score`.

use super::common::Any;
use super::voice::Voice;
use crate::strict_struct;
use std::collections::BTreeMap;

strict_struct! {
    /// A score: instruments, tracks and the global settings.
    pub struct Score {
        bpm: Any,
        beats: Any,
        bars: Any,
        key: String,
        scale: String,
        seed: Any,
        lufs: Any,
        reverb: ScoreReverb,
        delay: ScoreDelay,
        /// name -> voice.
        instruments: BTreeMap<String, Voice>,
        tracks: Vec<ScoreTrack>,
    }
}

strict_struct! {
    /// The score's reverb.
    pub struct ScoreReverb { decay: Any, size: Any, damp: Any, predelay: Any, mix: Any }
}

strict_struct! {
    /// The score's delay.
    pub struct ScoreDelay { time: Any, feedback: Any, damp: Any, mix: Any }
}

strict_struct! {
    /// One track: an instrument and what it plays.
    pub struct ScoreTrack {
        inst: String,
        play: String,
        chords: Any,
        every: Any,
        voicing: Any,
        strum: Any,
        pattern: Any,
        notes: Any,
        steps: Any,
        density: Any,
        range: Any,
        leap: Any,
        start: Any,
        octave: Any,
        hold: Any,
        gain: Any,
        pan: Any,
        send: Any,
        vel: Any,
        from: Any,
        to: Any,
        seed: Any,
    }
}
