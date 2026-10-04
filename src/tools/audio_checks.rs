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
use serde_json::Value;
use std::path::Path;

/// Keys of a `checks.audio` block.
pub const AUDIO_CHECK_KEYS: &[&str] = &["scores", "sounds", "soundscape"];

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
    if let Some(m) = seam_max.filter(|m| (r.seam as f64) > *m) {
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
    let Some(o) = block.as_object() else { return Err("checks.audio: must be an object like {\"scores\": {}}".into()) };
    let mut unknown = Vec::new();
    crate::strict::check_keys(&mut unknown, "checks.audio", o, AUDIO_CHECK_KEYS);
    if !unknown.is_empty() {
        return Err(unknown.join("; "));
    }
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
        let mut a = Ambience::new(item.get("seed").and_then(Value::as_u64).unwrap_or(1) as u32);
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
        let bad = verify_checks(&marcel(), &json!({"sounds": [{"sound": "bird.robin", "max_secs": 0.5}, {"sound": "no.such"}, {}]})).unwrap();
        assert!(bad.iter().all(|r| !r.1), "{bad:?}");
        assert!(bad[0].2.contains("longer") && bad[1].2.contains("no.such") && bad[2].2.contains("needs a `sound`"));
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
        assert!(!all_ok(&verify_checks(&marcel(), &json!({})).unwrap()));
    }
}
