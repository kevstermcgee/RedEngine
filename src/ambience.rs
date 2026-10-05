//! What a living world sounds like at this hour in this place: which beds are heard, which birds sing, which music is playing.
//!
//! A pure state machine. Each step it is told where the sun is (height, and whether it is rising or setting), the kind of country the listener is in and
//! how fast they are going; it answers with a gain for each background [`Bed`], a weight for each [`Mood`] of music, and the calls that start this moment
//! ([`Heard`]). The client turns those into sound; a headless run just reads them, so a script can assert that the dawn chorus is louder than noon and that
//! owls call only at night, without a sound card.
//!
//! Birds follow the real day: a chorus at first light (robins, blackbirds, wrens, chaffinches, all at once and close together), a scatter of song and the
//! occasional cuckoo or pigeon through the day, blackbirds and robins again at dusk, then quiet, crickets and an owl. Forests are noisier than fields; fields have
//! more wind; flower fields hum with bees at noon; the night is crickets and, now and then, one hoot. Everything slews, so nothing switches.

use crate::nature::Call;
use crate::procgen::noise::{smooth, Rng};
use crate::procgen::Biome;

/// `audio` keys.
pub const AUDIO_KEYS: &[&str] = &["ambience", "music", "music_volume", "ambience_volume", "reverb", "duck", "layers", "birds"];

/// A scene's `audio` block: the sounds of its world.
///
/// ```json
/// "audio": { "ambience": "nature", "music": { "dawn": "audio/dawn.json", "day": "audio/day.json", "dusk": "audio/dusk.json", "night": "audio/night.json" },
///            "music_volume": 0.6, "ambience_volume": 0.8 }
/// ```
/// `ambience: "nature"` plays the engine's countryside (wind, leaves, crickets, bees, birdsong, owls) as the hour and the place call for it; `music` names a
/// score (see `describe audio`) for each mood, which the hour crossfades between (any subset: a mood without a score is silent). Music is the player's `music`
/// setting, ambience their `sound` setting.
///
/// `"birds": false` takes the songbirds, cuckoo and pigeon out of the countryside (the beds and the owl stay).
///
/// Three more keys shape the mix: `"reverb": {"decay": 1.4, "mix": 0.25}` puts every nature call in a room; `"duck": {"events": ["hit"], "depth": 0.5, "hold": 1.0,
/// "release": 1.5}` pulls the music down whenever the rules raise one of those events; and `"layers": [{"score": "audio/chase.json", "var": "danger", "above": 0.5,
/// "fade": 3, "volume": 0.6}]` fades a score in while a rule variable is above a value (intensity that follows the game).
#[derive(Debug, Clone, PartialEq)]
pub struct AudioSpec {
    /// Whether the nature ambience plays.
    pub nature: bool,
    /// Whether birds sing (`audio.birds`, on unless false). The owl at night is not birdsong and stays.
    pub birds: bool,
    /// A score file for each mood, paths resolved against the scene's folder.
    pub music: Vec<(Mood, std::path::PathBuf)>,
    /// Music level, 0 to 1.
    pub music_volume: f32,
    /// Ambience level (beds and calls), 0 to 1.
    pub ambience_volume: f32,
    /// A room for the nature calls.
    pub reverb: Option<RoomSpec>,
    /// Ducking the music on game events.
    pub duck: Option<DuckSpec>,
    /// Scores that fade in and out with a rule variable.
    pub layers: Vec<LayerSpec>,
}

impl AudioSpec {
    /// Every file the block names (the mood scores and the layer scores), as the paths the scene resolved them to.
    pub fn files(&self) -> Vec<std::path::PathBuf> {
        self.music.iter().map(|(_, f)| f.clone()).chain(self.layers.iter().map(|l| l.score.clone())).collect()
    }
}

/// `audio.reverb`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoomSpec {
    /// Seconds for the tail to fall 60 dB.
    pub decay: f32,
    /// Wet level relative to the dry sound.
    pub mix: f32,
}

impl RoomSpec {
    /// The reverb this describes.
    pub fn reverb(&self) -> crate::audio_fx::Reverb {
        crate::audio_fx::Reverb { decay: self.decay, size: 1.0, damp: 0.5, predelay: 0.01, mix: self.mix }
    }
}

/// `audio.duck`.
#[derive(Debug, Clone, PartialEq)]
pub struct DuckSpec {
    /// Rule events that duck the music.
    pub events: Vec<String>,
    /// How the music falls and recovers.
    pub duck: crate::mixer::Duck,
    /// Seconds it stays down.
    pub hold: f32,
}

/// One entry of `audio.layers`.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerSpec {
    /// The score file (resolved against the scene's folder).
    pub score: std::path::PathBuf,
    /// The rule variable that drives it.
    pub var: String,
    /// It plays while the variable is at least this.
    pub above: f64,
    /// Seconds to fade in or out.
    pub fade: f32,
    /// Its level, 0 to 1.
    pub volume: f32,
}

/// How loud a layer wants to be: 1 while its variable is at or above the threshold, else 0.
pub fn layer_target(value: f64, above: f64) -> f32 {
    if value >= above {
        1.0
    } else {
        0.0
    }
}

/// A score file a scene names must exist next to the scene and parse as a score: otherwise the game would silently play without that music. `base` is the scene's
/// folder (`None` for a scene parsed from text with no folder: nothing to check against). `what` is the path of the field, for the message.
fn check_score_file(errs: &mut Vec<String>, what: &str, named: &str, base: Option<&std::path::Path>) {
    let Some(base) = base else { return };
    let path = base.join(named);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => {
            errs.push(format!(
                "{what}: `{named}` not found (looked for {}): a score file lives next to the scene, and a game published without it plays no music",
                path.display()
            ));
            return;
        }
    };
    match serde_json::from_str::<serde_json::Value>(&text) {
        Err(e) => errs.push(format!("{what}: `{named}` is not valid JSON: {e}")),
        Ok(v) if v.get("tracks").is_none() => {
            errs.push(format!("{what}: `{named}` is not a score (it has no `tracks`; `red_engine2 audio export score.ambient_drift` is a template)"))
        }
        Ok(v) => {
            if let Err(e) = crate::score::parse_score(&v) {
                errs.push(format!("{what}: `{named}` is not a valid score: {}", e.join("; ")));
            }
        }
    }
}

/// Parses a scene's `audio` block (`base` is the folder the scene lives in, for the score paths).
pub fn parse_audio(root: &serde_json::Map<String, serde_json::Value>, base: Option<&std::path::Path>) -> Result<Option<AudioSpec>, Vec<String>> {
    use serde_json::Value;
    let Some(raw) = root.get("audio") else { return Ok(None) };
    let Some(o) = raw.as_object() else { return Err(vec!["audio: must be an object like {\"ambience\": \"nature\"}".to_string()]) };
    let mut errs = Vec::new();
    crate::strict::check_keys(&mut errs, "audio", o, AUDIO_KEYS);
    let nature = match o.get("ambience") {
        None | Some(Value::Bool(false)) => false,
        Some(Value::String(s)) if s == "nature" => true,
        Some(_) => {
            errs.push("audio.ambience: must be \"nature\" (the engine's countryside) or false".to_string());
            false
        }
    };
    let birds = match o.get("birds") {
        None => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => {
            errs.push("audio.birds: must be true or false".to_string());
            true
        }
    };
    let mut music = Vec::new();
    match o.get("music") {
        None => {}
        Some(Value::Object(m)) => {
            crate::strict::check_keys(&mut errs, "audio.music", m, &["dawn", "day", "dusk", "night"]);
            for mood in Mood::ALL {
                match m.get(mood.name()) {
                    None => {}
                    Some(Value::String(file)) => {
                        check_score_file(&mut errs, &format!("audio.music.{}", mood.name()), file, base);
                        music.push((mood, base.map_or_else(|| file.into(), |b| b.join(file))))
                    }
                    Some(_) => errs.push(format!("audio.music.{}: must be a score file name", mood.name())),
                }
            }
        }
        Some(_) => errs.push("audio.music: must be an object like {\"day\": \"audio/day.json\"}".to_string()),
    }
    let mut level = |key: &str, default: f32| match o.get(key) {
        None => default,
        Some(v) => match v.as_f64().filter(|x| (0.0..=1.0).contains(x)) {
            Some(x) => x as f32,
            None => {
                errs.push(format!("audio.{key}: must be a number from 0 to 1"));
                default
            }
        },
    };
    let (music_volume, ambience_volume) = (level("music_volume", 0.6), level("ambience_volume", 0.8));
    let num = |obj: &serde_json::Map<String, Value>, path: &str, key: &str, default: f64, lo: f64, hi: f64, errs: &mut Vec<String>| match obj.get(key) {
        None => default,
        Some(v) => match v.as_f64().filter(|x| (lo..=hi).contains(x)) {
            Some(x) => x,
            None => {
                errs.push(format!("{path}.{key}: must be a number from {lo} to {hi}"));
                default
            }
        },
    };
    let reverb = match o.get("reverb") {
        None => None,
        Some(Value::Object(r)) => {
            crate::strict::check_keys(&mut errs, "audio.reverb", r, &["decay", "mix"]);
            Some(RoomSpec {
                decay: num(r, "audio.reverb", "decay", 1.2, 0.1, 12.0, &mut errs) as f32,
                mix: num(r, "audio.reverb", "mix", 0.25, 0.0, 1.5, &mut errs) as f32,
            })
        }
        Some(_) => {
            errs.push("audio.reverb: must be an object like {\"decay\": 1.4, \"mix\": 0.25}".to_string());
            None
        }
    };
    let duck = match o.get("duck") {
        None => None,
        Some(Value::Object(d)) => {
            crate::strict::check_keys(&mut errs, "audio.duck", d, &["events", "depth", "attack", "hold", "release"]);
            let events: Vec<String> =
                d.get("events").and_then(Value::as_array).map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default();
            if events.is_empty() {
                errs.push("audio.duck.events: name the rule events that duck the music, like [\"hit\"]".to_string());
            }
            let duck = crate::mixer::Duck {
                depth: num(d, "audio.duck", "depth", 0.5, 0.0, 1.0, &mut errs) as f32,
                attack: num(d, "audio.duck", "attack", 0.08, 0.0, 5.0, &mut errs) as f32,
                release: num(d, "audio.duck", "release", 1.2, 0.0, 20.0, &mut errs) as f32,
            };
            Some(DuckSpec { events, duck, hold: num(d, "audio.duck", "hold", 0.6, 0.0, 30.0, &mut errs) as f32 })
        }
        Some(_) => {
            errs.push("audio.duck: must be an object like {\"events\": [\"hit\"], \"depth\": 0.5}".to_string());
            None
        }
    };
    let mut layers = Vec::new();
    match o.get("layers") {
        None => {}
        Some(Value::Array(list)) => {
            for (i, item) in list.iter().enumerate() {
                let path = format!("audio.layers[{i}]");
                let Some(l) = item.as_object() else {
                    errs.push(format!("{path}: must be an object like {{\"score\": \"chase.json\", \"var\": \"danger\", \"above\": 0.5}}"));
                    continue;
                };
                crate::strict::check_keys(&mut errs, &path, l, &["score", "var", "above", "fade", "volume"]);
                let (Some(score), Some(var)) = (l.get("score").and_then(Value::as_str), l.get("var").and_then(Value::as_str)) else {
                    errs.push(format!("{path}: needs a `score` file and the rule `var` that drives it"));
                    continue;
                };
                check_score_file(&mut errs, &format!("{path}.score"), score, base);
                layers.push(LayerSpec {
                    score: base.map_or_else(|| score.into(), |b| b.join(score)),
                    var: var.to_string(),
                    above: num(l, &path, "above", 0.5, -1e9, 1e9, &mut errs),
                    fade: num(l, &path, "fade", 3.0, 0.0, 60.0, &mut errs) as f32,
                    volume: num(l, &path, "volume", 0.6, 0.0, 1.0, &mut errs) as f32,
                });
            }
        }
        Some(_) => errs.push("audio.layers: must be a list".to_string()),
    }
    if errs.is_empty() {
        Ok(Some(AudioSpec { nature, birds, music, music_volume, ambience_volume, reverb, duck, layers }))
    } else {
        Err(errs)
    }
}

/// What kind of music the hour calls for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Mood {
    /// First light.
    Dawn,
    /// The bright day.
    Day,
    /// The golden hour and the dusk.
    Dusk,
    /// The night.
    Night,
}

impl Mood {
    /// Every mood, in the order of [`Frame::music`].
    pub const ALL: [Mood; 4] = [Mood::Dawn, Mood::Day, Mood::Dusk, Mood::Night];

    /// Short name (`dawn` ...): the key of a scene's `audio.music`.
    pub fn name(self) -> &'static str {
        match self {
            Mood::Dawn => "dawn",
            Mood::Day => "day",
            Mood::Dusk => "dusk",
            Mood::Night => "night",
        }
    }
}

/// The weight of each mood for a sun at `sun_elev_deg` that is rising (morning) or setting. The four always sum to 1 and change smoothly with the sun.
pub fn mood_weights(sun_elev_deg: f32, rising: bool) -> [f32; 4] {
    let night = smooth(-3.0, -9.0, sun_elev_deg);
    let low_sun = (1.0 - night) * (1.0 - smooth(14.0, 26.0, sun_elev_deg));
    let day = (1.0 - night) * smooth(14.0, 26.0, sun_elev_deg);
    let (dawn, dusk) = if rising { (low_sun, 0.0) } else { (0.0, low_sun) };
    [dawn, day, dusk, night]
}

/// What the listener is surrounded by and doing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Context {
    /// The sun's height above the horizon, degrees (negative at night).
    pub sun_elev_deg: f32,
    /// Whether it is morning (the sun is climbing) rather than evening.
    pub rising: bool,
    /// The country here.
    pub biome: Biome,
    /// How fast the listener moves, m/s.
    pub speed: f32,
}

/// A call to start now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Heard {
    /// Who.
    pub call: Call,
    /// Which phrase (the seed of [`Call::render`]).
    pub seed: u32,
    /// How loud (0..1).
    pub gain: f32,
    /// Left (-1) to right (1).
    pub pan: f32,
}

/// One step's answer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Frame {
    /// The gain of each bed, in [`Bed::ALL`] order.
    pub beds: [f32; 5],
    /// The weight of each mood, in [`Mood::ALL`] order (sum 1).
    pub music: [f32; 4],
    /// Calls that begin now.
    pub calls: Vec<Heard>,
}

/// The state of the soundscape.
pub struct Ambience {
    rng: Rng,
    beds: [f32; 5],
    music: [f32; 4],
    primed: bool,
    /// Seconds until each call may next sound (a countdown per call, redrawn when it fires).
    wait: [f32; 7],
    /// Calls started, ever, per call (for a script's assertions).
    pub totals: [u32; 7],
    /// Whether the songbirds are silenced (the owl is not counted as one).
    pub birds_off: bool,
}

/// How fast a bed's gain moves toward its target, per second.
const BED_SLEW: f32 = 0.18;
/// How fast the music crossfades, per second (a fade takes about eight seconds).
const MUSIC_SLEW: f32 = 0.13;

fn call_index(c: Call) -> usize {
    Call::ALL.iter().position(|x| *x == c).unwrap_or(0)
}

impl Ambience {
    /// A soundscape; the same seed gives the same sequence of calls.
    pub fn new(seed: u32) -> Ambience {
        let mut rng = Rng::at(seed, 5, 5);
        let mut wait = [0.0; 7];
        for w in &mut wait {
            *w = rng.range(0.0, 20.0);
        }
        Ambience { rng, beds: [0.0; 5], music: [0.0; 4], primed: false, wait, totals: [0; 7], birds_off: false }
    }

    /// The gain each bed should settle at in this context.
    pub fn bed_targets(c: &Context) -> [f32; 5] {
        let sun = c.sun_elev_deg;
        let day = smooth(-2.0, 12.0, sun);
        let night = smooth(4.0, -8.0, sun);
        let evening = if c.rising { 0.0 } else { smooth(14.0, -2.0, sun) * smooth(-14.0, -2.0, sun) };
        let (open, wooded, flowery) = match c.biome {
            Biome::Meadow | Biome::Wildflowers => (1.0, 0.0, if c.biome == Biome::Wildflowers { 1.0 } else { 0.45 }),
            Biome::Grove | Biome::Glade => (0.75, 0.55, 0.3),
            Biome::Forest | Biome::Pinewood => (0.35, 1.0, 0.1),
        };
        let wind = (0.3 + 0.12 * day) * (0.45 + 0.55 * open) * (1.0 + 0.2 * (c.speed / 6.0).min(1.0));
        let leaves = wooded * (0.35 + 0.4 * day) * (0.5 + 0.5 * open.max(0.4));
        let crickets = (night * 0.95 + evening * 0.6) * (1.0 - 0.45 * wooded);
        let night_air = night * (0.5 + 0.3 * open);
        let bees = smooth(14.0, 36.0, sun) * flowery * 0.9;
        [wind, leaves, crickets, night_air, bees]
    }

    /// How many of each call per second are heard in this context (all callers of a kind together).
    pub fn call_rates(c: &Context) -> [f32; 7] {
        let sun = c.sun_elev_deg;
        let wooded = match c.biome {
            Biome::Forest | Biome::Pinewood => 1.0,
            Biome::Grove | Biome::Glade => 0.7,
            Biome::Meadow | Biome::Wildflowers => 0.35,
        };
        // The dawn chorus: most intense from just before sunrise to a couple of hours after.
        let chorus = if c.rising { smooth(-12.0, -2.0, sun) * (1.0 - smooth(16.0, 38.0, sun)) } else { 0.0 };
        // Dusk song, gentler.
        let dusk = if c.rising { 0.0 } else { smooth(-6.0, 6.0, sun) * (1.0 - smooth(14.0, 24.0, sun)) * 0.6 };
        let day = smooth(8.0, 24.0, sun);
        let night = smooth(-4.0, -12.0, sun);
        let song = 0.35 * chorus + 0.12 * dusk + 0.05 * day;
        let hush = 1.0 - 0.3 * (c.speed / 6.0).clamp(0.0, 1.0);
        let at = |v: f32| (v * hush).max(0.0);
        let mut r = [0.0; 7];
        r[call_index(Call::Robin)] = at(song * (0.6 + 0.6 * wooded));
        r[call_index(Call::Blackbird)] = at(song * 0.7 * (0.5 + 0.7 * wooded));
        r[call_index(Call::Wren)] = at((0.5 * chorus + 0.06 * day) * 0.3 * wooded);
        r[call_index(Call::Chaffinch)] = at((0.55 * chorus + 0.1 * day) * 0.25 * (0.4 + wooded));
        r[call_index(Call::Cuckoo)] = at(0.04 * day * (1.0 - chorus * 0.5) * wooded);
        r[call_index(Call::Pigeon)] = at((0.05 * day + 0.04 * dusk) * (0.3 + wooded));
        r[call_index(Call::Owl)] = at(0.028 * night * (0.5 + 0.5 * wooded));
        r
    }

    /// Advances by `dt` seconds in a context and returns the gains, the music and the calls that start.
    pub fn step(&mut self, dt: f32, c: &Context) -> Frame {
        let beds = Self::bed_targets(c);
        let music = mood_weights(c.sun_elev_deg, c.rising);
        if !self.primed {
            // The first step starts where the hour already is, not from silence.
            self.beds = beds;
            self.music = music;
            self.primed = true;
        }
        for (g, t) in self.beds.iter_mut().zip(beds) {
            *g += (t - *g).clamp(-BED_SLEW * dt, BED_SLEW * dt);
        }
        for (g, t) in self.music.iter_mut().zip(music) {
            *g += (t - *g).clamp(-MUSIC_SLEW * dt, MUSIC_SLEW * dt);
        }
        let sum: f32 = self.music.iter().sum::<f32>().max(1e-4);
        let weights = self.music.map(|w| w / sum);
        // Calls: each kind counts down; when it reaches zero it sounds and waits an exponentially distributed time for its current rate.
        let mut rates = Self::call_rates(c);
        if self.birds_off {
            for (i, call) in Call::ALL.into_iter().enumerate() {
                if call != Call::Owl {
                    rates[i] = 0.0;
                }
            }
        }
        let mut calls = Vec::new();
        for (i, call) in Call::ALL.into_iter().enumerate() {
            if rates[i] <= 1e-5 {
                // Not now; keep the countdown from running out into a burst when the hour comes.
                self.wait[i] = self.wait[i].max(2.0);
                continue;
            }
            self.wait[i] -= dt;
            if self.wait[i] <= 0.0 {
                let gap = -(1.0 - self.rng.white()).max(1e-4).ln() / rates[i];
                self.wait[i] = gap.clamp(0.6, 400.0);
                let near = self.rng.range(0.25, 1.0);
                calls.push(Heard { call, seed: self.rng.bits() % 4096, gain: 0.14 + 0.5 * near * near, pan: self.rng.range(-0.85, 0.85) });
                self.totals[i] += 1;
            }
        }
        Frame { beds: self.beds, music: weights, calls }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nature::Bed;

    fn ctx(sun: f32, rising: bool, biome: Biome) -> Context {
        Context { sun_elev_deg: sun, rising, biome, speed: 0.0 }
    }

    /// Runs the soundscape for `secs` in one fixed context and counts the calls.
    fn count(c: Context, secs: f32, seed: u32) -> [u32; 7] {
        let mut a = Ambience::new(seed);
        let mut t = 0.0;
        while t < secs {
            a.step(0.1, &c);
            t += 0.1;
        }
        a.totals
    }

    fn who(c: Call) -> usize {
        call_index(c)
    }

    #[test]
    fn music_weights_always_sum_to_one_and_follow_the_sun() {
        for rising in [true, false] {
            for k in -400..=900 {
                let w = mood_weights(k as f32 / 10.0, rising);
                assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-4 && w.iter().all(|x| (0.0..=1.0001).contains(x)), "{w:?}");
            }
        }
        let top = |sun, rising| {
            let w = mood_weights(sun, rising);
            Mood::ALL[w.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0]
        };
        assert_eq!(top(6.0, true), Mood::Dawn);
        assert_eq!(top(55.0, true), Mood::Day);
        assert_eq!(top(55.0, false), Mood::Day);
        assert_eq!(top(5.0, false), Mood::Dusk);
        assert_eq!(top(-30.0, false), Mood::Night);
        assert_eq!(top(-30.0, true), Mood::Night);
    }

    #[test]
    fn the_mood_never_jumps_as_the_sun_goes_round() {
        let mut prev = mood_weights(-60.0, false);
        // Evening to night to morning to day to evening: step the sun through a day.
        let day: Vec<(f32, bool)> = (0..=2400).map(|k| (k as f32 / 20.0 - 60.0, false)).chain((0..=2400).map(|k| (60.0 - k as f32 / 20.0, true))).collect();
        let order: Vec<(f32, bool)> = day.into_iter().rev().collect();
        for (sun, rising) in order {
            let w = mood_weights(sun, rising);
            let jump: f32 = w.iter().zip(prev).map(|(a, b)| (a - b).abs()).sum();
            assert!(jump < 0.08, "the music jumps by {jump} at {sun} degrees (rising {rising})");
            prev = w;
        }
    }

    #[test]
    fn the_dawn_chorus_is_far_louder_than_noon_and_noon_louder_than_night() {
        let meadow = Biome::Meadow;
        let birds = |t: [u32; 7]| t[who(Call::Robin)] + t[who(Call::Blackbird)] + t[who(Call::Wren)] + t[who(Call::Chaffinch)];
        let dawn = birds(count(ctx(4.0, true, meadow), 300.0, 1));
        let noon = birds(count(ctx(60.0, true, meadow), 300.0, 1));
        let night = birds(count(ctx(-40.0, false, meadow), 300.0, 1));
        assert!(dawn >= 40, "{dawn} calls in five minutes of dawn");
        assert!(noon * 4 <= dawn, "noon {noon} vs dawn {dawn}");
        assert!(noon >= 3, "some song at noon: {noon}");
        assert_eq!(night, 0, "the birds are silent at night");
    }

    #[test]
    fn owls_and_crickets_belong_to_the_night() {
        let night = count(ctx(-40.0, false, Biome::Forest), 900.0, 2);
        assert!(night[who(Call::Owl)] >= 6, "owl calls in a quarter of an hour of night: {}", night[who(Call::Owl)]);
        assert_eq!(count(ctx(60.0, true, Biome::Forest), 900.0, 2)[who(Call::Owl)], 0);
        let bed = |c: Context, b: Bed| Ambience::bed_targets(&c)[Bed::ALL.iter().position(|x| *x == b).unwrap()];
        assert!(bed(ctx(-40.0, false, Biome::Meadow), Bed::Crickets) > 0.8);
        assert!(bed(ctx(60.0, true, Biome::Meadow), Bed::Crickets) < 0.01);
        assert!(bed(ctx(60.0, true, Biome::Meadow), Bed::NightAir) < 0.01);
        assert!(bed(ctx(-40.0, false, Biome::Meadow), Bed::NightAir) > 0.4);
    }

    #[test]
    fn places_sound_different() {
        let bed = |c: Context, b: Bed| Ambience::bed_targets(&c)[Bed::ALL.iter().position(|x| *x == b).unwrap()];
        let noon = |biome| ctx(55.0, true, biome);
        assert!(bed(noon(Biome::Forest), Bed::Leaves) > 3.0 * bed(noon(Biome::Meadow), Bed::Leaves).max(0.01), "forests rustle");
        assert!(bed(noon(Biome::Meadow), Bed::Wind) > bed(noon(Biome::Forest), Bed::Wind), "fields are windier");
        assert!(bed(noon(Biome::Wildflowers), Bed::Bees) > 2.0 * bed(noon(Biome::Forest), Bed::Bees), "bees work the flowers");
        assert!(bed(ctx(-40.0, false, Biome::Meadow), Bed::Crickets) > bed(ctx(-40.0, false, Biome::Forest), Bed::Crickets));
        let wooded = count(ctx(4.0, true, Biome::Forest), 300.0, 4).iter().sum::<u32>();
        let open = count(ctx(4.0, true, Biome::Meadow), 300.0, 4).iter().sum::<u32>();
        assert!(wooded > open, "the wood is noisier at dawn: {wooded} vs {open}");
    }

    #[test]
    fn gains_move_gradually_and_the_first_step_starts_in_place() {
        let mut a = Ambience::new(1);
        let noon = ctx(60.0, true, Biome::Meadow);
        let first = a.step(0.016, &noon);
        assert!(first.beds[Bed::ALL.iter().position(|b| *b == Bed::Wind).unwrap()] > 0.3, "no fade-in from silence at the start");
        assert!((first.music.iter().sum::<f32>() - 1.0).abs() < 1e-3);
        // Night falls: the crickets rise slowly (about five seconds from nothing to everything cannot happen).
        let night = ctx(-40.0, false, Biome::Meadow);
        let c = Bed::ALL.iter().position(|b| *b == Bed::Crickets).unwrap();
        let after_one_second = (0..60).map(|_| a.step(1.0 / 60.0, &night)).last().unwrap();
        assert!(after_one_second.beds[c] < 0.25, "crickets jumped to {}", after_one_second.beds[c]);
        let after_a_minute = (0..3600).map(|_| a.step(1.0 / 60.0, &night)).last().unwrap();
        assert!(after_a_minute.beds[c] > 0.8);
        assert!(after_a_minute.music[3] > 0.99, "and the music has become night's");
    }

    #[test]
    fn the_same_seed_sings_the_same_song() {
        let c = ctx(3.0, true, Biome::Grove);
        let run = |seed| {
            let mut a = Ambience::new(seed);
            (0..3000).flat_map(|_| a.step(0.1, &c).calls).collect::<Vec<_>>()
        };
        assert_eq!(run(7), run(7));
        assert_ne!(run(7), run(8));
        assert!(run(7).iter().all(|h| (0.0..=1.0).contains(&h.gain) && (-1.0..=1.0).contains(&h.pan)));
    }

    /// A folder holding a real (minimal) score under each of `names`: a scene may only name score files that exist and parse.
    fn scores_in_a_folder(tag: &str, names: &[&str]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("re2_ambience_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for name in names {
            let file = dir.join(name);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(
                file,
                r#"{"bpm": 120, "beats": 4, "bars": 2, "key": "D", "scale": "major", "seed": 3, "lufs": -27,
                    "instruments": {"pad": {"seconds": 4, "level": 0.5, "layers": [{"sine": 1, "attack": 0.5, "release": 1}]}},
                    "tracks": [{"inst": "pad", "play": "chords", "chords": "I V", "every": "1 bar", "octave": 3}]}"#,
            )
            .unwrap();
        }
        dir
    }

    #[test]
    fn an_audio_block_is_checked() {
        let base = scores_in_a_folder("block", &["a/d.json", "n.json", "x.json"]);
        let parse = |v: serde_json::Value| parse_audio(v.as_object().unwrap(), Some(base.as_path()));
        let ok = parse(serde_json::json!({"audio": {"ambience": "nature", "music": {"dawn": "a/d.json", "night": "n.json"}, "music_volume": 0.4}}))
            .unwrap()
            .unwrap();
        assert!(ok.nature && ok.music.len() == 2 && ok.music[0] == (Mood::Dawn, base.join("a/d.json")) && ok.music_volume == 0.4 && ok.ambience_volume == 0.8);
        assert_eq!(parse(serde_json::json!({})).unwrap(), None);
        for (bad, needle) in [
            (serde_json::json!({"audio": {"ambience": "jungle"}}), "audio.ambience"),
            (serde_json::json!({"audio": {"music": {"noon": "x.json"}}}), "noon"),
            (serde_json::json!({"audio": {"music": {"day": 3}}}), "audio.music.day"),
            (serde_json::json!({"audio": {"music_volume": 2}}), "music_volume"),
            (serde_json::json!({"audio": {"sound": 1}}), "sound"),
        ] {
            let e = parse(bad).unwrap_err().join(" ");
            assert!(e.contains(needle), "{e}");
        }
    }

    #[test]
    fn reverb_duck_and_layers_are_checked() {
        let base = scores_in_a_folder("layers", &["c.json", "x.json"]);
        let parse = |v: serde_json::Value| parse_audio(v.as_object().unwrap(), Some(base.as_path()));
        let ok = parse(serde_json::json!({"audio": {"reverb": {"decay": 2, "mix": 0.3}, "duck": {"events": ["hit"], "depth": 0.6, "hold": 1}, "layers": [{"score": "c.json", "var": "danger", "above": 2}]}}))
            .unwrap()
            .unwrap();
        assert_eq!(ok.reverb, Some(RoomSpec { decay: 2.0, mix: 0.3 }));
        assert_eq!(ok.duck.as_ref().map(|d| (d.events.clone(), d.duck.depth, d.hold)), Some((vec!["hit".to_string()], 0.6, 1.0)));
        assert_eq!((ok.layers[0].score.clone(), ok.layers[0].var.as_str(), ok.layers[0].above, ok.layers[0].fade), (base.join("c.json"), "danger", 2.0, 3.0));
        for (bad, needle) in [
            (serde_json::json!({"audio": {"reverb": {"decay": 99}}}), "audio.reverb.decay"),
            (serde_json::json!({"audio": {"reverb": {"wet": 1}}}), "wet"),
            (serde_json::json!({"audio": {"duck": {"depth": 0.5}}}), "audio.duck.events"),
            (serde_json::json!({"audio": {"layers": [{"score": "x.json"}]}}), "audio.layers[0]"),
            (serde_json::json!({"audio": {"layers": [{"score": "x.json", "var": "v", "fade": -1}]}}), "fade"),
        ] {
            let e = parse(bad).unwrap_err().join(" ");
            assert!(e.contains(needle), "{e}");
        }
        assert_eq!((layer_target(2.0, 2.0), layer_target(1.9, 2.0)), (1.0, 0.0));
    }

    #[test]
    fn running_hushes_the_birds_a_little() {
        let mut c = ctx(4.0, true, Biome::Meadow);
        let still: f32 = Ambience::call_rates(&c).iter().sum();
        c.speed = 6.0;
        let running: f32 = Ambience::call_rates(&c).iter().sum();
        assert!(running < still && running > 0.6 * still);
    }
}
