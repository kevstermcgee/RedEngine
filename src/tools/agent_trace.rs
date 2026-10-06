//! `RED_TRACE=path`: one JSON line per command, so how a model *used* the engine can be read afterwards (docs: `scripts/agent_bench.py summary`).
//!
//! Set `RED_TRACE` to a file and every `red_engine2` invocation appends `{t, argv, command, exit, ms, error}`: which commands were run in what order, which failed and with what
//! first line of error, which `describe` topics and `search` questions were the documentation the model needed, how often it retried. It records nothing else (no output, no
//! file contents), never fails a command (an unwritable trace is silently skipped) and costs nothing when the variable is unset.

use serde_json::json;
use std::io::Write;
use std::sync::Mutex;

/// What a command that printed its own failure (and returned no error text) said first, kept for the trace line.
static DETAIL: Mutex<String> = Mutex::new(String::new());

/// A failing report's most telling line: the first `FAIL` row, else the first line. Call it where a command prints a failure and returns an empty error.
pub fn failure_detail(text: &str) {
    let line = text.lines().find(|l| l.starts_with("FAIL") || l.contains("error")).or_else(|| text.lines().next()).unwrap_or("");
    if let Ok(mut d) = DETAIL.lock() {
        *d = line.to_string();
    }
}

/// Appends one line to the file named by `RED_TRACE`, if set.
pub fn record(command: &str, code: i32, ms: u128, error: &str) {
    let Ok(path) = std::env::var("RED_TRACE") else { return };
    if path.is_empty() {
        return;
    }
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    let detail = DETAIL.lock().map(|d| d.clone()).unwrap_or_default();
    let first_line: String = if error.is_empty() { detail.as_str() } else { error }.lines().next().unwrap_or("").chars().take(300).collect();
    let line = json!({"t": (t * 1000.0).round() / 1000.0, "argv": argv, "command": command, "exit": code, "ms": ms as u64, "error": first_line});
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}
