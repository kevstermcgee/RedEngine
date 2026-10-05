//! The command-line side of 2D games: `validate`, `verify`, `sim`, `frame` and `capabilities` for a `*.game2d.json`.
//!
//! These are the same verbs as for 3D scenes (an AI learns one set), routed here by the file: a name ending `.game2d.json`, or a file whose first bytes name `"game2d"`. The game
//! itself (parser, simulation, renderer, sound) lives in the `red2d` crate, which the browser build also uses; this file only loads, runs and prints. Every report says what it proves
//! and what it does not: a green `verify` is a simulation, render and waveform claim, never a browser or a listening claim.

use red2d::caps;
use red2d::game::{self, GameDef};
use red2d::render;
use red2d::script::{self, Row};
use std::path::Path;
use std::sync::Arc;

/// Text for the terminal and whether the command succeeded.
pub struct Report {
    /// What to print.
    pub text: String,
    /// Exit code 0.
    pub ok: bool,
}

/// Whether a path names a 2D game (by name, or by `"game2d"` in its first bytes).
pub fn is_game(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name.ends_with(".game2d.json") {
        return true;
    }
    if !name.ends_with(".json") {
        return false;
    }
    use std::io::Read;
    let mut head = [0u8; 256];
    let n = std::fs::File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    String::from_utf8_lossy(&head[..n]).contains("\"game2d\"")
}

/// Reads and fully validates a game. The error is every problem, one per line, each prefixed with the file.
pub fn load(path: &Path) -> Result<(Arc<GameDef>, String), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    match game::parse(&text) {
        Ok(d) => Ok((Arc::new(d), text)),
        Err(errs) => Err(errs.iter().map(|e| format!("{}: {e}", path.display())).collect::<Vec<_>>().join("\n")),
    }
}

/// `validate`: parse everything and say what the game is.
pub fn validate(path: &Path) -> Report {
    match load(path) {
        Err(e) => Report { text: format!("{e}\n{} error(s)", e.lines().count()), ok: false },
        Ok((d, _)) => {
            let c = &d.caps;
            let mut t = format!(
                "OK: {} `{}` (revision {})\n  {} presentation, platforms {}, networking {}, input {}, saves {}\n  screen {}x{}, {} sprite(s), {} sound(s), {} music, {} prefab(s), {} placement(s), {} rule(s), {} HUD widget(s), {} scenario(s), {} browser check(s)\n",
                if c.presentation == caps::Presentation::TwoD { "2D game" } else { "game" },
                d.id,
                d.rev,
                c.presentation.name(),
                c.platforms.iter().map(|p| p.name()).collect::<Vec<_>>().join("+"),
                c.networking.name(),
                c.input.iter().map(|i| i.name()).collect::<Vec<_>>().join("+"),
                if c.persistence.is_empty() { "nothing".to_string() } else { c.persistence.iter().map(|p| p.name()).collect::<Vec<_>>().join("+") },
                d.view.width,
                d.view.height,
                d.sprites.len(),
                d.sounds.len(),
                d.music.len(),
                d.prefabs.len(),
                d.placements.len(),
                d.rules.len(),
                d.ui.len(),
                d.scenarios.len(),
                d.browser.len()
            );
            for w in caps::warnings(c) {
                t.push_str(&format!("  warning: {w}\n"));
            }
            t.push_str("  (validate proves the file is well formed and its names resolve; `verify` plays it)");
            Report { text: t, ok: true }
        }
    }
}

fn render_rows(rows: &[Row], only: Option<&str>) -> (String, usize, usize) {
    let (mut pass, mut fail) = (0, 0);
    let mut t = String::new();
    for r in rows {
        if let Some(o) = only {
            if !r.name.contains(o) && r.claim != o {
                continue;
            }
        }
        if r.ok {
            pass += 1;
        } else {
            fail += 1;
        }
        t.push_str(&format!("{}  [{}] {}: {}\n", if r.ok { "PASS" } else { "FAIL" }, r.claim, r.name, r.detail));
    }
    (t, pass, fail)
}

/// `verify`: every check the game can pass without a browser.
pub fn verify(path: &Path, only: Option<&str>) -> Report {
    let (d, _) = match load(path) {
        Ok(x) => x,
        Err(e) => return Report { text: format!("{e}\nFAIL: the game does not validate; nothing was run"), ok: false },
    };
    let started = std::time::Instant::now();
    let rows = script::verify(&d);
    let (mut t, pass, fail) = render_rows(&rows, only);
    for w in caps::warnings(&d.caps) {
        t.push_str(&format!("WARN  {w}\n"));
    }
    t.push_str(&format!("{} passed, {} failed in {:.1} s\n", pass, fail, started.elapsed().as_secs_f32()));
    t.push_str("PROVEN HERE: the scripted playthroughs run and end as asserted, deterministically (simulation); the first and last frames are not blank (CPU render); every sound is finite, unclipped and not silent (waveform).\n");
    t.push_str("NOT PROVEN HERE: that it runs in a browser (`red_engine2 web verify`), that anything is audible or sounds good, that it is fun.");
    Report { text: t, ok: fail == 0 && pass > 0 }
}

/// `sim`: each scenario's outcome in detail.
pub fn sim(path: &Path, only: Option<&str>, every: Option<f32>) -> Report {
    let (d, _) = match load(path) {
        Ok(x) => x,
        Err(e) => return Report { text: e, ok: false },
    };
    let mut t = String::new();
    let mut ok = true;
    let mut ran = 0;
    for sc in d.scenarios.iter().filter(|s| only.is_none_or(|o| s.name.contains(o))) {
        ran += 1;
        let (r, _) = script::run_scenario(&d, sc, None);
        ok &= r.ok;
        t.push_str(&format!(
            "{}  scenario `{}`: {} ticks ({:.1} s), ended {}, hash {}\n",
            if r.ok { "PASS" } else { "FAIL" },
            r.name,
            r.ticks,
            r.ticks as f64 / 60.0,
            r.ended.unwrap_or("not"),
            r.hash
        ));
        t.push_str(&format!("      vars: {}\n", r.vars.iter().map(|(n, v)| format!("{n}={v}")).collect::<Vec<_>>().join(" ")));
        if !r.ids.is_empty() {
            t.push_str(&format!("      at: {}\n", r.ids.iter().map(|(n, x, y)| format!("{n}=({x:.1}, {y:.1})")).collect::<Vec<_>>().join(" ")));
        }
        if !r.sounds.is_empty() {
            t.push_str(&format!("      sounds: {}\n", r.sounds.iter().map(|(n, c)| format!("{n} x{c}")).collect::<Vec<_>>().join(", ")));
        }
        if !r.events.is_empty() {
            t.push_str(&format!("      events: {}\n", r.events.iter().map(|(n, c)| format!("{n} x{c}")).collect::<Vec<_>>().join(", ")));
        }
        if let Some(step) = every.filter(|e| *e > 0.0) {
            let mut at = step;
            while at < r.ticks as f32 / 60.0 {
                let (rr, _) = script::run_scenario(&d, sc, Some((at * 60.0).round() as u64));
                t.push_str(&format!(
                    "      t={at:>6.1}s  {}\n",
                    rr.vars
                        .iter()
                        .map(|(n, v)| format!("{n}={}", if v.fract() == 0.0 { format!("{v}") } else { format!("{v:.1}") }))
                        .collect::<Vec<_>>()
                        .join(" ")
                ));
                at += step;
            }
        }
        for f in &r.failures {
            t.push_str(&format!("      FAILED: {f}\n"));
        }
        for n in &r.notes {
            t.push_str(&format!("      NOTE: {n}\n"));
        }
    }
    if ran == 0 {
        return Report {
            text: format!("no scenario{} in {} (add `checks.scenarios`)", only.map(|o| format!(" matching `{o}`")).unwrap_or_default(), path.display()),
            ok: false,
        };
    }
    Report { text: t.trim_end().to_string(), ok }
}

/// `frame`: a PNG of the game. With `scenario` (default: the first, if any) it plays that scenario for `t` seconds first; without any it draws the opening scene.
/// `size` presents the picture in a window of that shape (letterboxed, as the browser shows it).
pub fn frame(path: &Path, out: &Path, scenario: Option<&str>, t: f32, size: Option<(u32, u32)>) -> Result<String, String> {
    let (d, _) = load(path)?;
    let sc = match scenario {
        Some(n) => Some(
            d.scenarios
                .iter()
                .find(|s| s.name == n)
                .ok_or_else(|| format!("no scenario `{n}` (scenarios: {})", d.scenarios.iter().map(|s| s.name.clone()).collect::<Vec<_>>().join(", ")))?,
        ),
        None => d.scenarios.first().filter(|_| t > 0.0),
    };
    let sim = match sc {
        Some(s) => script::run_scenario(&d, s, Some((t * 60.0).round() as u64)).1,
        None => red2d::sim::Sim::new(d.clone(), 1),
    };
    let f = render::render(&sim);
    let stats = f.stats(d.view.background);
    let shown = match size {
        Some((w, h)) => render::present(&f, w, h, d.view.scale),
        None => f,
    };
    if let Some(dir) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let img = image::RgbaImage::from_raw(shown.w, shown.h, shown.rgba).ok_or("internal: bad frame size")?;
    img.save(out).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(format!(
        "wrote {} ({}x{}{}): {} colours, {:.1}% of the screen drawn, tick {}, hash {}\nThis is the same CPU renderer the browser uses; it shows the screen, not how it feels in motion.",
        out.display(),
        img.width(),
        img.height(),
        size.map(|_| format!(", window of the screen {}x{}", d.view.width, d.view.height)).unwrap_or_default(),
        stats.distinct_colors,
        stats.covered * 100.0,
        sim.tick,
        sim.hash_hex()
    ))
}

/// `capabilities`: the support matrix, a game's declaration against it, or a single question like `3d web`.
pub fn capabilities(path: Option<&Path>, query: &[String]) -> Result<Report, String> {
    if let Some(p) = path {
        let text = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{}: not JSON: {e}", p.display()))?;
        let Some(c) = v.get("capabilities") else {
            return Err(format!("{}: no `capabilities` block (a 2D game declares one; see `red_engine2 describe capabilities`)", p.display()));
        };
        let (parsed, problems) = caps::parse(c);
        let mut t = String::new();
        let Some(c) = parsed else {
            for pr in &problems {
                t.push_str(&format!("{}: {pr}\n", p.display()));
            }
            return Ok(Report { text: t.trim_end().to_string(), ok: false });
        };
        let mut all = problems;
        all.extend(caps::check(&c));
        for pr in &all {
            t.push_str(&format!("FAIL  {pr}\n"));
        }
        for plat in &c.platforms {
            let s = caps::support(c.presentation, *plat);
            t.push_str(&format!("{:<14} {} on {}\n", s.label(), c.presentation.name(), plat.name()));
        }
        for w in caps::warnings(&c) {
            t.push_str(&format!("WARN  {w}\n"));
        }
        return Ok(Report { text: t.trim_end().to_string(), ok: all.is_empty() });
    }
    if query.is_empty() {
        return Ok(Report { text: caps::matrix_text(), ok: true });
    }
    // `capabilities 3d web [authoritative]`: one verdict.
    let words: Vec<String> =
        query.iter().flat_map(|q| q.split(&[' ', ','][..]).map(|s| s.to_lowercase()).collect::<Vec<_>>()).filter(|s| !s.is_empty()).collect();
    let pres = words.iter().find_map(|w| caps::Presentation::parse(w));
    let plat = words.iter().find_map(|w| caps::Platform::parse(w));
    let net = words.iter().find_map(|w| caps::Networking::parse(w)).unwrap_or(caps::Networking::Offline);
    let (Some(pres), Some(plat)) = (pres, plat) else {
        return Err(format!(
            "say a presentation ({}) and a platform ({}), like `capabilities 2d web` (optionally a networking mode: {})",
            caps::Presentation::names().join(", "),
            caps::Platform::names().join(", "),
            caps::Networking::names().join(", ")
        ));
    };
    let c = caps::Capabilities { presentation: pres, platforms: vec![plat], networking: net, input: vec![], persistence: vec![], distribution: vec![] };
    let problems = caps::check(&c);
    let s = caps::support(pres, plat);
    let mut t = format!("{} on {}: {}{}\n", pres.name(), plat.name(), s.label(), if s.note().is_empty() { String::new() } else { format!(" — {}", s.note()) });
    let n = caps::networking_support(pres, plat, net);
    t.push_str(&format!("{} networking: {}{}", net.name(), n.label(), if n.note().is_empty() { String::new() } else { format!(" — {}", n.note()) }));
    Ok(Report { text: t, ok: problems.is_empty() })
}
