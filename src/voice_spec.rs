//! A sound as data: the JSON form of a [`Voice`] (`dsp.rs`), small enough to write by hand and to read back at a glance.
//!
//! ```json
//! { "seconds": 0.45, "level": 0.62, "seed": 7, "layers": [
//!   { "noise": "hp 0.5",   "decay": 420, "gain": 0.9 },
//!   { "glide": [190, 120, 7], "decay": 22, "gain": 0.55 },
//!   { "noise": "lp 0.22",  "decay": 45,  "gain": 1.1 },
//!   { "noise": "lp 0.045", "decay": 9, "fade_in": 70, "gain": 0.6 },
//!   { "sine": 523.25, "harmonics": [[2, 0.3]], "decay": 3, "delay": 0.1, "attack": 0.01, "release": 0.2 } ] }
//! ```
//!
//! A layer has exactly one source (`sine` with optional `harmonics`, `glide`, `sweep` or `noise`), and any of `decay`, `fade_in`, `delay`, `attack`,
//! `release` and `gain`. `noise` is `"white"`, `"lp K"`, `"hp K"` or `"lp A..B"` (a low-pass that opens from A to B with the layer's swell). Every mistake
//! is reported with its path (`layers[2].decay`) and the engine's usual did-you-mean; [`voice_to_json`] is the exact inverse, so any voice built in code
//! (a gun, a cue) can be exported, edited and played back.

use crate::dsp::{Env, Filter, Layer, Src, Voice};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const MAX_LAYERS: usize = 64;
const MAX_SECONDS: f32 = 30.0;
const LAYER_KEYS: &[&str] = &["sine", "harmonics", "glide", "sweep", "noise", "decay", "fade_in", "delay", "attack", "release", "gain"];
const VOICE_KEYS: &[&str] = &["seconds", "level", "seed", "attack", "pitched", "layers"];

#[derive(Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct LayerSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    sine: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    harmonics: Option<Vec<[f32; 2]>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    glide: Option<[f32; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sweep: Option<[f32; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    noise: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    decay: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fade_in: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    delay: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attack: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    release: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gain: Option<f32>,
}

fn near(name: &str, options: &[&str]) -> String {
    match crate::prefabs::suggest(name, options.iter().copied()).first() {
        Some(h) => format!(" — did you mean `{h}`?"),
        None => String::new(),
    }
}

fn noise_text(f: Filter) -> String {
    match f {
        Filter::None => "white".to_string(),
        Filter::Low(k) => format!("lp {k}"),
        Filter::High(k) => format!("hp {k}"),
        Filter::LowOpen { closed, open } => format!("lp {closed}..{open}"),
    }
}

fn parse_noise(text: &str, path: &str, errs: &mut Vec<String>) -> Filter {
    let coeff = |s: &str, errs: &mut Vec<String>| match s.parse::<f32>() {
        Ok(k) if k > 0.0 && k <= 1.0 => Some(k),
        _ => {
            errs.push(format!("{path}: `{s}` is not a filter coefficient (a number above 0 and up to 1; smaller is duller)"));
            None
        }
    };
    let parts: Vec<&str> = text.split_whitespace().collect();
    match parts.as_slice() {
        ["white"] => Filter::None,
        ["lp", k] if k.contains("..") => {
            let (a, b) = k.split_once("..").unwrap_or(("", ""));
            match (coeff(a, errs), coeff(b, errs)) {
                (Some(closed), Some(open)) => Filter::LowOpen { closed, open },
                _ => Filter::None,
            }
        }
        ["lp", k] => coeff(k, errs).map_or(Filter::None, Filter::Low),
        ["hp", k] => coeff(k, errs).map_or(Filter::None, Filter::High),
        _ => {
            errs.push(format!("{path}: `{text}` is not a noise filter (white, lp K, hp K, or lp A..B)"));
            Filter::None
        }
    }
}

fn layer_from(v: &Value, path: &str, pitched: bool, errs: &mut Vec<String>) -> Option<Layer> {
    let Some(obj) = v.as_object() else {
        errs.push(format!("{path}: must be an object like {{\"sine\": 440, \"decay\": 3}}"));
        return None;
    };
    for k in obj.keys().filter(|k| !LAYER_KEYS.contains(&k.as_str())) {
        errs.push(format!("{path}.{k}: unknown field{} (fields: {})", near(k, LAYER_KEYS), LAYER_KEYS.join(", ")));
    }
    let spec: LayerSpec = match serde_json::from_value(Value::Object(
        obj.iter().filter(|(k, _)| LAYER_KEYS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect(),
    )) {
        Ok(s) => s,
        Err(e) => {
            errs.push(format!("{path}: {e}"));
            return None;
        }
    };
    let sources = [spec.sine.is_some(), spec.glide.is_some(), spec.sweep.is_some(), spec.noise.is_some()].iter().filter(|b| **b).count();
    if sources != 1 {
        errs.push(format!("{path}: give exactly one source: sine, glide, sweep or noise (got {sources})"));
        return None;
    }
    if spec.harmonics.is_some() && spec.sine.is_none() {
        errs.push(format!("{path}.harmonics: only a `sine` layer has harmonics"));
    }
    let range = |name: &str, x: Option<f32>, lo: f32, hi: f32, errs: &mut Vec<String>| {
        if let Some(x) = x.filter(|x| !(lo..=hi).contains(x)) {
            errs.push(format!("{path}.{name}: {x} is outside {lo} to {hi}"));
        }
    };
    let hz = |name: &str, x: f32, errs: &mut Vec<String>| range(name, Some(x), 1.0, 20000.0, errs);
    let src = if let Some(f) = spec.sine {
        if pitched {
            // In an instrument the number is a multiple of the note, not a frequency.
            range("sine", Some(f), 0.1, 64.0, errs);
        } else {
            hz("sine", f, errs);
        }
        let partials: Vec<(f32, f32)> = spec.harmonics.clone().unwrap_or_default().into_iter().map(|[m, a]| (m, a)).collect();
        if partials.iter().any(|(m, _)| !(0.1..=64.0).contains(m)) {
            errs.push(format!("{path}.harmonics: a multiple must be 0.1 to 64"));
        }
        Src::Tone { hz: f, partials }
    } else if let Some([from, to, rate]) = spec.glide {
        hz("glide", from, errs);
        hz("glide", to, errs);
        range("glide", Some(rate), 0.0, 10000.0, errs);
        Src::Glide { from, to, rate }
    } else if let Some([from, to, rate]) = spec.sweep {
        hz("sweep", from, errs);
        hz("sweep", to, errs);
        range("sweep", Some(rate), 0.0, 10000.0, errs);
        Src::Sweep { from, to, rate }
    } else {
        Src::Noise(parse_noise(spec.noise.as_deref().unwrap_or(""), &format!("{path}.noise"), errs))
    };
    range("decay", spec.decay, 0.0, 10000.0, errs);
    range("fade_in", spec.fade_in, 0.0, 100000.0, errs);
    range("delay", spec.delay, 0.0, MAX_SECONDS, errs);
    range("attack", spec.attack, 0.0, MAX_SECONDS, errs);
    range("release", spec.release, 0.0, MAX_SECONDS, errs);
    range("gain", spec.gain, -50.0, 50.0, errs);
    let env = Env {
        decay: spec.decay.unwrap_or(0.0),
        fade_in: spec.fade_in.unwrap_or(0.0),
        delay: spec.delay.unwrap_or(0.0),
        attack: spec.attack.unwrap_or(0.0),
        release: spec.release.unwrap_or(0.0),
    };
    Some(Layer::new(src, env, spec.gain.unwrap_or(1.0)))
}

/// Parses the JSON form of a voice; every problem is reported with its path.
pub fn parse_voice(v: &Value) -> Result<Voice, Vec<String>> {
    parse_voice_up_to(v, MAX_SECONDS)
}

/// [`parse_voice`] allowing a length up to `max_seconds` (an instrument in a score may ring for a whole loop).
pub fn parse_voice_up_to(v: &Value, max_seconds: f32) -> Result<Voice, Vec<String>> {
    let Some(obj) = v.as_object() else {
        return Err(vec!["a sound is an object like {\"seconds\": 0.3, \"layers\": [{\"sine\": 440, \"decay\": 8}]}".to_string()]);
    };
    let mut errs = Vec::new();
    for k in obj.keys().filter(|k| !VOICE_KEYS.contains(&k.as_str())) {
        errs.push(format!("{k}: unknown field{} (fields: {})", near(k, VOICE_KEYS), VOICE_KEYS.join(", ")));
    }
    let num = |k: &str, default: Option<f32>, errs: &mut Vec<String>| -> f32 {
        match obj.get(k) {
            None => default.unwrap_or_else(|| {
                errs.push(format!("{k}: missing"));
                0.0
            }),
            Some(x) => x.as_f64().map(|f| f as f32).unwrap_or_else(|| {
                errs.push(format!("{k}: must be a number"));
                0.0
            }),
        }
    };
    let seconds = num("seconds", None, &mut errs);
    let level = num("level", Some(0.5), &mut errs);
    if !(0.01..=max_seconds).contains(&seconds) {
        errs.push(format!("seconds: {seconds} is outside 0.01 to {max_seconds}"));
    }
    if !(0.0..=0.98).contains(&level) || level == 0.0 {
        errs.push(format!("level: {level} is outside 0 (exclusive) to 0.98 (the peak the clip is normalised to)"));
    }
    let seed = match obj.get("seed") {
        None => 0,
        Some(x) => x.as_u64().filter(|s| *s <= u32::MAX as u64).unwrap_or_else(|| {
            errs.push("seed: must be a whole number up to 4294967295".to_string());
            0
        }) as u32,
    };
    let attack = match obj.get("attack") {
        None => true,
        Some(x) => x.as_bool().unwrap_or_else(|| {
            errs.push("attack: must be true or false (the 0.3 ms click-free ramp at the start)".to_string());
            true
        }),
    };
    let pitched = match obj.get("pitched") {
        None => false,
        Some(x) => x.as_bool().unwrap_or_else(|| {
            errs.push("pitched: must be true or false (an instrument: sine frequencies are multiples of the note)".to_string());
            false
        }),
    };
    let mut voice = Voice::new(seconds, level, seed);
    voice.attack = attack;
    voice.pitched = pitched;
    match obj.get("layers").and_then(Value::as_array) {
        Some(list) if !list.is_empty() && list.len() <= MAX_LAYERS => {
            for (i, l) in list.iter().enumerate() {
                if let Some(layer) = layer_from(l, &format!("layers[{i}]"), pitched, &mut errs) {
                    voice.layers.push(layer);
                }
            }
        }
        _ => errs.push(format!("layers: a list of 1 to {MAX_LAYERS} layers")),
    }
    if errs.is_empty() {
        Ok(voice)
    } else {
        Err(errs)
    }
}

/// Rewrites every number as the shortest decimal that is the same `f32` (0.45, not 0.44999998807907104) and whole numbers as integers (420, not 420.0).
fn tidy(v: &mut Value) {
    match v {
        Value::Number(n) if n.is_f64() => {
            if let Some(f) = n.as_f64() {
                let short: f64 = (f as f32).to_string().parse().unwrap_or(f);
                *v = if short.fract() == 0.0 && short.abs() < 1e12 {
                    Value::from(short as i64)
                } else {
                    serde_json::Number::from_f64(short).map_or(Value::Null, Value::Number)
                };
            }
        }
        Value::Number(_) => {}
        Value::Array(a) => a.iter_mut().for_each(tidy),
        Value::Object(o) => o.values_mut().for_each(tidy),
        _ => {}
    }
}

/// The JSON form of a voice: the exact inverse of [`parse_voice`] (defaults are left out).
pub fn voice_to_json(v: &Voice) -> Value {
    let mut out = voice_json(v);
    tidy(&mut out);
    out
}

fn voice_json(v: &Voice) -> Value {
    let layers: Vec<Value> = v
        .layers
        .iter()
        .map(|l| {
            let mut spec = LayerSpec::default();
            match &l.src {
                Src::Tone { hz, partials } => {
                    spec.sine = Some(*hz);
                    if !partials.is_empty() {
                        spec.harmonics = Some(partials.iter().map(|(m, a)| [*m, *a]).collect());
                    }
                }
                Src::Glide { from, to, rate } => spec.glide = Some([*from, *to, *rate]),
                Src::Sweep { from, to, rate } => spec.sweep = Some([*from, *to, *rate]),
                Src::Noise(f) => spec.noise = Some(noise_text(*f)),
            }
            let nonzero = |x: f32| (x != 0.0).then_some(x);
            spec.decay = nonzero(l.env.decay);
            spec.fade_in = nonzero(l.env.fade_in);
            spec.delay = nonzero(l.env.delay);
            spec.attack = nonzero(l.env.attack);
            spec.release = nonzero(l.env.release);
            spec.gain = (l.gain != 1.0).then_some(l.gain);
            serde_json::to_value(spec).unwrap_or(Value::Null)
        })
        .collect();
    let mut out = json!({ "seconds": v.seconds, "level": v.level });
    if v.seed != 0 {
        out["seed"] = json!(v.seed);
    }
    if !v.attack {
        out["attack"] = json!(false);
    }
    if v.pitched {
        out["pitched"] = json!(true);
    }
    out["layers"] = Value::Array(layers);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(v: Value) -> Result<Voice, Vec<String>> {
        parse_voice(&v)
    }

    #[test]
    fn the_documented_example_parses_and_round_trips_to_the_same_sound() {
        let v = json!({ "seconds": 0.45, "level": 0.62, "seed": 7, "layers": [
            { "noise": "hp 0.5", "decay": 420, "gain": 0.9 },
            { "glide": [190, 120, 7], "decay": 22, "gain": 0.55 },
            { "noise": "lp 0.22", "decay": 45, "gain": 1.1 },
            { "noise": "lp 0.045", "decay": 9, "fade_in": 70, "gain": 0.6 },
            { "sine": 523.25, "harmonics": [[2, 0.3]], "decay": 3, "delay": 0.1, "attack": 0.01, "release": 0.2 },
            { "sweep": [365, 45, 2.4], "decay": 4.5 }, { "noise": "lp 0.05..0.55", "attack": 0.1, "release": 0.1 } ] });
        let voice = parse(v.clone()).unwrap();
        let back = voice_to_json(&voice);
        assert_eq!(back, v, "defaults are left out and everything else survives");
        assert_eq!(parse(back).unwrap().render(), voice.render(), "same JSON, same samples");
    }

    #[test]
    fn every_voice_built_in_code_survives_the_trip_through_json() {
        for (name, voice) in crate::sfx::voice_catalog() {
            let json = voice_to_json(&voice);
            let again = parse_voice(&json).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            let (a, b) = (voice.render(), again.render());
            assert_eq!(a.len(), b.len(), "{name}");
            assert!(a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-4), "{name}: the exported JSON must sound the same");
        }
    }

    #[test]
    fn every_mistake_is_named_with_its_path_and_a_suggestion() {
        let e = parse(json!({ "seconds": 0.3, "layers": [
            { "sine": 440, "decy": 3 }, { "sine": 440, "noise": "white" }, { "decay": 3 }, { "noise": "lp 1.5" }, { "noise": "buzz" },
            { "sine": 99999 }, { "noise": "white", "harmonics": [[2, 1]] }, { "sine": 440, "gain": 500, "delay": -1 }, 7 ], "levl": 0.5 }))
        .unwrap_err()
        .join("\n");
        for want in [
            "levl: unknown field — did you mean `level`?",
            "layers[0].decy: unknown field — did you mean `decay`?",
            "layers[1]: give exactly one source",
            "layers[2]: give exactly one source",
            "layers[3].noise: `1.5` is not a filter coefficient",
            "layers[4].noise: `buzz` is not a noise filter",
            "layers[5].sine: 99999 is outside 1 to 20000",
            "layers[6].harmonics: only a `sine` layer has harmonics",
            "layers[7].gain: 500 is outside -50 to 50",
            "layers[7].delay: -1 is outside 0 to 30",
            "layers[8]: must be an object",
        ] {
            assert!(e.contains(want), "missing `{want}` in:\n{e}");
        }
        let top = parse(json!({ "seconds": 99, "level": 2, "layers": [] })).unwrap_err().join("\n");
        assert!(top.contains("seconds: 99 is outside") && top.contains("level: 2 is outside") && top.contains("layers: a list of 1 to 64"), "{top}");
        assert!(parse(json!([1])).unwrap_err()[0].contains("a sound is an object"));
        assert!(parse(json!({ "layers": [{ "sine": 1 }] })).unwrap_err().join(" ").contains("seconds: missing"));
    }

    #[test]
    fn a_hand_written_sound_renders_and_measures_as_described() {
        let voice = parse(json!({ "seconds": 0.6, "level": 0.5, "layers": [{ "sine": 440, "decay": 4 }] })).unwrap();
        let clip = voice.render();
        let r = crate::audio_analysis::analyze(crate::audio_analysis::Clip { samples: &clip, channels: 1, rate: crate::synth::SAMPLE_RATE });
        assert!(crate::audio_analysis::note_name(r.dominant_hz).starts_with("A4"), "{}", crate::audio_analysis::note_name(r.dominant_hz));
        assert!((r.peak_dbfs + 6.0).abs() < 0.3, "{}", r.peak_dbfs);
    }
}
