//! `red_engine2 web verify | setup-browser`: run a web package in a real headless browser.
//!
//! The driver is `crates/red2d/web/browser_verify.py` (Playwright for Python, Chromium). It is embedded in this binary and written to a temp file, so an installed
//! `red_engine2` needs only the browser, not a source checkout. `setup-browser` creates the virtual environment once per machine; `verify` never installs anything.
//! No credentials and no internet are needed to verify a local package (the browser talks to a server on 127.0.0.1).

use red2d::script::Row;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

const SCRIPT: &str = include_str!("../../crates/red2d/web/browser_verify.py");

/// Where `setup-browser` puts the Python environment.
pub fn browser_home() -> PathBuf {
    if let Ok(p) = std::env::var("RED2D_BROWSER_HOME") {
        return PathBuf::from(p);
    }
    let base = std::env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|h| Path::new(&h).join(".cache")))
        .or_else(|_| std::env::var("LOCALAPPDATA").map(PathBuf::from))
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join("red_engine2/browser")
}

fn venv_python(home: &Path) -> PathBuf {
    if cfg!(windows) {
        home.join("Scripts/python.exe")
    } else {
        home.join("bin/python")
    }
}

fn has_playwright(py: &Path) -> bool {
    Command::new(py).args(["-c", "import playwright"]).output().map(|o| o.status.success()).unwrap_or(false)
}

/// A Python that can import Playwright: `RED2D_BROWSER_PYTHON`, the `setup-browser` environment, or the system one.
pub fn browser_python() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("RED2D_BROWSER_PYTHON") {
        let p = PathBuf::from(p);
        return if has_playwright(&p) { Ok(p) } else { Err(format!("RED2D_BROWSER_PYTHON={} cannot `import playwright`", p.display())) };
    }
    let v = venv_python(&browser_home());
    if v.is_file() && has_playwright(&v) {
        return Ok(v);
    }
    for name in ["python3", "python"] {
        let p = PathBuf::from(name);
        if has_playwright(&p) {
            return Ok(p);
        }
    }
    Err("no headless browser is set up for `web verify` (a web package can still be built, checked and served without one). Fix: `red_engine2 web setup-browser` (needs Python 3 and internet once, installs under ~/.cache/red_engine2/browser)".into())
}

/// Creates the Python environment and downloads Chromium. Needs internet; run once per machine.
pub fn setup_browser() -> Result<String, String> {
    let home = browser_home();
    let mut log = String::new();
    let run = |cmd: &mut Command, what: &str, log: &mut String| -> Result<(), String> {
        let o = cmd.output().map_err(|e| format!("{what}: {e}"))?;
        log.push_str(&format!("$ {what}: {}\n", if o.status.success() { "ok" } else { "FAILED" }));
        if o.status.success() {
            Ok(())
        } else {
            Err(format!(
                "{log}{}\n{}",
                String::from_utf8_lossy(&o.stdout).lines().rev().take(8).collect::<Vec<_>>().join("\n"),
                String::from_utf8_lossy(&o.stderr).lines().rev().take(12).collect::<Vec<_>>().join("\n")
            ))
        }
    };
    std::fs::create_dir_all(&home).map_err(|e| format!("{}: {e}", home.display()))?;
    if !venv_python(&home).is_file() {
        run(Command::new("python3").args(["-m", "venv"]).arg(&home), "python3 -m venv", &mut log)
            .map_err(|e| format!("{e}\nfix: install Python 3 with venv (Debian/Ubuntu: `sudo apt install python3-venv`)"))?;
    }
    let py = venv_python(&home);
    run(Command::new(&py).args(["-m", "pip", "install", "--quiet", "playwright"]), "pip install playwright", &mut log)?;
    run(Command::new(&py).args(["-m", "playwright", "install", "chromium"]), "playwright install chromium", &mut log)?;
    Ok(format!("{log}browser ready under {}", home.display()))
}

/// What a browser run found.
pub struct Verified {
    /// The rows (claim `browser`, `browser-audio`).
    pub rows: Vec<Row>,
    /// The browser's name and version.
    pub browser: String,
    /// The full JSON report.
    pub raw: Value,
}

/// Runs the driver against a package directory or a URL; writes screenshots into `out`.
pub fn verify(dir: Option<&Path>, url: Option<&str>, out: &Path) -> Result<Verified, String> {
    let py = browser_python()?;
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let script = out.join("browser_verify.py");
    std::fs::write(&script, SCRIPT).map_err(|e| format!("{}: {e}", script.display()))?;
    let mut cmd = Command::new(py);
    cmd.arg(&script).arg("--out").arg(out);
    match (dir, url) {
        (Some(d), None) => cmd.arg("--dir").arg(d),
        (None, Some(u)) => cmd.arg("--url").arg(u),
        _ => return Err("give a package directory or a URL".into()),
    };
    let o = cmd.output().map_err(|e| format!("could not start the browser driver: {e}"))?;
    let stdout = String::from_utf8_lossy(&o.stdout);
    let last = stdout.lines().rev().find(|l| l.starts_with('{')).unwrap_or("");
    let v: Value = serde_json::from_str(last).map_err(|_| {
        format!(
            "the browser driver printed no report (exit {:?}):\n{}\n{}",
            o.status.code(),
            stdout.lines().rev().take(10).collect::<Vec<_>>().join("\n"),
            String::from_utf8_lossy(&o.stderr).lines().rev().take(15).collect::<Vec<_>>().join("\n")
        )
    })?;
    if let Some(e) = v.get("harness_error").and_then(|e| e.as_str()) {
        return Err(format!(
            "the browser driver failed: {e}{}",
            if e.contains("Executable doesn't exist") || e.contains("playwright install") { "\nfix: `red_engine2 web setup-browser`" } else { "" }
        ));
    }
    let rows = v["rows"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|r| Row {
                    ok: r["ok"].as_bool().unwrap_or(false),
                    claim: match r["claim"].as_str() {
                        Some("browser-audio") => "browser-audio",
                        _ => "browser",
                    },
                    name: r["name"].as_str().unwrap_or("").to_string(),
                    detail: r["detail"].as_str().unwrap_or("").to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Verified { rows, browser: v["browser"].as_str().unwrap_or("unknown").to_string(), raw: v })
}
