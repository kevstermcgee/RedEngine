//! `red_engine2 audio`: hear nothing, know everything. Lists the engine's built-in sounds, measures them (loudness, peaks, clipping, silence, the loop
//! seam, brightness, pitch), renders them to WAV, draws a picture (waveform over spectrogram) and holds them to a standard. Works in the headless
//! build: the synthesis is pure arithmetic (`crate::sfx`, `crate::music`, `crate::synth`), only playback needs a sound card.
//!
//! The numbers and the picture are the evidence an AI author or CI has instead of ears (`audio_analysis`).

use crate::audio_analysis::{analyze, problems, read_wav, spectrogram, warnings, Clip, Kind, Report};
use crate::dsp::Voice;
use crate::synth::{wav_bytes_i16, SAMPLE_RATE};
use crate::tools::font::draw_text;
use image::{Rgb, RgbImage};
use serde_json::{json, Value};

/// A named generator of one mono clip.
type Generator = (&'static str, fn() -> Vec<f32>);
/// A named pick of one clip out of the tactical sound bank.
type BankPick = (&'static str, fn(&crate::sfx::SoundBank) -> Vec<f32>);

/// One built-in sound.
pub struct Entry {
    /// Dotted name: `gun.pistol`, `fx.explosion`, `music.loop`.
    pub name: String,
    /// What it is for: `gun`, `fx`, `tactical`, `synth`, `music`, `ambience`.
    pub group: &'static str,
    /// Plays once, or loops.
    pub kind: Kind,
    /// 1 or 2.
    pub channels: usize,
    /// The sound as data, when it is built as a [`Voice`] (what `audio export` prints).
    pub voice: Option<Voice>,
    render: Box<dyn Fn() -> Vec<f32>>,
}

impl Entry {
    fn new(name: impl Into<String>, group: &'static str, kind: Kind, channels: usize, render: impl Fn() -> Vec<f32> + 'static) -> Entry {
        Entry { name: name.into(), group, kind, channels, voice: None, render: Box::new(render) }
    }

    fn voiced(name: String, group: &'static str, voice: Voice) -> Entry {
        let v = voice.clone();
        Entry { name, group, kind: Kind::OneShot, channels: 1, voice: Some(voice), render: Box::new(move || v.clone().render()) }
    }

    /// The samples (interleaved when stereo), generated fresh.
    pub fn samples(&self) -> Vec<f32> {
        (self.render)()
    }

    /// The report for this sound.
    pub fn report(&self) -> Report {
        analyze(Clip { samples: &self.samples(), channels: self.channels, rate: SAMPLE_RATE })
    }
}

/// Every sound the engine builds in code, in a stable order.
pub fn catalog() -> Vec<Entry> {
    use crate::sfx;
    // Everything built as a voice comes first: guns and the cues that are layers (these can be exported as JSON).
    let mut v: Vec<Entry> = sfx::voice_catalog()
        .into_iter()
        .map(|(name, voice)| {
            let group = if name.starts_with("gun.") { "gun" } else { "fx" };
            Entry::voiced(name, group, voice)
        })
        .collect();
    let one: [Generator; 14] = [
        ("respawn", sfx::respawn),
        ("alert", sfx::alert),
        ("go", sfx::go),
        ("explosion", sfx::explosion),
        ("flash_pop", sfx::flash_pop),
        ("smoke_pop", sfx::smoke_pop),
        ("fire_burst", sfx::fire_burst),
        ("grenade_bounce", sfx::grenade_bounce),
        ("reload_mag", sfx::reload_mag),
        ("reload_shell", sfx::reload_shell),
        ("bolt_cycle", sfx::bolt_cycle),
        ("pickup", sfx::pickup),
        ("blade_swish", sfx::blade_swish),
        ("blade_hit", sfx::blade_hit),
    ];
    for (name, f) in one {
        v.push(Entry::new(format!("fx.{name}"), "fx", Kind::OneShot, 1, f));
    }
    v.push(Entry::new("fx.countdown_beep", "fx", Kind::OneShot, 1, || sfx::beep(880.0, 0.11)));
    v.push(Entry::new("synth.bat_hit", "synth", Kind::OneShot, 1, crate::synth::synth_bat_hit));
    v.push(Entry::new("synth.weapon_click", "synth", Kind::OneShot, 1, crate::synth::synth_weapon_click));
    let tactical: [BankPick; 8] = [
        ("hit_tick", |b| b.hit_tick.clone()),
        ("kill", |b| b.kill.clone()),
        ("respawn", |b| b.respawn.clone()),
        ("beep", |b| b.beep.clone()),
        ("go", |b| b.go.clone()),
        ("victory", |b| b.victory.clone()),
        ("defeat", |b| b.defeat.clone()),
        ("jump", |b| b.jump.clone()),
    ];
    for (name, pick) in tactical {
        v.push(Entry::new(format!("tactical.{name}"), "tactical", Kind::OneShot, 1, move || pick(&sfx::SoundBank::new_tactical())));
    }
    v.push(Entry::new("music.loop", "music", Kind::Loop, 2, crate::music::loop_samples));
    v.push(Entry::new("ambience.map", "ambience", Kind::Loop, 2, || sfx::ambience(20.0)));
    v
}

/// The entry called `name`, or an error with the closest names.
pub fn find(name: &str) -> Result<Entry, String> {
    let all = catalog();
    let names: Vec<String> = all.iter().map(|e| e.name.clone()).collect();
    match all.into_iter().find(|e| e.name == name) {
        Some(e) => Ok(e),
        None => {
            let hint = crate::prefabs::suggest(name, names.iter().map(String::as_str));
            Err(format!(
                "no built-in sound `{name}`{} (`red_engine2 audio list` shows them)",
                hint.first().map(|h| format!(" — did you mean `{h}`?")).unwrap_or_default()
            ))
        }
    }
}

/// `audio export`: the JSON of a built-in sound that is built as a voice.
pub fn export_json(name: &str) -> Result<String, String> {
    let e = find(name)?;
    match e.voice {
        Some(v) => Ok(serde_json::to_string_pretty(&crate::voice_spec::voice_to_json(&v)).unwrap_or_default() + "\n"),
        None => Err(format!(
            "`{name}` is still hand-written code, not a voice, so it has no JSON form (voices: {})",
            catalog().iter().filter(|e| e.voice.is_some()).count()
        )),
    }
}

/// `audio list`: name, group, kind, length.
pub fn list_text() -> String {
    let mut out = String::new();
    for e in catalog() {
        let r = e.report();
        out.push_str(&format!(
            "{:<26} {:<9} {:<8} {:>5.2}s {}{}\n",
            e.name,
            e.group,
            if e.kind == Kind::Loop { "loop" } else { "one-shot" },
            r.secs,
            if e.channels == 2 { "stereo" } else { "mono" },
            if e.voice.is_some() { " json" } else { "" }
        ));
    }
    out
}

/// A sound to measure: a built-in by name, a `.wav` file or a sound `.json` file.
pub struct Source {
    /// What to call it.
    pub name: String,
    /// Interleaved samples.
    pub samples: Vec<f32>,
    /// 1 or 2.
    pub channels: usize,
    /// Sample rate.
    pub rate: u32,
    /// How to judge it.
    pub kind: Kind,
    /// Which group of sounds it belongs to (`file` for a WAV), for loudness comparisons.
    pub group: &'static str,
}

impl Source {
    fn of(e: &Entry) -> Source {
        Source { name: e.name.clone(), samples: e.samples(), channels: e.channels, rate: SAMPLE_RATE, kind: e.kind, group: e.group }
    }
}

/// Resolves `name` (a built-in, or a path ending `.wav`) into samples.
pub fn source(name: &str) -> Result<Source, String> {
    if name.to_lowercase().ends_with(".wav") {
        let bytes = std::fs::read(name).map_err(|e| format!("{name}: {e}"))?;
        let (samples, channels, rate) = read_wav(&bytes).map_err(|e| format!("{name}: {e}"))?;
        return Ok(Source { name: name.to_string(), samples, channels, rate, kind: Kind::OneShot, group: "file" });
    }
    if name.to_lowercase().ends_with(".json") {
        let text = std::fs::read_to_string(name).map_err(|e| format!("{name}: {e}"))?;
        let value: Value = serde_json::from_str(&text).map_err(|e| format!("{name}: not valid JSON: {e}"))?;
        let voice = crate::voice_spec::parse_voice(&value).map_err(|errs| format!("{name}: not a valid sound:\n  {}", errs.join("\n  ")))?;
        return Ok(Source { name: name.to_string(), samples: voice.render(), channels: 1, rate: SAMPLE_RATE, kind: Kind::OneShot, group: "file" });
    }
    let e = find(name)?;
    Ok(Source::of(&e))
}

/// The report for a source.
pub fn report_of(s: &Source) -> Report {
    analyze(Clip { samples: &s.samples, channels: s.channels, rate: s.rate })
}

/// `audio report`: one line per sound, or JSON; `names` empty means every built-in.
pub fn report_text(names: &[String], json_out: bool) -> Result<String, String> {
    let sources: Vec<Source> =
        if names.is_empty() { catalog().iter().map(Source::of).collect() } else { names.iter().map(|n| source(n)).collect::<Result<_, _>>()? };
    let rows: Vec<(String, Report)> = sources.iter().map(|s| (s.name.clone(), report_of(s))).collect();
    if json_out {
        return Ok(serde_json::to_string_pretty(
            &json!({ "sounds": rows.iter().map(|(n, r)| json!({"name": n, "report": r.to_json()})).collect::<Vec<Value>>() }),
        )
        .unwrap_or_default()
            + "\n");
    }
    let mut out = String::new();
    for (n, r) in &rows {
        out.push_str(&format!("{n:<26} {}\n", r.line()));
    }
    if rows.len() > 1 {
        let lufs: Vec<f32> = rows.iter().map(|(_, r)| r.lufs).filter(|l| *l > -100.0).collect();
        if let (Some(lo), Some(hi)) = (lufs.iter().copied().reduce(f32::min), lufs.iter().copied().reduce(f32::max)) {
            out.push_str(&format!("-- {} sounds, loudness {lo:.1} to {hi:.1} LUFS ({:.1} LU spread)\n", rows.len(), hi - lo));
        }
    }
    Ok(out)
}

/// `audio render`: the sound as a 16-bit WAV file.
pub fn render_wav(name: &str, out: &std::path::Path) -> Result<String, String> {
    let s = source(name)?;
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(out, wav_bytes_i16(&s.samples, s.rate, s.channels as u16)).map_err(|e| format!("{}: {e}", out.display()))?;
    let r = report_of(&s);
    Ok(format!(
        "wrote {} ({:.2} s, {} Hz, {}) {:.1} LUFS peak {:.1} dBFS\n",
        out.display(),
        r.secs,
        s.rate,
        if s.channels == 2 { "stereo" } else { "mono" },
        r.lufs,
        r.peak_dbfs
    ))
}

/// The median loudness of each group (`gun`, `fx`, ...), for the outlier warnings.
fn group_medians(rows: &[(String, &'static str, Report)]) -> std::collections::HashMap<&'static str, f32> {
    let mut by: std::collections::HashMap<&'static str, Vec<f32>> = std::collections::HashMap::new();
    for (_, g, r) in rows {
        if r.lufs > -100.0 {
            by.entry(*g).or_default().push(r.lufs);
        }
    }
    by.into_iter()
        .filter(|(_, v)| v.len() >= 4)
        .map(|(g, mut v)| {
            v.sort_by(f32::total_cmp);
            (g, v[v.len() / 2])
        })
        .collect()
}

/// `audio check`: each sound against the standard (FAIL lines), plus loudness outliers within a group (WARN lines, which do not fail). Returns the
/// text and whether every sound passed.
pub fn check_text(names: &[String]) -> Result<(String, bool), String> {
    let sources: Vec<Source> =
        if names.is_empty() { catalog().iter().map(Source::of).collect() } else { names.iter().map(|n| source(n)).collect::<Result<_, _>>()? };
    let rows: Vec<(String, &'static str, Report)> = sources.iter().map(|s| (s.name.clone(), s.group, report_of(s))).collect();
    let medians = group_medians(&rows);
    let (mut out, mut failed, mut warned) = (String::new(), 0, 0);
    for (s, (_, group, r)) in sources.iter().zip(&rows) {
        let p = problems(r, s.kind);
        let w = if s.kind == Kind::OneShot { warnings(r, medians.get(group).copied()) } else { Vec::new() };
        if !p.is_empty() {
            failed += 1;
            out.push_str(&format!("FAIL {}\n", s.name));
            p.iter().for_each(|line| out.push_str(&format!("     {line}\n")));
        }
        if !w.is_empty() {
            warned += 1;
            out.push_str(&format!("WARN {}\n", s.name));
            w.iter().for_each(|line| out.push_str(&format!("     {line}\n")));
        }
    }
    out.push_str(&format!("{} sound(s) checked, {failed} failed, {warned} warning(s)\n", sources.len()));
    Ok((out, failed == 0))
}

/// The measurements of every built-in sound that identify how it sounds, rounded: what `audio golden` stores and compares. Pitch is kept only for
/// tonal sounds (a noisy spectrum's strongest bin can flip between neighbours on another platform without the sound changing).
pub fn golden() -> Value {
    let mut sounds = serde_json::Map::new();
    for e in catalog() {
        let r = e.report();
        let tonal = r.flatness < 0.1;
        sounds.insert(
            e.name.clone(),
            json!({
                "secs": (r.secs * 100.0).round() / 100.0, "peak_dbfs": (r.peak_dbfs * 10.0).round() / 10.0, "lufs": (r.lufs * 10.0).round() / 10.0,
                "centroid_hz": r.centroid_hz.round(), "bands_pct": r.bands.map(|b| b.round()), "clipped": r.clipped,
                "dominant_hz": if tonal { json!(r.dominant_hz.round()) } else { Value::Null },
            }),
        );
    }
    json!({ "format": 1, "note": "audio golden: how each built-in sound measures. Regenerate with `red_engine2 audio golden --write` and review the diff.", "sounds": sounds })
}

/// What differs between a stored golden and the current sounds, as sentences. Tolerances absorb the last-digit differences between platforms
/// (loudness 0.3 LU, peak 0.5 dB, length 10 ms, brightness 3%, a band 3 points, pitch 2%) and nothing audible.
pub fn golden_diff(expected: &Value, actual: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let (Some(e), Some(a)) = (expected["sounds"].as_object(), actual["sounds"].as_object()) else { return vec!["the golden file has no `sounds`".to_string()] };
    for name in e.keys().filter(|k| !a.contains_key(*k)) {
        out.push(format!("{name}: in the golden but no longer a built-in sound"));
    }
    for (name, now) in a {
        let Some(was) = e.get(name) else {
            out.push(format!("{name}: a new sound the golden does not know (run `audio golden --write`)"));
            continue;
        };
        let f = |v: &Value, k: &str| v[k].as_f64().unwrap_or(0.0);
        let mut note = |what: &str, was: f64, now: f64, tol: f64| {
            if (was - now).abs() > tol {
                out.push(format!("{name}: {what} {was} -> {now} (tolerance {tol})"));
            }
        };
        note("secs", f(was, "secs"), f(now, "secs"), 0.01);
        note("lufs", f(was, "lufs"), f(now, "lufs"), 0.3);
        note("peak_dbfs", f(was, "peak_dbfs"), f(now, "peak_dbfs"), 0.5);
        note("centroid_hz", f(was, "centroid_hz"), f(now, "centroid_hz"), (f(was, "centroid_hz") * 0.03).max(5.0));
        note("clipped", f(was, "clipped"), f(now, "clipped"), 0.0);
        for i in 0..5 {
            note(
                &format!("band {}", crate::audio_analysis::BAND_NAMES[i]),
                was["bands_pct"][i].as_f64().unwrap_or(0.0),
                now["bands_pct"][i].as_f64().unwrap_or(0.0),
                3.0,
            );
        }
        if let (Some(w), Some(n)) = (was["dominant_hz"].as_f64(), now["dominant_hz"].as_f64()) {
            note("pitch_hz", w, n, (w * 0.02).max(3.0));
        }
    }
    out
}

/// A viridis-like ramp for a dB value (-100 dark .. 0 bright).
fn heat(db: f32) -> Rgb<u8> {
    let t = ((db + 100.0) / 100.0).clamp(0.0, 1.0);
    let stops: [(f32, [f32; 3]); 5] =
        [(0.0, [8.0, 6.0, 24.0]), (0.3, [60.0, 30.0, 120.0]), (0.55, [180.0, 50.0, 100.0]), (0.8, [250.0, 150.0, 40.0]), (1.0, [255.0, 250.0, 200.0])];
    for w in stops.windows(2) {
        let ((t0, a), (t1, b)) = (w[0], w[1]);
        if t <= t1 {
            let k = (t - t0) / (t1 - t0);
            return Rgb([0, 1, 2].map(|i| (a[i] + (b[i] - a[i]) * k) as u8));
        }
    }
    Rgb([255, 250, 200])
}

/// `audio picture`: the waveform over the spectrogram, with the key numbers in the title line.
pub fn picture(s: &Source, width: u32, height: u32) -> RgbImage {
    let (w, h) = (width.max(320), height.max(200));
    let mut img = RgbImage::from_pixel(w, h, Rgb([14, 16, 24]));
    let title = 16u32;
    let wave_h = ((h - title) as f32 * 0.28) as u32;
    let spec_top = title + wave_h + 2;
    let spec_h = h - spec_top - 14;
    let mono: Vec<f32> = if s.channels == 2 { s.samples.chunks(2).map(|f| (f[0] + f[1]) / 2.0).collect() } else { s.samples.clone() };
    let r = report_of(s);
    draw_text(
        &mut img,
        4,
        4,
        &format!("{}  {:.2}s  {:.1} LUFS  peak {:.1}  {}", s.name, r.secs, r.lufs, r.peak_dbfs, crate::audio_analysis::note_name(r.dominant_hz)),
        1,
        Rgb([235, 238, 245]),
        Some(Rgb([0, 0, 0])),
    );
    // Waveform: the min/max of each column, mirrored about the centre line.
    let mid = (title + wave_h / 2) as i32;
    for x in 0..w {
        let a = mono.len() * x as usize / w as usize;
        let b = (mono.len() * (x as usize + 1) / w as usize).max(a + 1).min(mono.len());
        let (lo, hi) = mono[a.min(mono.len().saturating_sub(1))..b].iter().fold((0.0f32, 0.0f32), |(l, h), v| (l.min(*v), h.max(*v)));
        let (y0, y1) = (mid - (hi * (wave_h as f32 / 2.0)) as i32, mid - (lo * (wave_h as f32 / 2.0)) as i32);
        for y in y0.min(y1)..=y0.max(y1).max(y0.min(y1)) {
            if y >= title as i32 && y < (title + wave_h) as i32 {
                img.put_pixel(x, y as u32, Rgb([110, 200, 255]));
            }
        }
        if mid >= 0 {
            img.put_pixel(x, mid as u32, Rgb([40, 60, 80]));
        }
    }
    // Spectrogram: log frequency up, time right.
    let grid = spectrogram(&mono, s.rate, w as usize, spec_h as usize);
    for (row, line) in grid.iter().enumerate() {
        for (col, db) in line.iter().enumerate() {
            img.put_pixel(col as u32, spec_top + row as u32, heat(*db));
        }
    }
    // Frequency marks (100 Hz, 1k, 10k) and the end time.
    let nyquist = s.rate as f32 / 2.0 * 0.98;
    for (hz, label) in [(100.0f32, "100"), (1000.0, "1k"), (10000.0, "10k")] {
        if hz < nyquist {
            let t = 1.0 - (hz / 40.0).ln() / (nyquist / 40.0).ln();
            let y = spec_top + (t * spec_h as f32) as u32;
            for x in (0..w).step_by(8) {
                img.put_pixel(x, y.min(h - 1), Rgb([255, 255, 255]));
            }
            draw_text(&mut img, 2, y.saturating_sub(8) as i32, label, 1, Rgb([255, 255, 255]), Some(Rgb([0, 0, 0])));
        }
    }
    draw_text(&mut img, 4, (h - 11) as i32, &format!("0s{:>w$}{:.2}s", "", r.secs, w = ((w / 6).saturating_sub(10)) as usize), 1, Rgb([200, 205, 220]), None);
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_is_complete_unique_and_every_sound_renders_finite_audio() {
        let all = catalog();
        let mut names: Vec<&str> = all.iter().map(|e| e.name.as_str()).collect();
        let n = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), n, "every sound has its own name");
        assert!(n >= 60, "{n} sounds");
        assert!(all.iter().any(|e| e.name == "music.loop" && e.kind == Kind::Loop && e.channels == 2));
        assert!(all.iter().any(|e| e.name.starts_with("gun.r9")) && !all.iter().any(|e| e.name.contains("frag")), "melee and grenades are not guns");
        for e in &all {
            let s = e.samples();
            assert!(!s.is_empty() && s.len() % e.channels == 0 && s.iter().all(|v| v.is_finite()), "{}", e.name);
        }
    }

    #[test]
    fn every_builtin_sound_meets_the_standard() {
        let (text, ok) = check_text(&[]).unwrap();
        assert!(ok, "the engine's own sounds break its audio standard:\n{text}");
    }

    #[test]
    fn a_misspelt_name_gets_a_suggestion_and_wav_files_are_measured_too() {
        let e = find("fx.explosoin").err().unwrap();
        assert!(e.contains("did you mean `fx.explosion`"), "{e}");
        let dir = std::env::temp_dir().join(format!("re2_audio_tool_{}", std::process::id()));
        let wav = dir.join("boom.wav");
        let msg = render_wav("fx.explosion", &wav).unwrap();
        assert!(msg.contains("LUFS"), "{msg}");
        let back = source(wav.to_str().unwrap()).unwrap();
        let direct = report_of(&source("fx.explosion").unwrap());
        assert!((report_of(&back).lufs - direct.lufs).abs() < 0.3, "a 16-bit round trip keeps the loudness");
        let text = report_text(&["fx.explosion".into(), wav.to_string_lossy().to_string()], false).unwrap();
        assert!(text.contains("lufs") && text.contains("spread"), "{text}");
        let j: Value = serde_json::from_str(&report_text(&["fx.jump".into()], true).unwrap()).unwrap();
        assert!(j["sounds"][0]["report"]["lufs"].is_number());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn every_builtin_sound_still_sounds_like_its_golden() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio_golden.json");
        let stored: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("tests/fixtures/audio_golden.json (run `red_engine2 audio golden --write`)")).unwrap();
        let diff = golden_diff(&stored, &golden());
        assert!(
            diff.is_empty(),
            "a built-in sound changed; if that is intended run `red_engine2 audio golden --write` and review the diff:\n{}",
            diff.join("\n")
        );
    }

    #[test]
    fn the_golden_comparison_notices_each_kind_of_change_and_ignores_rounding() {
        let base = golden();
        assert!(golden_diff(&base, &base).is_empty());
        let mut louder = base.clone();
        louder["sounds"]["fx.jump"]["lufs"] = json!(louder["sounds"]["fx.jump"]["lufs"].as_f64().unwrap() + 1.0);
        assert!(golden_diff(&base, &louder).iter().any(|d| d.contains("fx.jump: lufs")));
        let mut nudged = base.clone();
        nudged["sounds"]["fx.jump"]["lufs"] = json!(nudged["sounds"]["fx.jump"]["lufs"].as_f64().unwrap() + 0.2);
        assert!(golden_diff(&base, &nudged).is_empty(), "platform noise is tolerated");
        let mut gone = base.clone();
        gone["sounds"].as_object_mut().unwrap().remove("fx.jump");
        assert!(golden_diff(&gone, &base).iter().any(|d| d.contains("a new sound")));
        assert!(golden_diff(&base, &gone).iter().any(|d| d.contains("no longer")));
    }

    #[test]
    fn the_picture_has_a_waveform_a_spectrogram_and_text() {
        let s = source("fx.explosion").unwrap();
        let img = picture(&s, 640, 300);
        assert_eq!(img.dimensions(), (640, 300));
        let bright = img.pixels().filter(|p| p.0.iter().map(|c| *c as u32).sum::<u32>() > 300).count();
        assert!(bright > 800, "something is drawn: {bright}");
        // The explosion starts loud: the left of the spectrogram is hotter than the far right.
        let heat_at = |x0: u32, x1: u32| {
            (x0..x1).flat_map(|x| (120..280).map(move |y| (x, y))).map(|(x, y)| img.get_pixel(x, y).0.iter().map(|c| *c as u32).sum::<u32>()).sum::<u32>()
        };
        assert!(heat_at(0, 60) > heat_at(580, 640), "an explosion decays");
    }
}
