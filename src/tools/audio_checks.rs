//! `checks.audio`: a scene holds its own sound to a standard, run by `verify` with everything else (no sound card, no GPU).
//!
//! ```json
//! "checks": { "audio": {
//!   "scores": { "lufs": [-34, -22], "peak_max": -3 },
//!   "sounds": [ { "name": "robin is quiet", "sound": "bird.robin", "lufs": [-30, -18] } ],
//!   "soundscape": [
//!     { "name": "dawn chorus", "sun": 4, "rising": true, "biome": "meadow", "secs": 300, "calls": { "any": [30, 400], "owl": [0, 0] } },
//!     { "name": "night",       "sun": -40, "biome": "forest", "secs": 600, "calls": { "owl": [3, 100], "robin": [0, 0] }, "beds": { "crickets": [0.5, 1] } } ] } }
//! ```
//! * `scores` holds every score the scene's `audio` block names to the loop standard (no seam, no clipping, audible) and to a loudness window (LUFS, default
//!   -34 to -20) and a peak ceiling (dBFS, default -3).
//! * `sounds` does the same for any built-in sound or sound file, with `lufs`, `peak_max`, `seam_max` and `max_secs` as the author wants them.
//! * `soundscape` runs the ambience state machine for `secs` of game time in a given place and hour (`sun` degrees above the horizon, `rising`, `biome`,
//!   `speed`) and counts the calls heard (by name, or `any`) and reads the bed gains the place settles at, each against `[min, max]`: "owls call at night and never at
//!   noon" is a check, not a hope.

use crate::ambience::{Ambience, Context};
use crate::audio_analysis::{problems, Kind};
use crate::nature::{Bed, Call};
use crate::procgen::Biome;
use crate::strict::{check_fields, check_keys, describe_value, opt, req, Field, Ty};
use serde_json::{Map, Value};
use std::path::Path;

/// Keys of a `checks.audio` block.
pub const AUDIO_CHECK_KEYS: &[&str] = &["scores", "sounds", "soundscape"];

/// `checks.audio.scores`: a loudness window and a peak ceiling for every score the scene names.
const SCORES_FIELDS: &[Field] = &[opt("lufs", Ty::Range, "LUFS, default [-34, -20]"), opt("peak_max", Ty::Num(Some((-120.0, 0.0))), "dBFS, default -3")];

/// `checks.audio.sounds[i]`.
const SOUND_FIELDS: &[Field] = &[
    opt("name", Ty::Str, "what the row is called"),
    req("sound", Ty::Str, "a built-in like bird.robin, or a .json/.wav file next to the scene"),
    opt("lufs", Ty::Range, "LUFS"),
    opt("peak_max", Ty::Num(Some((-120.0, 0.0))), "dBFS, default -1"),
    opt("seam_max", Ty::Num(Some((0.0, 1000.0))), "loops only: the end-to-start jump as a multiple of the usual step"),
    opt("max_secs", Ty::Num(Some((0.0, 3600.0))), "seconds"),
];

/// `checks.audio.soundscape[i]`.
const SOUNDSCAPE_FIELDS: &[Field] = &[
    opt("name", Ty::Str, "what the row is called"),
    req("sun", Ty::Num(Some((-90.0, 90.0))), "the sun's height in degrees, negative at night"),
    opt("rising", Ty::Bool, "default true"),
    opt("biome", Ty::Custom(valid_biome), "meadow, wildflowers, grove, forest, pinewood or glade; default meadow"),
    opt("secs", Ty::Num(Some((1.0, 7200.0))), "game seconds to run, default 300"),
    opt("seed", Ty::Count, "default 1"),
    opt("birds", Ty::Bool, "default: the scene's `audio.birds`, so the check hears what the player hears"),
    opt("speed", Ty::Num(Some((0.0, 20.0))), "how fast the listener moves, m/s, default 0"),
    opt("calls", Ty::Custom(valid_calls), "{ \"robin\": [min, max], \"any\": [min, max] }: calls heard in `secs`"),
    opt("beds", Ty::Custom(valid_beds), "{ \"crickets\": [min, max] }: bed gains the place settles at, 0 to 1"),
];

fn valid_biome(v: &Value) -> Result<(), String> {
    let names = "meadow, wildflowers, grove, forest, pinewood, glade";
    match v.as_str() {
        Some(b) if biome_of(b).is_some() => Ok(()),
        Some(b) => {
            let near = crate::prefabs::suggest(b, names.split(", "));
            Err(format!("`{b}` is not a biome ({names}){}", if near.is_empty() { String::new() } else { format!(" — did you mean `{}`?", near[0]) }))
        }
        None => Err(format!("expected a biome name ({names}), got {}", describe_value(v))),
    }
}

/// `{ name: [min, max] }` where every name is one of `names` and every range is two numbers with min <= max.
fn name_ranges(v: &Value, what: &str, names: &[&str], unit_max: Option<f64>) -> Result<(), String> {
    let Some(map) = v.as_object() else { return Err(format!("expected an object like {{\"{}\": [min, max]}}, got {}", names[0], describe_value(v))) };
    if map.is_empty() {
        return Err(format!("is empty: name at least one {what} with a [min, max] range"));
    }
    let mut bad = Vec::new();
    for (k, range) in map {
        if !names.contains(&k.as_str()) {
            let near = crate::prefabs::suggest(k, names.iter().copied());
            bad.push(format!(
                "`{k}` is not a {what}{} (valid: {})",
                near.first().map_or(String::new(), |n| format!(" — did you mean `{n}`?")),
                names.join(", ")
            ));
        } else if let Err(e) = Ty::Range.check(range, "") {
            bad.push(format!("`{k}`: {e}"));
        } else if let (Some(max), Some(hi)) = (unit_max, range.get(1).and_then(Value::as_f64)) {
            if hi > max || range[0].as_f64().is_some_and(|lo| lo < 0.0) {
                bad.push(format!("`{k}`: a gain is between 0 and {max}, got {range}"));
            }
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(bad.join("; "))
    }
}

fn valid_calls(v: &Value) -> Result<(), String> {
    let mut names: Vec<&str> = Call::ALL.iter().map(|c| c.name()).collect();
    names.push("any");
    name_ranges(v, "call", &names, None)
}

fn valid_beds(v: &Value) -> Result<(), String> {
    let names: Vec<&str> = Bed::ALL.iter().map(|b| b.name()).collect();
    name_ranges(v, "bed", &names, Some(1.0))
}

/// Everything wrong with a `checks.audio` block before any sound is rendered, as `path: message` lines: unknown keys (with a likely fix), values of the wrong type,
/// names that are not sounds, calls or beds, and items that assert nothing. A key that is misspelled never becomes a check that quietly passes.
pub fn invalid_fields(block: &Value) -> Vec<String> {
    let mut errs = Vec::new();
    let Some(o) = block.as_object() else { return vec!["checks.audio: must be an object like {\"scores\": {}}".into()] };
    check_keys(&mut errs, "checks.audio", o, AUDIO_CHECK_KEYS);
    if !AUDIO_CHECK_KEYS.iter().any(|k| o.contains_key(*k)) {
        errs.push("checks.audio: checks nothing; give `scores`, `sounds` or `soundscape`".into());
    }
    if let Some(sc) = o.get("scores") {
        match sc.as_object() {
            Some(m) => check_fields(&mut errs, "checks.audio.scores", m, SCORES_FIELDS),
            None => errs
                .push(format!("checks.audio.scores: expected an object like {{\"lufs\": [-34, -22]}} (or {{}} for the defaults), got {}", describe_value(sc))),
        }
    }
    let each = |errs: &mut Vec<String>, key: &str, fields: &[Field], rule: fn(&Map<String, Value>) -> Option<String>| {
        let Some(v) = o.get(key) else { return };
        let Some(list) = v.as_array() else {
            errs.push(format!("checks.audio.{key}: expected a list of objects, got {}", describe_value(v)));
            return;
        };
        if list.is_empty() {
            errs.push(format!("checks.audio.{key}: is empty, so it checks nothing; add an item or remove it"));
        }
        for (i, item) in list.iter().enumerate() {
            let path = format!("checks.audio.{key}[{i}]");
            match item.as_object() {
                Some(m) => {
                    check_fields(errs, &path, m, fields);
                    if let Some(e) = rule(m) {
                        errs.push(format!("{path}: {e}"));
                    }
                }
                None => errs.push(format!("{path}: expected an object, got {}", describe_value(item))),
            }
        }
    };
    each(&mut errs, "sounds", SOUND_FIELDS, |_| None);
    each(&mut errs, "soundscape", SOUNDSCAPE_FIELDS, |m| {
        let said = |k: &str| m.get(k).and_then(Value::as_object).is_some_and(|o| !o.is_empty());
        (!said("calls") && !said("beds"))
            .then(|| "asserts nothing: give `calls` (how often each bird is heard) and/or `beds` (how loud each background sound settles)".to_string())
    });
    errs
}

type Row = (String, bool, String);

fn range(v: Option<&Value>, what: &str) -> Result<Option<(f64, f64)>, String> {
    let Some(v) = v else { return Ok(None) };
    match v.as_array().map(|a| a.iter().map(Value::as_f64).collect::<Vec<_>>()).as_deref() {
        Some([Some(lo), Some(hi)]) if lo <= hi => Ok(Some((*lo, *hi))),
        _ => Err(format!("{what}: expected [min, max] with min <= max")),
    }
}

fn judge(name: String, s: &crate::tools::audio::Source, lufs: Option<(f64, f64)>, peak_max: f64, seam_max: Option<f64>, max_secs: Option<f64>) -> Row {
    let r = crate::tools::audio::report_of(s);
    let mut bad = problems(&r, s.kind);
    if let Some((lo, hi)) = lufs {
        if (r.lufs as f64) < lo || (r.lufs as f64) > hi {
            bad.push(format!("{:.1} LUFS is outside {lo} to {hi}", r.lufs));
        }
    }
    if r.peak_dbfs as f64 > peak_max {
        bad.push(format!("peak {:.1} dBFS is above {peak_max}", r.peak_dbfs));
    }
    if seam_max.is_some() && s.kind == Kind::OneShot {
        // A loop seam is the jump from the end back to the start; a one-shot never plays that join, so a ceiling on it can never fail and would only look like a check.
        bad.push(
            "`seam_max` is a loop measurement and this is a one-shot (it has no `tracks`): drop it (a one-shot's ending is already held to the standard)"
                .to_string(),
        );
    } else if let Some(m) = seam_max.filter(|m| (r.seam as f64) > *m) {
        bad.push(format!("seam {:.1} is above {m}", r.seam));
    }
    if let Some(m) = max_secs.filter(|m| (r.secs as f64) > *m) {
        bad.push(format!("{:.1} s is longer than {m} s", r.secs));
    }
    let detail = format!(
        "{:.1} LUFS, peak {:.1} dBFS, {:.1} s{}",
        r.lufs,
        r.peak_dbfs,
        r.secs,
        if bad.is_empty() { String::new() } else { format!(": {}", bad.join("; ")) }
    );
    (name, bad.is_empty(), detail)
}

fn biome_of(name: &str) -> Option<Biome> {
    [Biome::Meadow, Biome::Wildflowers, Biome::Grove, Biome::Forest, Biome::Pinewood, Biome::Glade]
        .into_iter()
        .find(|b| b.name().replace(' ', "_") == name || b.name() == name)
}

/// The scene's `checks.audio` as `verify` rows `(name, ok, detail)`.
pub fn verify_checks(path: &Path, block: &Value) -> Result<Vec<Row>, String> {
    let errs = invalid_fields(block);
    if !errs.is_empty() {
        return Err(errs.join("; "));
    }
    let Some(o) = block.as_object() else { return Err("checks.audio: must be an object like {\"scores\": {}}".into()) };
    let mut rows = Vec::new();
    if let Some(sc) = o.get("scores") {
        let scene = crate::load_scene(path).map_err(|e| e.join("; "))?;
        let lufs = range(sc.get("lufs"), "checks.audio.scores.lufs")?.unwrap_or((-34.0, -20.0));
        let peak = sc.get("peak_max").and_then(Value::as_f64).unwrap_or(-3.0);
        match scene.audio.as_ref().filter(|a| !a.music.is_empty()) {
            None => rows.push(("audio scores".into(), false, "the scene has no `audio.music` scores to check".into())),
            Some(a) => {
                for (mood, file) in &a.music {
                    let name = format!("audio score {}", mood.name());
                    match crate::tools::audio::source(&file.display().to_string()) {
                        Ok(s) if s.kind == Kind::Loop => rows.push(judge(name, &s, Some(lufs), peak, None, None)),
                        Ok(_) => rows.push((name, false, format!("{} is a sound, not a score (it has no `tracks`)", file.display()))),
                        Err(e) => rows.push((name, false, e)),
                    }
                }
            }
        }
    }
    for (i, item) in o.get("sounds").and_then(Value::as_array).into_iter().flatten().enumerate() {
        let name = item.get("name").and_then(Value::as_str).map_or_else(|| format!("audio sounds[{i}]"), |n| format!("audio {n}"));
        let Some(sound) = item.get("sound").and_then(Value::as_str) else {
            rows.push((name, false, "needs a `sound` (a built-in like bird.robin, or a .json/.wav file)".into()));
            continue;
        };
        let lufs = range(item.get("lufs"), &format!("checks.audio.sounds[{i}].lufs"))?;
        let file = if sound.ends_with(".json") || sound.ends_with(".wav") {
            path.parent().unwrap_or(Path::new(".")).join(sound).display().to_string()
        } else {
            sound.to_string()
        };
        match crate::tools::audio::source(&file) {
            Ok(s) => rows.push(judge(
                name,
                &s,
                lufs,
                item.get("peak_max").and_then(Value::as_f64).unwrap_or(-1.0),
                item.get("seam_max").and_then(Value::as_f64),
                item.get("max_secs").and_then(Value::as_f64),
            )),
            Err(e) => rows.push((name, false, e)),
        }
    }
    for (i, item) in o.get("soundscape").and_then(Value::as_array).into_iter().flatten().enumerate() {
        let name = item.get("name").and_then(Value::as_str).map_or_else(|| format!("audio soundscape[{i}]"), |n| format!("audio {n}"));
        let Some(sun) = item.get("sun").and_then(Value::as_f64) else {
            rows.push((name, false, "needs `sun`: the sun's height in degrees (negative at night)".into()));
            continue;
        };
        let biome = match item.get("biome").and_then(Value::as_str) {
            None => Biome::Meadow,
            Some(b) => {
                biome_of(b).ok_or_else(|| format!("checks.audio.soundscape[{i}].biome: `{b}` is not meadow, wildflowers, grove, forest, pinewood or glade"))?
            }
        };
        let ctx = Context {
            sun_elev_deg: sun as f32,
            rising: item.get("rising").and_then(Value::as_bool).unwrap_or(true),
            biome,
            speed: item.get("speed").and_then(Value::as_f64).unwrap_or(0.0) as f32,
        };
        let secs = item.get("secs").and_then(Value::as_f64).unwrap_or(300.0).clamp(1.0, 7200.0);
        // The countryside the player hears is the one the scene's `audio` block configures (`birds: false` silences songbirds); a check that ignored it would count calls nobody hears.
        let scene_birds = crate::load_scene(path).ok().and_then(|s| s.audio).is_none_or(|a| a.birds);
        let mut a = Ambience::new(item.get("seed").and_then(Value::as_u64).unwrap_or(1) as u32);
        a.birds_off = !item.get("birds").and_then(Value::as_bool).unwrap_or(scene_birds);
        let mut t = 0.0;
        while t < secs {
            a.step(0.1, &ctx);
            t += 0.1;
        }
        let mut bad = Vec::new();
        let mut said = Vec::new();
        if let Some(calls) = item.get("calls").and_then(Value::as_object) {
            for (who, want) in calls {
                let (lo, hi) = range(Some(want), &format!("checks.audio.soundscape[{i}].calls.{who}"))?.unwrap_or((0.0, f64::MAX));
                let n = if who == "any" {
                    a.totals.iter().sum::<u32>()
                } else {
                    let c = Call::ALL.iter().position(|c| c.name() == who).ok_or_else(|| {
                        format!("checks.audio.soundscape[{i}].calls.{who}: no such call (robin, blackbird, wren, chaffinch, cuckoo, pigeon, owl, or any)")
                    })?;
                    a.totals[c]
                };
                said.push(format!("{who} {n}"));
                if (n as f64) < lo || (n as f64) > hi {
                    bad.push(format!("{who}: {n} calls in {secs:.0} s, wanted {lo} to {hi}"));
                }
            }
        }
        if let Some(beds) = item.get("beds").and_then(Value::as_object) {
            let targets = Ambience::bed_targets(&ctx);
            for (which, want) in beds {
                let (lo, hi) = range(Some(want), &format!("checks.audio.soundscape[{i}].beds.{which}"))?.unwrap_or((0.0, 1.0));
                let k = Bed::ALL
                    .iter()
                    .position(|b| b.name() == which)
                    .ok_or_else(|| format!("checks.audio.soundscape[{i}].beds.{which}: no such bed (wind, leaves, crickets, night_air, bees)"))?;
                let g = targets[k] as f64;
                said.push(format!("{which} {g:.2}"));
                if g < lo || g > hi {
                    bad.push(format!("{which} bed settles at {g:.2}, wanted {lo} to {hi}"));
                }
            }
        }
        let detail = if bad.is_empty() { said.join(", ") } else { bad.join("; ") };
        rows.push((name, bad.is_empty(), detail));
    }
    if rows.is_empty() {
        rows.push(("audio".into(), false, "checks.audio has nothing to check: give `scores`, `sounds` or `soundscape`".into()));
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A tiny scene with one short score, written to a temp folder (rendering Marcel's real scores takes seconds, which a unit test should not).
    fn marcel() -> std::path::PathBuf {
        static SCENE: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
        SCENE.get_or_init(fixture).clone()
    }

    fn fixture() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("re2_audio_checks_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("day.json"),
            r#"{"bpm": 120, "beats": 4, "bars": 2, "key": "D", "scale": "major", "seed": 3, "lufs": -27,
                "reverb": {"decay": 2, "mix": 0.3},
                "instruments": {"pad": {"seconds": 4, "level": 0.5, "layers": [{"sine": 1, "attack": 0.5, "release": 1}]}},
                "tracks": [{"inst": "pad", "play": "chords", "chords": "I V", "every": "1 bar", "octave": 3}]}"#,
        )
        .unwrap();
        // The same countryside with the songbirds switched off (`audio.birds: false`), for the check that must hear what the player hears.
        std::fs::write(
            dir.join("quiet.json"),
            r#"{"camera": {"position": [0, 1, 0], "target": [0, 1, -1]}, "audio": {"ambience": "nature", "birds": false}, "objects": []}"#,
        )
        .unwrap();
        let scene = dir.join("scene.json");
        std::fs::write(
            &scene,
            r#"{"camera": {"position": [0, 1, 0], "target": [0, 1, -1]}, "audio": {"ambience": "nature", "music": {"day": "day.json"}}, "objects": []}"#,
        )
        .unwrap();
        scene
    }

    fn all_ok(rows: &[Row]) -> bool {
        rows.iter().all(|r| r.1)
    }

    #[test]
    fn marcels_scores_meet_the_standard_and_a_tight_window_fails_them() {
        let rows = verify_checks(&marcel(), &json!({"scores": {}})).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(all_ok(&rows), "{rows:?}");
        let tight = verify_checks(&marcel(), &json!({"scores": {"lufs": [-23, -20]}})).unwrap();
        assert!(tight.iter().all(|r| !r.1) && tight[0].2.contains("outside"), "{tight:?}");
    }

    #[test]
    fn sounds_are_held_to_what_the_author_asks() {
        let ok = verify_checks(&marcel(), &json!({"sounds": [{"name": "robin", "sound": "bird.robin", "lufs": [-30, -18], "max_secs": 3}]})).unwrap();
        assert!(all_ok(&ok), "{ok:?}");
        let bad = verify_checks(
            &marcel(),
            &json!({"sounds": [{"sound": "bird.robin", "max_secs": 0.5}, {"sound": "no.such"}, {"sound": "bird.robin", "seam_max": 2}]}),
        )
        .unwrap();
        assert!(bad.iter().all(|r| !r.1), "{bad:?}");
        assert!(bad[0].2.contains("longer") && bad[1].2.contains("no.such") && bad[2].2.contains("loop measurement"), "{bad:?}");
    }

    #[test]
    fn a_soundscape_check_says_what_is_heard_at_an_hour() {
        let block = json!({"soundscape": [
            {"name": "dawn", "sun": 4, "rising": true, "secs": 300, "calls": {"any": [30, 1000], "owl": [0, 0]}, "beds": {"crickets": [0, 0.05]}},
            {"name": "night", "sun": -40, "rising": false, "biome": "forest", "secs": 900, "calls": {"owl": [4, 200], "robin": [0, 0]}, "beds": {"crickets": [0.4, 1]}}]});
        let rows = verify_checks(&marcel(), &block).unwrap();
        assert!(all_ok(&rows), "{rows:?}");
        let wrong = verify_checks(&marcel(), &json!({"soundscape": [{"sun": 60, "calls": {"owl": [1, 5]}}]})).unwrap();
        assert!(!wrong[0].1 && wrong[0].2.contains("owl"), "{wrong:?}");
    }

    #[test]
    fn typos_and_empty_blocks_are_reported() {
        assert!(verify_checks(&marcel(), &json!({"score": {}})).unwrap_err().contains("score"));
        assert!(verify_checks(&marcel(), &json!({"soundscape": [{"sun": 1, "biome": "jungle"}]})).unwrap_err().contains("jungle"));
        assert!(verify_checks(&marcel(), &json!({"soundscape": [{"sun": 1, "calls": {"dodo": [0, 1]}}]})).unwrap_err().contains("dodo"));
        assert!(verify_checks(&marcel(), &json!({"soundscape": [{"sun": 1, "calls": {"owl": [3, 1]}}]})).unwrap_err().contains("min <= max"));
        assert!(verify_checks(&marcel(), &json!({})).unwrap_err().contains("checks nothing"));
    }

    /// What an author hears is what the check hears: the scene says `audio.birds: false`, so a dawn has no songbirds in the check as it does in the game (the check used to
    /// count 190 calls nobody would hear). An explicit `birds` on the item overrides the scene, so a check can still assert the chorus.
    #[test]
    fn the_soundscape_check_honours_the_scenes_birds_setting() {
        let quiet_scene = marcel().with_file_name("quiet.json");
        let quiet = json!({"soundscape": [{"name": "dawn", "sun": 4, "secs": 300, "calls": {"any": [0, 0]}}]});
        assert!(all_ok(&verify_checks(&quiet_scene, &quiet).unwrap()), "the scene switches songbirds off");
        assert!(!all_ok(&verify_checks(&marcel(), &quiet).unwrap()), "a scene that leaves them on has a dawn chorus: the same check fails");
        let loud = json!({"soundscape": [{"name": "dawn", "sun": 4, "secs": 300, "birds": true, "calls": {"any": [0, 0]}}]});
        let rows = verify_checks(&quiet_scene, &loud).unwrap();
        assert!(!rows[0].1 && rows[0].2.contains("any:"), "with `birds: true` on the item the quiet scene's check must fail: {rows:?}");
    }

    /// Plausible AI mistakes, each of which used to be ignored, defaulted or clamped, now one message at its path.
    #[test]
    fn plausible_audio_check_mistakes_are_each_reported_where_they_are() {
        let robin = |extra: Value| {
            let mut m = json!({"sound": "bird.robin"});
            m.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            json!({"sounds": [m]})
        };
        let scape = |extra: Value| {
            let mut m = json!({"sun": 4, "calls": {"robin": [0, 9]}});
            m.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            json!({"soundscape": [m]})
        };
        let cases = [
            ("a misspelled score ceiling", json!({"scores": {"peek_max": -3}}), "checks.audio.scores.peek_max: unknown field — did you mean `peak_max`?"),
            ("a loudness window backwards", json!({"scores": {"lufs": [-20, -34]}}), "checks.audio.scores.lufs: expected [min, max]"),
            (
                "a loudness window as a string",
                json!({"scores": {"lufs": "-30"}}),
                "checks.audio.scores.lufs: expected [min, max]: two numbers with min <= max (LUFS, default [-34, -20]), got string \"-30\"",
            ),
            ("scores as a list", json!({"scores": []}), "checks.audio.scores: expected an object"),
            ("a peak ceiling above full scale", json!({"scores": {"peak_max": 3}}), "checks.audio.scores.peak_max: expected a number from -120 to 0"),
            ("a truncated assertion name", robin(json!({"seam": 2})), "checks.audio.sounds[0].seam: unknown field — did you mean `seam_max`?"),
            (
                "a ceiling as a string",
                robin(json!({"peak_max": "-3"})),
                "checks.audio.sounds[0].peak_max: expected a number from -120 to 0 (dBFS, default -1), got string \"-3\"",
            ),
            ("a sound with no name of a sound", json!({"sounds": [{"lufs": [-30, -18]}]}), "checks.audio.sounds[0]: needs `sound`"),
            ("an empty sounds list", json!({"sounds": []}), "checks.audio.sounds: is empty, so it checks nothing"),
            ("a sounds list that is an object", json!({"sounds": {"sound": "bird.robin"}}), "checks.audio.sounds: expected a list of objects"),
            ("a soundscape that asserts nothing", json!({"soundscape": [{"sun": 4}]}), "checks.audio.soundscape[0]: asserts nothing"),
            ("a soundscape with empty calls", scape(json!({"calls": {}})), "checks.audio.soundscape[0].calls: is empty"),
            ("a bird that does not exist", scape(json!({"calls": {"robn": [0, 1]}})), "`robn` is not a call — did you mean `robin`?"),
            ("a bed that does not exist", scape(json!({"beds": {"cricket": [0.1, 1]}})), "`cricket` is not a bed — did you mean `crickets`?"),
            ("a bed gain above one", scape(json!({"beds": {"wind": [0.2, 1.5]}})), "a gain is between 0 and 1"),
            ("a call range as a number", scape(json!({"calls": {"robin": 3}})), "`robin`: expected [min, max]"),
            ("the sun as a string", scape(json!({"sun": "4"})), "checks.audio.soundscape[0].sun: expected a number from -90 to 90"),
            ("the sun above the sky", scape(json!({"sun": 400})), "checks.audio.soundscape[0].sun: expected a number from -90 to 90"),
            ("rising as a string", scape(json!({"rising": "false"})), "checks.audio.soundscape[0].rising: expected true or false"),
            ("a duration that used to be clamped", scape(json!({"secs": 99999})), "checks.audio.soundscape[0].secs: expected a number from 1 to 7200"),
            ("a seed that is negative", scape(json!({"seed": -1})), "checks.audio.soundscape[0].seed: expected a whole number"),
            ("a biome typo", scape(json!({"biome": "forrest"})), "`forrest` is not a biome"),
            ("a group that is not a check", json!({"soundscapes": []}), "checks.audio.soundscapes: unknown field — did you mean `soundscape`"),
            ("an empty block", json!({}), "checks.audio: checks nothing"),
            ("a non-object block", json!("scores"), "checks.audio: must be an object"),
        ];
        for (what, block, want) in cases {
            let got = invalid_fields(&block).join(" | ");
            assert!(got.contains(want), "{what}: wanted `{want}` in `{got}`");
            // and the runner refuses the same block instead of running a partial check
            let err = verify_checks(&marcel(), &block).expect_err(what);
            assert!(err.contains(want), "{what}: the runner says `{err}`");
        }
    }

    #[test]
    fn a_well_formed_block_is_accepted_whole() {
        let block = json!({
            "scores": {"lufs": [-34, -22], "peak_max": -3},
            "sounds": [{"name": "r", "sound": "bird.robin", "lufs": [-30, -18], "peak_max": -1, "max_secs": 3, "x-note": "quiet"}],
            "soundscape": [{"name": "n", "sun": -40, "rising": false, "biome": "forest", "secs": 600, "seed": 3, "speed": 1.5, "birds": true,
                "calls": {"owl": [3, 100], "any": [0, 500]}, "beds": {"crickets": [0.5, 1]}}]
        });
        assert_eq!(invalid_fields(&block), Vec::<String>::new());
    }
}
