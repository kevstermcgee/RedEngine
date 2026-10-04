//! Music as data: a **score** is instruments, tracks and a few global settings, rendered to a seamless stereo loop at a stated loudness.
//!
//! ```json
//! { "bpm": 52, "bars": 8, "key": "D", "scale": "minor_pentatonic", "seed": 11, "lufs": -23,
//!   "reverb": { "decay": 7, "mix": 0.5 }, "delay": { "time": 0.75, "feedback": 0.4, "mix": 0.25 },
//!   "instruments": {
//!     "pad":  { "seconds": 16, "level": 0.5, "layers": [ { "sine": 1, "attack": 3, "release": 4 }, { "sine": 1.005, "attack": 4, "release": 4, "gain": 0.6 } ] },
//!     "bell": { "seconds": 6,  "level": 0.4, "layers": [ { "sine": 1, "harmonics": [[2.76, 0.25]], "decay": 1.3 } ] } },
//!   "tracks": [
//!     { "inst": "pad",  "play": "chords", "chords": "i VI III VII", "every": "2 bars", "octave": 3, "gain": 0.8 },
//!     { "inst": "bell", "play": "walk", "every": "1 beat", "density": 0.25, "range": [1, 8], "octave": 5, "pan": "spread", "send": 1 } ] }
//! ```
//!
//! An **instrument** is a [`Voice`] whose sine frequencies are multiples of the note played. A **track** plays one instrument one of three ways: `chords` (a
//! progression, `"i VI III VII"` or degrees, each chord the scale's own triad), `pattern` (a rhythm string such as `"x.x. ..x."` with scale `notes` cycled across
//! the hits) or `walk` (a seeded random walk through the scale, sparse and endless: the generative one). Notes are **scale degrees** (1 is the key, 8 the octave
//! of a seven-note scale), so everything stays in tune. Tracks pan (a number or `"spread"`), send to the effects by some amount, and may enter and leave at
//! given bars. The result is rendered as a loop of exactly `bars`, note tails and effect tails wrapped round the end, then scaled to `lufs`.

use crate::audio_analysis::{lufs, Clip};
use crate::audio_fx::{Delay, Reverb, Stereo};
use crate::dsp::{Noise, Voice};
use crate::synth::SAMPLE_RATE;
use serde_json::Value;

/// Names of the scales and their semitones above the key.
pub const SCALES: &[(&str, &[i32])] = &[
    ("major", &[0, 2, 4, 5, 7, 9, 11]),
    ("minor", &[0, 2, 3, 5, 7, 8, 10]),
    ("dorian", &[0, 2, 3, 5, 7, 9, 10]),
    ("phrygian", &[0, 1, 3, 5, 7, 8, 10]),
    ("lydian", &[0, 2, 4, 6, 7, 9, 11]),
    ("mixolydian", &[0, 2, 4, 5, 7, 9, 10]),
    ("harmonic_minor", &[0, 2, 3, 5, 7, 8, 11]),
    ("pentatonic", &[0, 2, 4, 7, 9]),
    ("minor_pentatonic", &[0, 3, 5, 7, 10]),
    ("blues", &[0, 3, 5, 6, 7, 10]),
    ("whole_tone", &[0, 2, 4, 6, 8, 10]),
    ("chromatic", &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]),
];

const TOP_KEYS: &[&str] = &["bpm", "beats", "bars", "key", "scale", "seed", "lufs", "reverb", "delay", "instruments", "tracks"];
const TRACK_KEYS: &[&str] = &[
    "inst", "play", "chords", "every", "voicing", "strum", "pattern", "notes", "steps", "density", "range", "leap", "start", "octave", "hold", "gain", "pan",
    "send", "vel", "from", "to", "seed",
];
const REVERB_KEYS: &[&str] = &["decay", "size", "damp", "predelay", "mix"];
const DELAY_KEYS: &[&str] = &["time", "feedback", "damp", "mix"];
const MAX_BARS: u32 = 64;
const MAX_NOTES: usize = 4000;
const MAX_SECONDS: f32 = 180.0;

/// Where a track's notes come from.
#[derive(Debug, Clone, PartialEq)]
pub enum Play {
    /// A chord progression: each chord's root degree, held for `every` beats, voiced on the scale as `voicing` degrees (default 1 3 5) with `strum` beats between notes.
    Chords {
        /// Root degree of each chord.
        roots: Vec<i32>,
        /// Beats per chord.
        every: f32,
        /// Which scale steps of the chord sound, counted from its root.
        voicing: Vec<i32>,
        /// Beats between the notes of a chord.
        strum: f32,
    },
    /// A rhythm: `steps` per bar of which `hits` sound, with `notes` (degrees) cycled across the hits.
    Pattern {
        /// Per step, whether it sounds.
        hits: Vec<bool>,
        /// Degrees, cycled across the hits.
        notes: Vec<i32>,
    },
    /// A seeded random walk through the scale: every `every` beats, with probability `density`, move at most `leap` degrees within `range` and play.
    Walk {
        /// Beats between chances to play.
        every: f32,
        /// Chance of playing at each.
        density: f32,
        /// Lowest and highest degree.
        range: (i32, i32),
        /// Largest step between notes, in degrees.
        leap: i32,
        /// Where the walk starts.
        start: i32,
    },
}

/// Where a track sits in the stereo field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pan {
    /// A fixed position, -1 (left) to 1 (right).
    Fixed(f32),
    /// Each note somewhere within this distance of the centre, chosen by the score's seed.
    Spread(f32),
}

/// One track.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    /// Index into [`Score::instruments`].
    pub inst: usize,
    /// How its notes are made.
    pub play: Play,
    /// Octave of degree 1 (middle C is the start of octave 4).
    pub octave: i32,
    /// Gate length in beats; `None` lets each note ring for the instrument's own length.
    pub hold: Option<f32>,
    /// Loudness of the track in the mix.
    pub gain: f32,
    /// Stereo position.
    pub pan: Pan,
    /// How much of it goes to the delay and reverb, 0 to 1.
    pub send: f32,
    /// Range of note loudness, chosen per note.
    pub vel: (f32, f32),
    /// First beat it plays.
    pub from_beat: f32,
    /// Beat it stops at (exclusive).
    pub to_beat: f32,
    /// Extra seed for this track.
    pub seed: u32,
}

/// A score.
#[derive(Debug, Clone)]
pub struct Score {
    /// Beats per minute.
    pub bpm: f32,
    /// Beats in a bar.
    pub beats: u32,
    /// Bars in the loop.
    pub bars: u32,
    /// Pitch class of the key, 0 (C) to 11 (B).
    pub key: i32,
    /// Semitones of the scale above the key.
    pub scale: Vec<i32>,
    /// Seed of everything random.
    pub seed: u32,
    /// The integrated loudness the loop is scaled to.
    pub lufs: f32,
    /// The reverb, if any.
    pub reverb: Option<Reverb>,
    /// The delay, if any.
    pub delay: Option<Delay>,
    /// Instruments by name.
    pub instruments: Vec<(String, Voice)>,
    /// The tracks.
    pub tracks: Vec<Track>,
}

/// One note to play.
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    /// Index of the track.
    pub track: usize,
    /// Start, in beats from the top of the loop.
    pub beat: f32,
    /// Pitch as a MIDI note (69 is A4).
    pub midi: i32,
    /// Loudness 0..1.
    pub vel: f32,
    /// Stereo position -1..1.
    pub pan: f32,
}

/// A rendered loop.
#[derive(Debug, Clone)]
pub struct Rendered {
    /// Interleaved stereo.
    pub samples: Vec<f32>,
    /// Notes played.
    pub notes: usize,
    /// Loudness reached, LUFS.
    pub lufs: f32,
    /// True when the loop had to be turned down below its target to keep its peak under full scale.
    pub limited: bool,
}

fn near(name: &str, options: &[&str]) -> String {
    match crate::prefabs::suggest(name, options.iter().copied()).first() {
        Some(h) => format!(" — did you mean `{h}`?"),
        None => String::new(),
    }
}

/// `1 bar`, `2 bars`, `0.5 beat`, `3 steps` (of `steps_per_bar`) or a bare number of beats, as beats.
fn time_beats(v: &Value, beats_per_bar: u32, steps_per_bar: u32, path: &str, errs: &mut Vec<String>) -> Option<f32> {
    let (n, unit) = match v {
        Value::Number(n) => (n.as_f64()? as f32, "beats".to_string()),
        Value::String(s) => {
            let mut it = s.split_whitespace();
            let n = it.next().and_then(|n| n.parse::<f32>().ok());
            match (n, it.next()) {
                (Some(n), Some(u)) => (n, u.to_string()),
                (Some(n), None) => (n, "beats".to_string()),
                _ => {
                    errs.push(format!("{path}: `{s}` is not a time (like \"2 bars\", \"1 beat\" or a number of beats)"));
                    return None;
                }
            }
        }
        _ => {
            errs.push(format!("{path}: must be a time like \"2 bars\" or a number of beats"));
            return None;
        }
    };
    let b = match unit.trim_end_matches('s') {
        "bar" => n * beats_per_bar as f32,
        "beat" => n,
        "step" => n * beats_per_bar as f32 / steps_per_bar.max(1) as f32,
        other => {
            errs.push(format!("{path}: unit `{other}` is not one of bar, beat, step"));
            return None;
        }
    };
    if b <= 0.0 || !b.is_finite() {
        errs.push(format!("{path}: must be longer than zero"));
        return None;
    }
    Some(b)
}

fn key_class(name: &str) -> Option<i32> {
    let mut chars = name.chars();
    let base: i32 = match chars.next()?.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let shift = match chars.as_str() {
        "" => 0,
        "#" => 1,
        "b" => -1,
        _ => return None,
    };
    Some((base + shift).rem_euclid(12))
}

fn degrees(v: &Value, path: &str, errs: &mut Vec<String>) -> Vec<i32> {
    let parse = |s: &str, errs: &mut Vec<String>| -> Option<i32> {
        let roman = ["i", "ii", "iii", "iv", "v", "vi", "vii"].iter().position(|r| r.eq_ignore_ascii_case(s)).map(|p| p as i32 + 1);
        let n = roman.or_else(|| s.parse::<i32>().ok());
        if n.is_none() {
            errs.push(format!("{path}: `{s}` is not a scale degree (a number, or a numeral I to VII)"));
        }
        n
    };
    match v {
        Value::String(s) => s.split(|c: char| c.is_whitespace() || c == ',').filter(|t| !t.is_empty()).filter_map(|t| parse(t, errs)).collect(),
        Value::Array(a) => a
            .iter()
            .filter_map(|x| match x {
                Value::Number(n) => n.as_i64().map(|n| n as i32),
                Value::String(s) => parse(s, errs),
                _ => None,
            })
            .collect(),
        Value::Number(n) => n.as_i64().map(|n| vec![n as i32]).unwrap_or_default(),
        _ => {
            errs.push(format!("{path}: must be degrees like \"1 3 5\" or [1, 3, 5]"));
            Vec::new()
        }
    }
}

fn num(o: &serde_json::Map<String, Value>, key: &str, default: f32, lo: f32, hi: f32, path: &str, errs: &mut Vec<String>) -> f32 {
    match o.get(key) {
        None => default,
        Some(v) => match v.as_f64().map(|f| f as f32) {
            Some(x) if (lo..=hi).contains(&x) => x,
            Some(x) => {
                errs.push(format!("{path}.{key}: {x} is outside {lo} to {hi}"));
                default
            }
            None => {
                errs.push(format!("{path}.{key}: must be a number"));
                default
            }
        },
    }
}

fn check_keys(o: &serde_json::Map<String, Value>, allowed: &[&str], path: &str, errs: &mut Vec<String>) {
    for k in o.keys().filter(|k| !allowed.contains(&k.as_str())) {
        let at = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
        errs.push(format!("{at}: unknown field{} (fields: {})", near(k, allowed), allowed.join(", ")));
    }
}

/// Parses a score; every problem is reported with its path.
pub fn parse_score(v: &Value) -> Result<Score, Vec<String>> {
    let Some(o) = v.as_object() else {
        return Err(vec!["a score is an object like {\"bpm\": 60, \"bars\": 4, \"instruments\": {...}, \"tracks\": [...]}".to_string()]);
    };
    let mut errs = Vec::new();
    check_keys(o, TOP_KEYS, "", &mut errs);
    let bpm = num(o, "bpm", 60.0, 20.0, 300.0, "", &mut errs);
    let beats = num(o, "beats", 4.0, 1.0, 16.0, "", &mut errs) as u32;
    let bars = num(o, "bars", 4.0, 1.0, MAX_BARS as f32, "", &mut errs) as u32;
    if bars as f32 * beats as f32 * 60.0 / bpm > MAX_SECONDS {
        errs.push(format!("bars: {bars} bars of {beats} at {bpm} bpm is over {MAX_SECONDS} seconds (a loop, not a song)"));
    }
    let key = match o.get("key").and_then(Value::as_str) {
        None => 0,
        Some(k) => key_class(k).unwrap_or_else(|| {
            errs.push(format!("key: `{k}` is not a key (C, C#, Db, D ... B)"));
            0
        }),
    };
    let scale = match o.get("scale").and_then(Value::as_str) {
        None => SCALES[1].1.to_vec(),
        Some(name) => match SCALES.iter().find(|(n, _)| *n == name) {
            Some((_, s)) => s.to_vec(),
            None => {
                let names: Vec<&str> = SCALES.iter().map(|(n, _)| *n).collect();
                errs.push(format!("scale: `{name}` is not a scale{} (scales: {})", near(name, &names), names.join(", ")));
                SCALES[1].1.to_vec()
            }
        },
    };
    let seed = num(o, "seed", 0.0, 0.0, 4294967295.0, "", &mut errs) as u32;
    let lufs = num(o, "lufs", -23.0, -50.0, -6.0, "", &mut errs);
    let reverb = o.get("reverb").filter(|r| !r.is_null()).and_then(|r| match r.as_object() {
        Some(r) => {
            check_keys(r, REVERB_KEYS, "reverb", &mut errs);
            Some(Reverb {
                decay: num(r, "decay", 4.0, 0.2, 30.0, "reverb", &mut errs),
                size: num(r, "size", 1.0, 0.3, 3.0, "reverb", &mut errs),
                damp: num(r, "damp", 0.4, 0.0, 0.98, "reverb", &mut errs),
                predelay: num(r, "predelay", 0.02, 0.0, 0.5, "reverb", &mut errs),
                mix: num(r, "mix", 0.3, 0.0, 3.0, "reverb", &mut errs),
            })
        }
        None => {
            errs.push("reverb: must be an object like {\"decay\": 6, \"mix\": 0.4}".to_string());
            None
        }
    });
    let delay = o.get("delay").filter(|d| !d.is_null()).and_then(|d| match d.as_object() {
        Some(d) => {
            check_keys(d, DELAY_KEYS, "delay", &mut errs);
            let beats_time = num(d, "time", 0.75, 0.05, 8.0, "delay", &mut errs);
            Some(Delay {
                time: beats_time * 60.0 / bpm,
                feedback: num(d, "feedback", 0.35, 0.0, 0.95, "delay", &mut errs),
                damp: num(d, "damp", 0.4, 0.0, 0.98, "delay", &mut errs),
                mix: num(d, "mix", 0.25, 0.0, 3.0, "delay", &mut errs),
            })
        }
        None => {
            errs.push("delay: must be an object like {\"time\": 0.75, \"feedback\": 0.4} (time in beats)".to_string());
            None
        }
    });
    let mut instruments: Vec<(String, Voice)> = Vec::new();
    match o.get("instruments").and_then(Value::as_object) {
        Some(m) if !m.is_empty() => {
            for (name, iv) in m {
                // An instrument is a pitched voice: say so for the author.
                let mut spec = iv.clone();
                if let Some(obj) = spec.as_object_mut() {
                    obj.insert("pitched".into(), Value::Bool(true));
                }
                match crate::voice_spec::parse_voice_up_to(&spec, MAX_SECONDS) {
                    Ok(voice) => instruments.push((name.clone(), voice)),
                    Err(es) => errs.extend(es.into_iter().map(|e| format!("instruments.{name}.{e}"))),
                }
            }
        }
        _ => errs.push("instruments: an object of named instruments (voices whose sine values are multiples of the note)".to_string()),
    }
    let mut tracks = Vec::new();
    match o.get("tracks").and_then(Value::as_array) {
        Some(list) if !list.is_empty() => {
            for (i, t) in list.iter().enumerate() {
                if let Some(track) = parse_track(t, i, &instruments, beats, bars, &mut errs) {
                    tracks.push(track);
                }
            }
        }
        _ => errs.push("tracks: a list of tracks like {\"inst\": \"pad\", \"play\": \"chords\", \"chords\": \"i VI\"}".to_string()),
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    let score = Score { bpm, beats, bars, key, scale, seed, lufs, reverb, delay, instruments, tracks };
    let n = score.notes().len();
    if n > MAX_NOTES {
        return Err(vec![format!("tracks: {n} notes is more than the {MAX_NOTES} a loop may have (thin a pattern, lower a density, lengthen `every`)")]);
    }
    Ok(score)
}

fn parse_track(t: &Value, i: usize, instruments: &[(String, Voice)], beats: u32, bars: u32, errs: &mut Vec<String>) -> Option<Track> {
    let path = format!("tracks[{i}]");
    let Some(o) = t.as_object() else {
        errs.push(format!("{path}: must be an object"));
        return None;
    };
    check_keys(o, TRACK_KEYS, &path, errs);
    let names: Vec<&str> = instruments.iter().map(|(n, _)| n.as_str()).collect();
    let inst = match o.get("inst").and_then(Value::as_str) {
        Some(n) => match names.iter().position(|x| *x == n) {
            Some(p) => p,
            None => {
                errs.push(format!("{path}.inst: no instrument `{n}`{} (instruments: {})", near(n, &names), names.join(", ")));
                return None;
            }
        },
        None => {
            errs.push(format!("{path}.inst: missing (an instrument name: {})", names.join(", ")));
            return None;
        }
    };
    let pattern_steps =
        o.get("pattern").and_then(Value::as_str).map(|p| p.chars().filter(|c| !c.is_whitespace() && *c != '|').count() as u32).unwrap_or(16).max(1);
    let steps = num(o, "steps", pattern_steps as f32, 1.0, 64.0, &path, errs) as u32;
    let time = |key: &str, errs: &mut Vec<String>| o.get(key).and_then(|v| time_beats(v, beats, steps, &format!("{path}.{key}"), errs));
    let play = match o.get("play").and_then(Value::as_str) {
        Some("chords") => {
            let roots = o.get("chords").map(|c| degrees(c, &format!("{path}.chords"), errs)).unwrap_or_default();
            if roots.is_empty() {
                errs.push(format!("{path}.chords: a progression like \"i VI III VII\""));
                return None;
            }
            let every = time("every", errs).unwrap_or(beats as f32);
            let voicing = o.get("voicing").map(|v| degrees(v, &format!("{path}.voicing"), errs)).unwrap_or_else(|| vec![1, 3, 5]);
            Play::Chords { roots, every, voicing, strum: num(o, "strum", 0.0, 0.0, 4.0, &path, errs) }
        }
        Some("pattern") => {
            let Some(p) = o.get("pattern").and_then(Value::as_str) else {
                errs.push(format!("{path}.pattern: a rhythm like \"x.x. ..x.\" (x sounds, . rests)"));
                return None;
            };
            let hits: Vec<bool> = p.chars().filter(|c| !c.is_whitespace() && *c != '|').map(|c| matches!(c, 'x' | 'X')).collect();
            let notes = o.get("notes").map(|n| degrees(n, &format!("{path}.notes"), errs)).unwrap_or_else(|| vec![1]);
            if notes.is_empty() {
                errs.push(format!("{path}.notes: at least one scale degree"));
                return None;
            }
            Play::Pattern { hits, notes }
        }
        Some("walk") => {
            let range = match o.get("range").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_i64).collect::<Vec<_>>()) {
                Some(r) if r.len() == 2 && r[0] <= r[1] => (r[0] as i32, r[1] as i32),
                None => (1, 8),
                _ => {
                    errs.push(format!("{path}.range: [lowest, highest] scale degree, like [1, 8]"));
                    (1, 8)
                }
            };
            let leap = num(o, "leap", 2.0, 1.0, 12.0, &path, errs) as i32;
            let start = num(o, "start", ((range.0 + range.1) / 2) as f32, -40.0, 40.0, &path, errs) as i32;
            Play::Walk { every: time("every", errs).unwrap_or(1.0), density: num(o, "density", 0.3, 0.0, 1.0, &path, errs), range, leap, start }
        }
        Some(other) => {
            errs.push(format!("{path}.play: `{other}` is not one of chords, pattern, walk{}", near(other, &["chords", "pattern", "walk"])));
            return None;
        }
        None => {
            errs.push(format!("{path}.play: missing (chords, pattern or walk)"));
            return None;
        }
    };
    let pan = match o.get("pan") {
        None => Pan::Fixed(0.0),
        Some(Value::String(s)) if s == "spread" => Pan::Spread(0.7),
        Some(Value::Number(n)) => Pan::Fixed(n.as_f64().unwrap_or(0.0).clamp(-1.0, 1.0) as f32),
        Some(_) => {
            errs.push(format!("{path}.pan: a number from -1 (left) to 1 (right), or \"spread\""));
            Pan::Fixed(0.0)
        }
    };
    let vel = match o.get("vel").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).collect::<Vec<_>>()) {
        None => (0.75, 1.0),
        Some(v) if v.len() == 2 && v[0] <= v[1] && v[0] >= 0.0 && v[1] <= 1.0 => (v[0] as f32, v[1] as f32),
        _ => {
            errs.push(format!("{path}.vel: [lowest, highest] loudness between 0 and 1"));
            (0.75, 1.0)
        }
    };
    let total_bars = bars as f32;
    let from = num(o, "from", 1.0, 1.0, total_bars, &path, errs);
    let to = num(o, "to", total_bars, from, total_bars, &path, errs);
    let hold = time("hold", errs).or(match &play {
        Play::Chords { every, .. } => Some(*every),
        _ => None,
    });
    Some(Track {
        inst,
        play,
        octave: num(o, "octave", 4.0, -1.0, 8.0, &path, errs) as i32,
        hold,
        gain: num(o, "gain", 1.0, 0.0, 4.0, &path, errs),
        pan,
        send: num(o, "send", 1.0, 0.0, 1.0, &path, errs),
        vel,
        from_beat: (from - 1.0) * beats as f32,
        to_beat: to * beats as f32,
        seed: num(o, "seed", 0.0, 0.0, 4294967295.0, &path, errs) as u32,
    })
}

impl Score {
    /// Seconds in one beat.
    pub fn beat_secs(&self) -> f32 {
        60.0 / self.bpm
    }

    /// Frames in the loop.
    pub fn loop_frames(&self) -> usize {
        (self.bars as f32 * self.beats as f32 * self.beat_secs() * SAMPLE_RATE as f32).round() as usize
    }

    /// MIDI note of scale `degree` (1 is the key) in `octave`.
    pub fn midi(&self, degree: i32, octave: i32) -> i32 {
        let n = self.scale.len() as i32;
        let d = degree - 1;
        12 * (octave + 1) + self.key + self.scale[d.rem_euclid(n) as usize] + 12 * d.div_euclid(n)
    }

    /// Every note of every track, in time order.
    pub fn notes(&self) -> Vec<Note> {
        let total = self.bars as f32 * self.beats as f32;
        let mut out = Vec::new();
        for (ti, t) in self.tracks.iter().enumerate() {
            let mut rng = Noise(self.seed ^ t.seed ^ (ti as u32 + 1).wrapping_mul(0x9E37_79B1) ^ 0x5EED);
            let mut unit = move || (rng.white() + 1.0) * 0.5;
            let (lo, hi) = (t.from_beat, t.to_beat.min(total));
            let vel = |u: &mut dyn FnMut() -> f32| t.vel.0 + (t.vel.1 - t.vel.0) * u();
            let pan = |u: &mut dyn FnMut() -> f32| match t.pan {
                Pan::Fixed(p) => p,
                Pan::Spread(w) => (u() * 2.0 - 1.0) * w,
            };
            match &t.play {
                Play::Chords { roots, every, voicing, strum } => {
                    let mut k = 0;
                    loop {
                        let at = lo + k as f32 * every;
                        if at >= hi {
                            break;
                        }
                        let root = roots[k % roots.len()];
                        for (j, v) in voicing.iter().enumerate() {
                            out.push(Note {
                                track: ti,
                                beat: at + j as f32 * strum,
                                midi: self.midi(root + v - 1, t.octave),
                                vel: vel(&mut unit),
                                pan: pan(&mut unit),
                            });
                        }
                        k += 1;
                    }
                }
                Play::Pattern { hits, notes } => {
                    let step = self.beats as f32 / hits.len() as f32;
                    let mut hit = 0;
                    let mut bar = ((lo / self.beats as f32).floor() as i32).max(0);
                    while (bar as f32) * (self.beats as f32) < hi {
                        for (s, on) in hits.iter().enumerate() {
                            let at = bar as f32 * self.beats as f32 + s as f32 * step;
                            if *on && at >= lo && at < hi {
                                out.push(Note {
                                    track: ti,
                                    beat: at,
                                    midi: self.midi(notes[hit % notes.len()], t.octave),
                                    vel: vel(&mut unit),
                                    pan: pan(&mut unit),
                                });
                                hit += 1;
                            }
                        }
                        bar += 1;
                    }
                }
                Play::Walk { every, density, range, leap, start } => {
                    let mut degree = (*start).clamp(range.0, range.1);
                    let mut k = 0;
                    loop {
                        let at = lo + k as f32 * every;
                        if at >= hi {
                            break;
                        }
                        if unit() < *density {
                            let mut d = 1 + (unit() * *leap as f32) as i32;
                            if unit() < 0.5 {
                                d = -d;
                            }
                            // Reflect off the ends of the range instead of sticking to them.
                            let mut next = degree + d;
                            if next < range.0 || next > range.1 {
                                next = degree - d;
                            }
                            degree = next.clamp(range.0, range.1);
                            out.push(Note { track: ti, beat: at, midi: self.midi(degree, t.octave), vel: vel(&mut unit), pan: pan(&mut unit) });
                        }
                        k += 1;
                    }
                }
            }
        }
        out.sort_by(|a, b| a.beat.total_cmp(&b.beat));
        out
    }

    /// Renders the loop: notes mixed with their tails wrapped round the end, run through the delay and reverb over enough passes that the last is the steady
    /// state (so the loop has no seam), then scaled to `lufs`.
    pub fn render(&self) -> Rendered {
        let n = self.loop_frames();
        let (mut dry_l, mut dry_r, mut send_l, mut send_r) = (vec![0.0f32; n], vec![0.0f32; n], vec![0.0f32; n], vec![0.0f32; n]);
        let notes = self.notes();
        for (idx, note) in notes.iter().enumerate() {
            let t = &self.tracks[note.track];
            let hz = crate::dsp::hz(note.midi);
            let hold = t.hold.map(|b| b * self.beat_secs());
            let wave = self.instruments[t.inst].1.render_note(hz, hold, idx as u32 + 1);
            let angle = (note.pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
            let (gl, gr) = (angle.cos() * t.gain * note.vel, angle.sin() * t.gain * note.vel);
            let start = (note.beat * self.beat_secs() * SAMPLE_RATE as f32).round() as usize;
            for (k, s) in wave.iter().enumerate() {
                let at = (start + k) % n;
                dry_l[at] += s * gl;
                dry_r[at] += s * gr;
                send_l[at] += s * gl * t.send;
                send_r[at] += s * gr * t.send;
            }
        }
        // The dry signal is everything; the send adds the effects on top.
        let (mut out_l, mut out_r) = (dry_l, dry_r);
        if self.reverb.is_some() || self.delay.is_some() {
            let tail = self
                .reverb
                .map_or(0.0, |r| r.decay * 1.2 + r.predelay)
                .max(self.delay.map_or(0.0, |d| d.time * (0.001f32.ln() / d.feedback.max(0.05).ln()).min(40.0)));
            let loop_secs = n as f32 / SAMPLE_RATE as f32;
            let passes = (2 + (tail / loop_secs).ceil() as usize).min(8);
            let rep = |v: &Vec<f32>| -> Vec<f32> { (0..passes).flat_map(|_| v.iter().copied()).collect() };
            let sent: Stereo = (rep(&send_l), rep(&send_r));
            let mut wet_l = vec![0.0f32; sent.0.len()];
            let mut wet_r = vec![0.0f32; sent.0.len()];
            let mut reverb_in = sent.clone();
            if let Some(d) = &self.delay {
                let echoes = d.process(&sent);
                for i in 0..wet_l.len() {
                    wet_l[i] += echoes.0[i];
                    wet_r[i] += echoes.1[i];
                    reverb_in.0[i] += echoes.0[i];
                    reverb_in.1[i] += echoes.1[i];
                }
            }
            if let Some(r) = &self.reverb {
                let tailed = r.process(&reverb_in);
                for i in 0..wet_l.len() {
                    wet_l[i] += tailed.0[i];
                    wet_r[i] += tailed.1[i];
                }
            }
            let from = n * (passes - 1);
            for i in 0..n {
                out_l[i] += wet_l[from + i];
                out_r[i] += wet_r[from + i];
            }
        }
        let mut samples: Vec<f32> = out_l.iter().zip(&out_r).flat_map(|(l, r)| [*l, *r]).collect();
        let mut gain = 1.0;
        let reached = lufs(Clip { samples: &samples, channels: 2, rate: SAMPLE_RATE });
        if reached > -100.0 {
            gain = 10f32.powf((self.lufs - reached) / 20.0);
        }
        let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs())) * gain;
        let limited = peak > 0.98;
        if limited {
            gain *= 0.98 / peak;
        }
        samples.iter_mut().for_each(|s| *s *= gain);
        let lufs_out = lufs(Clip { samples: &samples, channels: 2, rate: SAMPLE_RATE });
        Rendered { samples, notes: notes.len(), lufs: lufs_out, limited }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_analysis::{analyze, note_name, Clip};
    use serde_json::json;

    fn tiny(extra: Value) -> Value {
        let mut v = json!({
            "bpm": 120, "bars": 2, "key": "A", "scale": "minor", "seed": 3, "lufs": -23,
            "instruments": { "pluck": { "seconds": 1.5, "level": 0.5, "layers": [ { "sine": 1, "decay": 4 } ] },
                             "pad": { "seconds": 4, "level": 0.5, "layers": [ { "sine": 1, "attack": 0.4, "release": 0.6 }, { "sine": 2, "gain": 0.3, "attack": 0.4, "release": 0.6 } ] } },
            "tracks": [ { "inst": "pluck", "play": "pattern", "pattern": "x..x ..x.", "notes": "1 3 5" } ] });
        if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
            e.iter().for_each(|(k, x)| {
                o.insert(k.clone(), x.clone());
            });
        }
        v
    }

    fn score(v: Value) -> Score {
        parse_score(&v).unwrap_or_else(|e| panic!("{e:?}"))
    }

    fn report(r: &Rendered) -> crate::audio_analysis::Report {
        analyze(Clip { samples: &r.samples, channels: 2, rate: SAMPLE_RATE })
    }

    #[test]
    fn scale_degrees_become_notes_in_the_key_and_wrap_up_the_octaves() {
        let s = score(tiny(json!({})));
        assert_eq!((s.midi(1, 4), s.midi(3, 4), s.midi(5, 4), s.midi(8, 4)), (69, 72, 76, 81), "A minor: A C E and the octave");
        assert_eq!(s.midi(0, 4), 67, "the degree below the key is the seventh below");
        let pent = score(tiny(json!({ "scale": "minor_pentatonic", "key": "C" })));
        assert_eq!((pent.midi(1, 4), pent.midi(5, 4), pent.midi(6, 4)), (60, 70, 72), "five notes, the sixth degree is the octave");
        assert_eq!(score(tiny(json!({ "key": "Bb" }))).key, 10);
        assert_eq!(score(tiny(json!({ "key": "F#" }))).key, 6);
    }

    #[test]
    fn chords_are_the_scales_own_triads_one_per_step_of_the_progression() {
        let v = tiny(json!({ "bars": 4, "tracks": [ { "inst": "pad", "play": "chords", "chords": "i VI III VII", "every": "1 bar", "octave": 4 } ] }));
        let s = score(v);
        let n = s.notes();
        let at = |bar: f32| n.iter().filter(|x| (x.beat - bar * 4.0).abs() < 1e-3).map(|x| x.midi).collect::<Vec<_>>();
        assert_eq!(at(0.0), vec![69, 72, 76], "Am");
        assert_eq!(at(1.0), vec![77, 81, 84], "F");
        assert_eq!(at(2.0), vec![72, 76, 79], "C");
        assert_eq!(at(3.0), vec![79, 83, 86], "G");
        // Digits and numerals mean the same; strum staggers the notes.
        let strummed = score(tiny(json!({ "tracks": [ { "inst": "pad", "play": "chords", "chords": [1, 6], "every": "1 bar", "strum": 0.25 } ] })));
        assert_eq!(strummed.notes().iter().take(3).map(|x| x.beat).collect::<Vec<_>>(), vec![0.0, 0.25, 0.5]);
    }

    #[test]
    fn a_pattern_plays_its_hits_each_bar_with_the_notes_cycled_across_them() {
        let s = score(tiny(json!({})));
        let n = s.notes();
        assert_eq!(n.len(), 6, "three hits a bar, two bars");
        assert_eq!(n.iter().map(|x| x.midi).collect::<Vec<_>>(), vec![69, 72, 76, 69, 72, 76], "1 3 5 across the hits, continuing over the bar line");
        assert_eq!(n.iter().map(|x| x.beat).collect::<Vec<_>>(), vec![0.0, 1.5, 3.0, 4.0, 5.5, 7.0], "x..x ..x. over a 4 beat bar: eighths of 0.5 beat");
        let entering = score(tiny(json!({ "bars": 4, "tracks": [ { "inst": "pluck", "play": "pattern", "pattern": "x...", "from": 3, "to": 3 } ] })));
        assert_eq!(entering.notes().iter().map(|x| x.beat).collect::<Vec<_>>(), vec![8.0], "only bar 3");
    }

    #[test]
    fn a_walk_is_seeded_stays_in_its_range_steps_by_at_most_its_leap_and_thins_with_density() {
        let walk = |seed: u32, density: f32| {
            score(tiny(
                json!({ "bars": 16, "seed": seed, "tracks": [ { "inst": "pluck", "play": "walk", "every": "1 beat", "density": density, "range": [1, 8], "leap": 2, "octave": 4 } ] }),
            ))
        };
        let a = walk(1, 0.5);
        assert_eq!(a.notes(), a.notes(), "deterministic");
        assert_ne!(
            a.notes().iter().map(|n| n.midi).collect::<Vec<_>>(),
            walk(2, 0.5).notes().iter().map(|n| n.midi).collect::<Vec<_>>(),
            "another seed, another line"
        );
        let degrees: Vec<i32> = a.notes().iter().map(|n| (0..=20).find(|d| a.midi(*d, 4) == n.midi).unwrap()).collect();
        assert!(degrees.iter().all(|d| (1..=8).contains(d)), "{degrees:?}");
        assert!(degrees.windows(2).all(|w| (w[1] - w[0]).abs() <= 2), "never leaps more than 2 degrees: {degrees:?}");
        let (sparse, dense) = (walk(1, 0.1).notes().len(), walk(1, 0.9).notes().len());
        assert!(sparse < 12 && dense > 40, "density thins the line: {sparse} vs {dense}");
        assert_eq!(walk(1, 0.0).notes().len(), 0);
    }

    #[test]
    fn a_rendered_loop_is_stereo_exactly_as_long_as_its_bars_and_deterministic() {
        let s = score(tiny(json!({})));
        let r = s.render();
        assert_eq!(r.samples.len(), s.loop_frames() * 2);
        assert_eq!(s.loop_frames(), (2.0 * 4.0 * 0.5 * SAMPLE_RATE as f32) as usize, "2 bars of 4 beats at 120 bpm is 4 s");
        assert_eq!(r.notes, 6);
        assert_eq!(r.samples, s.render().samples);
    }

    #[test]
    fn a_score_lands_on_its_stated_loudness_and_is_turned_down_rather_than_clipped() {
        for target in [-30.0f32, -23.0, -18.0] {
            let r = score(tiny(json!({ "lufs": target }))).render();
            assert!((r.lufs - target).abs() < 0.3 && !r.limited, "asked {target}, got {} (limited {})", r.lufs, r.limited);
        }
        let hot = score(tiny(json!({ "lufs": -6, "tracks": [ { "inst": "pluck", "play": "pattern", "pattern": "x...........", "notes": "1" } ] }))).render();
        assert!(hot.limited && hot.lufs < -6.0, "too loud a target for a peaky sound is limited, not clipped: {} {}", hot.lufs, hot.limited);
        assert!(hot.samples.iter().all(|s| s.abs() <= 0.981));
    }

    #[test]
    fn the_loop_has_no_seam_even_with_a_long_reverb_and_a_delay_wrapping_round_the_end() {
        let wet = score(tiny(json!({
            "reverb": { "decay": 5, "mix": 0.6 }, "delay": { "time": 0.5, "feedback": 0.5, "mix": 0.4 },
            "tracks": [ { "inst": "pad", "play": "chords", "chords": "i VII", "every": "1 bar", "octave": 3 },
                        { "inst": "pluck", "play": "pattern", "pattern": "x...x...", "notes": "1 5", "octave": 5 } ] })));
        let r = wet.render();
        let rep = report(&r);
        assert!(rep.seam < 3.0, "the end meets the start: {}", rep.seam);
        // The last note of the loop rings into the start: there is sound in the first moments even though nothing is struck there.
        let quiet = score(tiny(
            json!({ "tracks": [ { "inst": "pluck", "play": "pattern", "pattern": "......x.", "notes": "1" } ], "reverb": { "decay": 3, "mix": 0.5 } }),
        ))
        .render();
        let first_100ms: f32 = quiet.samples[..4410 * 2].iter().map(|s| s * s).sum();
        assert!(first_100ms > 1e-6, "a tail wrapped round from the end of the loop");
    }

    #[test]
    fn panning_places_a_track_in_the_stereo_field_and_spread_varies_per_note() {
        let side = |pan: Value| {
            let r = score(tiny(json!({ "tracks": [ { "inst": "pluck", "play": "pattern", "pattern": "x...x...", "pan": pan } ] }))).render();
            let (l, rr) = r.samples.chunks(2).fold((0.0f32, 0.0f32), |(a, b), f| (a + f[0] * f[0], b + f[1] * f[1]));
            (l, rr)
        };
        let (l, r) = side(json!(-1));
        assert!(l > r * 100.0, "hard left: {l} {r}");
        let (l, r) = side(json!(1));
        assert!(r > l * 100.0, "hard right: {l} {r}");
        let (l, r) = side(json!(0));
        assert!((l / r - 1.0).abs() < 0.01, "centre: equal power");
        let spread = score(tiny(json!({ "tracks": [ { "inst": "pluck", "play": "pattern", "pattern": "xxxxxxxx", "pan": "spread" } ] })));
        let pans: Vec<f32> = spread.notes().iter().map(|n| n.pan).collect();
        assert!(pans.iter().all(|p| p.abs() <= 0.7) && pans.windows(2).any(|w| w[0] != w[1]), "{pans:?}");
    }

    #[test]
    fn an_instrument_plays_at_the_pitch_of_the_note_and_holds_for_its_gate() {
        let s = score(tiny(
            json!({ "tracks": [ { "inst": "pad", "play": "chords", "chords": "1", "voicing": [1], "every": "2 bars", "octave": 4 } ], "reverb": null }),
        ));
        let _ = &s;
        let note = s.instruments[1].1.render_note(440.0, Some(1.0), 0);
        let r = analyze(Clip { samples: &note, channels: 1, rate: SAMPLE_RATE });
        assert!(note_name(r.dominant_hz).starts_with("A4"), "{}", note_name(r.dominant_hz));
        assert!((r.secs - 1.6).abs() < 0.01, "the gate (1 s) plus the release (0.6 s): {}", r.secs);
        let natural = s.instruments[0].1.render_note(440.0, None, 0);
        assert!((natural.len() as f32 / SAMPLE_RATE as f32 - 1.5).abs() < 0.01, "no gate: the instrument's own length");
    }

    #[test]
    fn every_mistake_is_named_with_its_path_and_a_suggestion() {
        let e = parse_score(&tiny(json!({
            "bpmm": 90, "key": "H", "scale": "minr", "reverb": { "decy": 4 }, "delay": 3,
            "tracks": [ { "inst": "plucc", "play": "chords", "chords": "i" }, { "inst": "pluck", "play": "chrds" }, { "inst": "pluck", "play": "pattern" },
                        { "inst": "pluck", "play": "chords", "chords": "i XX", "every": "2 fortnights" }, { "inst": "pluck", "play": "walk", "range": [9, 2], "pann": 1 },
                        { "inst": "pluck", "play": "pattern", "pattern": "x", "pan": "wide", "vel": [1, 0.2] } ] })))
        .unwrap_err()
        .join("\n");
        for want in [
            "bpmm: unknown field — did you mean `bpm`?",
            "key: `H` is not a key",
            "scale: `minr` is not a scale — did you mean `minor`?",
            "reverb.decy: unknown field — did you mean `decay`?",
            "delay: must be an object",
            "tracks[0].inst: no instrument `plucc` — did you mean `pluck`?",
            "tracks[1].play: `chrds` is not one of chords, pattern, walk — did you mean `chords`?",
            "tracks[2].pattern: a rhythm like",
            "tracks[3].chords: `XX` is not a scale degree",
            "tracks[3].every: unit `fortnight` is not one of bar, beat, step",
            "tracks[4].range: [lowest, highest]",
            "tracks[4].pann: unknown field — did you mean `pan`?",
            "tracks[5].pan: a number from -1",
            "tracks[5].vel: [lowest, highest]",
        ] {
            assert!(e.contains(want), "missing `{want}` in:\n{e}");
        }
        let bad_inst = parse_score(&tiny(json!({ "instruments": { "x": { "seconds": 1, "layers": [ { "sine": 99 } ] } } }))).unwrap_err().join("\n");
        assert!(bad_inst.contains("instruments.x.layers[0].sine: 99 is outside 0.1 to 64"), "an instrument's sine is a multiple of the note: {bad_inst}");
        assert!(parse_score(&tiny(json!({ "bars": 64, "bpm": 20 }))).unwrap_err().join(" ").contains("a loop, not a song"));
        assert!(parse_score(&json!([1])).unwrap_err()[0].contains("a score is an object"));
    }
}
