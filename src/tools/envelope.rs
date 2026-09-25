//! The structured-output envelope: `red_engine2 --json <command> ...` wraps **any** command's result in one
//! stable JSON shape an AI (or the MCP adapter) can parse without guessing.
//!
//! ```json
//! { "schema": 1, "command": "validate", "ok": false, "exit": 1,
//!   "data": { ... } | { "text": "..." },
//!   "diagnostics": [ { "code": "unknown-field", "path": "b.pos", "message": "unknown field — did you mean `position`?", "fix": "position" } ],
//!   "stderr": "..." }
//! ```
//!
//! - `ok` is `exit == 0`. A command can fail *and* still have `data` (a failing `lint` lists its findings).
//! - `data` is what the command printed on stdout: parsed JSON when the command has a JSON form (`lint`, `reach`,
//!   `verify`, `ls`, `catalog`, `describe`, ...), else `{"text": ...}`. Nothing else is ever printed in this mode.
//! - `diagnostics` are the `path: message` problems the command reported, each with a stable [`code`](classify)
//!   (`red_engine2 describe diagnostics` lists them) and, when the message carries one, a `fix`.
//! - The MCP adapter and any other tool consume exactly this; there is no second implementation.
//!
//! Implementation: the CLI's `print!`/`println!`/`eprintln!` are routed through [`write_out`]/[`write_err`], which
//! append to a buffer while [`begin_capture`] is active and write to the terminal otherwise.

use serde_json::{json, Value};
use std::cell::RefCell;
use std::fmt::Arguments;
use std::io::Write;

/// Version of the envelope shape. Bumped only on an incompatible change.
pub const ENVELOPE_SCHEMA: u32 = 1;

thread_local! {
    static CAPTURE: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
}

/// Starts buffering stdout/stderr (this thread).
pub fn begin_capture() {
    CAPTURE.with(|c| *c.borrow_mut() = Some((String::new(), String::new())));
}

/// True while output is being buffered (commands use it to pick their JSON form).
pub fn capturing() -> bool {
    CAPTURE.with(|c| c.borrow().is_some())
}

/// Stops buffering; returns `(stdout, stderr)`.
pub fn end_capture() -> (String, String) {
    CAPTURE.with(|c| c.borrow_mut().take()).unwrap_or_default()
}

/// The target of the CLI's `print!`: the capture buffer, or stdout.
pub fn write_out(args: Arguments) {
    let done = CAPTURE.with(|c| match c.borrow_mut().as_mut() {
        Some((out, _)) => {
            let _ = std::fmt::write(out, args);
            true
        }
        None => false,
    });
    if !done {
        let _ = std::io::stdout().write_fmt(args);
    }
}

/// The target of the CLI's `eprint!`: the capture buffer, or stderr.
pub fn write_err(args: Arguments) {
    let done = CAPTURE.with(|c| match c.borrow_mut().as_mut() {
        Some((_, err)) => {
            let _ = std::fmt::write(err, args);
            true
        }
        None => false,
    });
    if !done {
        let _ = std::io::stderr().write_fmt(args);
    }
}

/// The stable diagnostic codes, with what each means and the usual fix. `describe diagnostics` prints this table;
/// [`classify`] must return only these (a test checks it).
pub const CODES: &[(&str, &str, &str)] = &[
    ("json-syntax", "the file is not valid JSON", "fix the syntax at the reported line/column"),
    ("io", "a file could not be read or written", "check the path"),
    ("unknown-field", "a key the engine does not know (a typo, or a property in the wrong place)", "use the suggested key, or prefix a note with `x-`"),
    ("wrong-type", "a value has the wrong JSON type or shape", "match the type shown in the message"),
    ("missing-field", "a required key is absent", "add the key named in the message"),
    ("unknown-type", "an object `type` the engine does not have", "use one of the listed types (`describe objects`)"),
    ("unknown-name", "an unknown prefab, prop, zone, id or parameter", "use a listed name (`catalog`, `props`, `ls`)"),
    ("duplicate-id", "two objects share an id", "rename one"),
    ("out-of-range", "a number outside its allowed range", "use a value inside the range in the message"),
    ("invalid", "any other problem with the input", "read the message"),
];

/// Assigns a stable code to one problem message (heuristic on its wording; every code is in [`CODES`]).
pub fn classify(message: &str) -> &'static str {
    let m = message.to_lowercase();
    if m.starts_with("json:") || m.contains("expected value at line") || m.contains("not valid json") {
        "json-syntax"
    } else if m.starts_with("io:") || m.contains("os error") || m.contains("no such file") {
        "io"
    } else if m.contains("unknown field") {
        "unknown-field"
    } else if m.contains("must be a number")
        || m.contains("must be an array")
        || m.contains("must be an object")
        || m.contains("must be a string")
        || m.contains("must be a whole number")
        || m.contains("must be [")
        || m.contains("must look like")
    {
        "wrong-type"
    } else if m.contains(": missing") || m.contains("missing (") || m.contains(" needs ") {
        "missing-field"
    } else if m.contains("unknown type") || m.contains("unknown light type") {
        "unknown-type"
    } else if m.contains("unknown prefab")
        || m.contains("unknown prop")
        || m.contains("no such param")
        || m.contains("unknown param")
        || m.contains("no zone")
        || m.contains("no object")
        || m.contains("unknown name")
    {
        "unknown-name"
    } else if m.contains("duplicate") {
        "duplicate-id"
    } else if m.contains("must be >") || m.contains("out of range") || m.contains("at most") || m.contains("too long") {
        "out-of-range"
    } else {
        "invalid"
    }
}

/// Splits problem text into one diagnostic per `path: message` line (other lines are folded into the previous
/// message). `fix` is filled from a trailing "did you mean `x`" when present.
pub fn diagnostics(text: &str) -> Vec<Value> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let line = line.strip_prefix("error: ").unwrap_or(line);
        if is_summary(line) {
            continue;
        }
        match split_path(line) {
            Some((path, msg)) => out.push((path.to_string(), msg.to_string())),
            None => match out.last_mut() {
                Some((_, m)) => {
                    m.push(' ');
                    m.push_str(line);
                }
                None => out.push((String::new(), line.to_string())),
            },
        }
    }
    out.into_iter()
        .map(|(path, message)| {
            let mut d = json!({"code": classify(&format!("{path}: {message}")), "path": path, "message": message});
            if let Some(fix) = fix_of(&message) {
                d["fix"] = json!(fix);
            }
            d
        })
        .collect()
}

/// A count line such as `3 error(s)`: not a problem itself.
fn is_summary(line: &str) -> bool {
    let mut w = line.split_whitespace();
    matches!((w.next(), w.next(), w.next()), (Some(n), Some(what), None) if n.chars().all(|c| c.is_ascii_digit()) && what.starts_with("error"))
}

/// `crate_1.pos: unknown field ...` →`("crate_1.pos", "unknown field ...")`. A line whose head contains spaces or is
/// too long is prose, not a path.
fn split_path(line: &str) -> Option<(&str, &str)> {
    let (head, tail) = line.split_once(": ")?;
    let path_like = !head.is_empty() && head.len() <= 80 && head.chars().all(|c| c.is_alphanumeric() || "_.-[]()/ ".contains(c)) && !head.starts_with(' ');
    let single_word = !head.trim().contains(' ') || head.contains('[') || head.contains('(');
    (path_like && single_word).then_some((head, tail))
}

/// The first backticked name after "did you mean".
fn fix_of(message: &str) -> Option<String> {
    let rest = &message[message.find("did you mean")? + "did you mean".len()..];
    let start = rest.find('`')? + 1;
    let end = start + rest[start..].find('`')?;
    Some(rest[start..end].to_string())
}

/// Builds the envelope for a finished command.
pub fn envelope(command: &str, exit: i32, stdout: &str, stderr: &str) -> Value {
    let data = match serde_json::from_str::<Value>(stdout.trim()) {
        Ok(v) if !stdout.trim().is_empty() => v,
        _ => json!({"text": stdout}),
    };
    let mut diags = Vec::new();
    if exit != 0 {
        diags.extend(diagnostics(stderr));
    }
    json!({
        "schema": ENVELOPE_SCHEMA,
        "command": command,
        "ok": exit == 0,
        "exit": exit,
        "data": data,
        "diagnostics": diags,
        "stderr": stderr.trim_end(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn problems_become_coded_diagnostics_with_fixes() {
        let d = diagnostics("b.pos: unknown field — did you mean `position`?\nlights[0].range: must be a number (got \"x\")\ncamera: missing");
        assert_eq!(d.len(), 3);
        assert_eq!(d[0]["code"], "unknown-field");
        assert_eq!(d[0]["path"], "b.pos");
        assert_eq!(d[0]["fix"], "position");
        assert_eq!(d[1]["code"], "wrong-type");
        assert_eq!(d[2]["code"], "missing-field");
    }

    #[test]
    fn every_code_classify_can_return_is_documented() {
        for msg in [
            "json: expected value at line 1",
            "io: nope",
            "a.b: unknown field — x",
            "a: must be an object",
            "a.id: missing (x)",
            "a.type: unknown type 'q'",
            "a.prefab: unknown prefab 'q'",
            "duplicate id 'x'",
            "lights: at most 16 lights are allowed",
            "something else entirely",
        ] {
            let c = classify(msg);
            assert!(CODES.iter().any(|(k, _, _)| *k == c), "{msg} -> {c} is not in CODES");
        }
    }

    #[test]
    fn stdout_that_is_json_becomes_data_and_text_is_wrapped() {
        let e = envelope("lint", 0, "{\"findings\":[]}\n", "");
        assert_eq!(e["ok"], true);
        assert_eq!(e["data"]["findings"], json!([]));
        let t = envelope("plan", 1, "hello\nworld\n", "x.json: unknown field — nope");
        assert_eq!(t["data"]["text"], "hello\nworld\n");
        assert_eq!(t["ok"], false);
        assert_eq!(t["diagnostics"][0]["code"], "unknown-field");
    }

    #[test]
    fn capture_buffers_output_and_stops() {
        assert!(!capturing());
        begin_capture();
        write_out(format_args!("a{}", 1));
        write_err(format_args!("e"));
        assert!(capturing());
        assert_eq!(end_capture(), ("a1".to_string(), "e".to_string()));
        assert!(!capturing());
    }
}
