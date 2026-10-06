//! What a browser game's publication record claims, one piece at a time.
//!
//! "It passed" is several different statements. This module keeps them apart: every piece of evidence has its own status (`passed`, `failed`, `not_run`, `not_applicable`) and its own
//! sentence saying what was seen, and a record never lets one success stand for another. `passed` on `wasm_instantiated` says nothing about `offline_reload`, and a game without saves
//! is `not_applicable` for persistence because it said so, never because nobody looked.
//!
//! The same object is written to `verification.json` (what `web verify` saw), to the game's `game.json` / `catalog.json` entry (what a library shows) and to `publication.json` (what
//! one `publish` run did, with the one thing a record cannot contain about itself: whether the deployed copy was reached).

use super::webverify::Verified;
use red2d::script::Row;
use serde_json::{json, Map, Value};

/// Schema of the evidence object.
pub const SCHEMA: &str = "red2d-evidence/1";

/// Every piece of evidence, in report order, with what a `passed` proves. (Nothing here proves a human played, heard or enjoyed the game.)
pub const KEYS: &[(&str, &str)] = &[
    ("native_scenarios", "the game's own scenarios pass in the native, headless simulation"),
    ("wasm_compiled", "the WebAssembly player was built (or supplied) and packaged with the game, and exports the player ABI"),
    (
        "browser_package_valid",
        "the static package is internally consistent: every file and hash, self-contained, no build-machine paths, installable web app files",
    ),
    ("wasm_instantiated", "a real browser instantiated game.wasm and the game initialised (the start screen appeared)"),
    (
        "loading_robustness",
        "input, focus and visibility changes while the game is still loading do nothing wrong, and a failed load shows a useful message with no live controls",
    ),
    ("playable_state", "the game reached a running state after the player started it, and time advanced"),
    ("input_keyboard", "real key events changed the game's state"),
    ("input_pointer", "a real mouse click changed the game's state"),
    ("input_touch", "emulated touches on the on-screen controller changed the game's state (an emulated phone, not a physical one)"),
    ("input_gamepad", "a (simulated) gamepad press started the game"),
    ("persistence_write", "after play, the browser's storage held the game's save"),
    ("persistence_reload", "after a reload, the saved progress came back into the game"),
    ("audio_api", "the Web Audio context started inside the player's first gesture (nobody listened)"),
    ("audio_playback", "a sound buffer and the music loop were handed to Web Audio and started (nobody listened)"),
    ("offline_cache", "a service worker stored the whole game"),
    ("offline_reload", "the game reloaded and ran with the network switched off"),
    ("installable", "the browser reported the page as installable"),
    ("browser_scenarios", "the game's own scenarios replayed in the browser's WebAssembly to the native state hashes"),
    ("remote_deployment", "a real browser played the deployed copy at a non-loopback URL"),
];

/// How one piece of evidence stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Checked, and held.
    Passed,
    /// Checked, and did not hold.
    Failed,
    /// Nobody checked (the stage did not run, or the run had no check for it).
    NotRun,
    /// The game never claimed the feature, so there is nothing to prove.
    NotApplicable,
}

impl Status {
    /// The word used in JSON and reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Passed => "passed",
            Status::Failed => "failed",
            Status::NotRun => "not_run",
            Status::NotApplicable => "not_applicable",
        }
    }
    fn parse(s: &str) -> Status {
        match s {
            "passed" => Status::Passed,
            "failed" => Status::Failed,
            "not_applicable" => Status::NotApplicable,
            _ => Status::NotRun,
        }
    }
}

/// One claim: its status and the sentence that says what was seen.
#[derive(Debug, Clone, PartialEq)]
pub struct Claim {
    /// Where it stands.
    pub status: Status,
    /// What was seen, in a sentence.
    pub detail: String,
}

/// All the evidence of one build, in the order of [`KEYS`]. A new value has every piece `not_run`.
#[derive(Debug, Clone, PartialEq)]
pub struct Evidence(pub Vec<(String, Claim)>);

impl Default for Evidence {
    fn default() -> Self {
        Evidence(KEYS.iter().map(|(k, _)| (k.to_string(), Claim { status: Status::NotRun, detail: "not run".into() })).collect())
    }
}

impl Evidence {
    /// Sets one piece. Unknown keys are a programming error.
    pub fn set(&mut self, key: &str, status: Status, detail: impl Into<String>) {
        let detail = detail.into();
        match self.0.iter_mut().find(|(k, _)| k == key) {
            Some((_, c)) => *c = Claim { status, detail },
            None => panic!("`{key}` is not a piece of evidence (see tools::evidence::KEYS)"),
        }
    }

    /// One piece.
    pub fn get(&self, key: &str) -> Option<&Claim> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, c)| c)
    }

    /// Whether a piece is `passed`.
    pub fn passed(&self, key: &str) -> bool {
        self.get(key).is_some_and(|c| c.status == Status::Passed)
    }

    /// Every piece that was applicable and run passed, and none failed (nothing says every piece ran: see [`Self::count`]).
    pub fn any_failed(&self) -> bool {
        self.0.iter().any(|(_, c)| c.status == Status::Failed)
    }

    /// `(passed, failed, not_run, not_applicable)`.
    pub fn count(&self) -> (usize, usize, usize, usize) {
        let n = |s: Status| self.0.iter().filter(|(_, c)| c.status == s).count();
        (n(Status::Passed), n(Status::Failed), n(Status::NotRun), n(Status::NotApplicable))
    }

    /// The native, headless half: the result of the game's own scenarios (`Ok(detail)` or `Err(failures)`).
    pub fn set_native(&mut self, result: Result<String, String>) {
        match result {
            Ok(d) => self.set("native_scenarios", Status::Passed, d),
            Err(e) => self.set("native_scenarios", Status::Failed, e),
        }
    }

    /// The package half: `rows` are `webpkg::check`'s. The module is "compiled" when the package has it and it passes the ABI rows.
    pub fn set_package(&mut self, rows: &[Row]) {
        let bad: Vec<String> = rows.iter().filter(|r| !r.ok).map(|r| format!("{}: {}", r.name, r.detail)).collect();
        if rows.is_empty() {
            self.set("browser_package_valid", Status::NotRun, "the package was not checked");
        } else if bad.is_empty() {
            self.set("browser_package_valid", Status::Passed, format!("{} package checks passed", rows.len()));
        } else {
            self.set("browser_package_valid", Status::Failed, bad.join("; "));
        }
        let abi: Vec<&Row> = rows.iter().filter(|r| r.name.starts_with("wasm ")).collect();
        match (abi.is_empty(), abi.iter().all(|r| r.ok)) {
            (true, _) => self.set("wasm_compiled", Status::NotRun, "the package check has no row about game.wasm"),
            (false, true) => self.set("wasm_compiled", Status::Passed, abi.iter().map(|r| r.detail.as_str()).collect::<Vec<_>>().join("; ")),
            (false, false) => self.set(
                "wasm_compiled",
                Status::Failed,
                abi.iter().filter(|r| !r.ok).map(|r| format!("{}: {}", r.name, r.detail)).collect::<Vec<_>>().join("; "),
            ),
        }
    }

    /// The browser half: fold the driver's tagged rows into one claim per key. A key with no tagged row is `not_applicable` if the game declared it so, else `not_run`.
    pub fn set_browser(&mut self, v: &Verified) {
        const BROWSER: &[&str] = &[
            "wasm_instantiated",
            "loading_robustness",
            "playable_state",
            "input_keyboard",
            "input_pointer",
            "input_touch",
            "input_gamepad",
            "persistence_write",
            "persistence_reload",
            "audio_api",
            "audio_playback",
            "offline_cache",
            "offline_reload",
            "installable",
            "browser_scenarios",
        ];
        for key in BROWSER {
            if let Some(why) = v.not_applicable.get(*key) {
                self.set(key, Status::NotApplicable, why.clone());
                continue;
            }
            let rows: Vec<&Row> = v.rows.iter().zip(&v.tags).filter(|(_, t)| t.as_deref() == Some(*key)).map(|(r, _)| r).collect();
            if rows.is_empty() {
                self.set(key, Status::NotRun, "the browser run had no check for this");
            } else if rows.iter().all(|r| r.ok) {
                self.set(key, Status::Passed, format!("{} check(s) passed in {}: {}", rows.len(), v.browser, rows[0].detail.lines().next().unwrap_or("")));
            } else {
                self.set(key, Status::Failed, rows.iter().filter(|r| !r.ok).map(|r| format!("{}: {}", r.name, r.detail)).collect::<Vec<_>>().join("; "));
            }
        }
    }

    /// `{key: {status, detail}}` plus the schema, for JSON files.
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        for (k, c) in &self.0 {
            m.insert(k.clone(), json!({"status": c.status.as_str(), "detail": c.detail}));
        }
        json!({"schema": SCHEMA, "pieces": Value::Object(m), "order": KEYS.iter().map(|(k, _)| *k).collect::<Vec<_>>(), "human_listening_verified": false, "human_playtest": false})
    }

    /// Reads one back; a record without evidence (an older build) is all `not_run`.
    pub fn from_json(v: &Value) -> Evidence {
        let mut e = Evidence::default();
        if let Some(p) = v["pieces"].as_object() {
            for (k, c) in p {
                if KEYS.iter().any(|(key, _)| key == k) {
                    e.set(k, Status::parse(c["status"].as_str().unwrap_or("")), c["detail"].as_str().unwrap_or(""));
                }
            }
        }
        e
    }

    /// One line per piece: `passed          wasm_instantiated  ...`, then the totals.
    pub fn render(&self) -> String {
        let mut t = String::new();
        for (k, c) in &self.0 {
            let detail = c.detail.lines().next().unwrap_or("");
            let detail = if detail.chars().count() > 110 { format!("{}...", detail.chars().take(107).collect::<String>()) } else { detail.to_string() };
            t.push_str(&format!("  {:<14} {:<22} {}\n", c.status.as_str(), k, detail));
        }
        let (p, f, n, na) = self.count();
        t.push_str(&format!("  = {p} passed, {f} failed, {n} not run, {na} not applicable. None of this says a human played, heard or enjoyed the game.\n"));
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(items: &[(bool, &str)]) -> Vec<Row> {
        items.iter().map(|(ok, name)| Row { ok: *ok, claim: "package", name: name.to_string(), detail: format!("detail of {name}") }).collect()
    }

    #[test]
    fn a_new_record_claims_nothing() {
        let e = Evidence::default();
        assert_eq!(e.count(), (0, 0, KEYS.len(), 0));
        assert!(KEYS.iter().all(|(k, _)| !e.passed(k)));
    }

    #[test]
    fn one_success_never_stands_for_another() {
        let mut e = Evidence::default();
        e.set("wasm_instantiated", Status::Passed, "ok");
        assert!(e.passed("wasm_instantiated"));
        for (k, _) in KEYS.iter().filter(|(k, _)| *k != "wasm_instantiated") {
            assert!(!e.passed(k), "`{k}` must not pass because another piece did");
        }
    }

    #[test]
    fn package_rows_decide_the_package_and_the_module_separately() {
        let mut e = Evidence::default();
        e.set_package(&rows(&[(true, "files and hashes"), (false, "wasm exports the player ABI")]));
        assert_eq!(e.get("browser_package_valid").unwrap().status, Status::Failed);
        assert_eq!(e.get("wasm_compiled").unwrap().status, Status::Failed);
        e.set_package(&rows(&[(true, "files and hashes"), (true, "wasm exports the player ABI")]));
        assert!(e.passed("browser_package_valid") && e.passed("wasm_compiled"));
        e.set_package(&rows(&[(true, "files and hashes")]));
        assert_eq!(e.get("wasm_compiled").unwrap().status, Status::NotRun, "no row about the module: not claimed");
    }

    #[test]
    fn evidence_survives_json_and_an_older_record_reads_as_not_run() {
        let mut e = Evidence::default();
        e.set("offline_reload", Status::Failed, "reload with no network failed");
        e.set("persistence_write", Status::NotApplicable, "the game declares no persistence");
        let back = Evidence::from_json(&e.to_json());
        assert_eq!(back, e);
        assert_eq!(Evidence::from_json(&Value::Null), Evidence::default());
        assert_eq!(e.to_json()["human_playtest"], false);
        let line = e.render().lines().find(|l| l.contains("offline_reload")).unwrap().to_string();
        assert!(line.trim_start().starts_with("failed") && e.render().contains("None of this says a human played"), "{line}");
    }
}
