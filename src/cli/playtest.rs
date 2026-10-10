//! `red_engine2 playtest`: the game as a player sees it, for somebody who cannot look at a screen (ADR 2026-09-28-seeing-what-the-player-sees).
//!
//! The work is done by the client itself (`re2 --playtest`: the real loop, a script, offscreen pictures, a JSON dump); this command finds that binary, runs it, and turns
//! its report into a verdict a person or an AI can read at once.

use super::*;
use red_engine2::tools::game::sibling_exe;

/// Everything `playtest` passes on to the client.
pub(crate) struct PlaytestArgs<'a> {
    pub scene: &'a Path,
    pub secs: Option<f32>,
    pub shots: Option<usize>,
    pub out: &'a Path,
    pub script: Option<&'a Path>,
    pub connect: Option<&'a str>,
    pub fill: Option<usize>,
    pub bot_skill: Option<&'a str>,
    pub size: Option<&'a str>,
}

/// The `re2` arguments for a playtest (the scene first).
pub(crate) fn client_args(a: &PlaytestArgs) -> Vec<String> {
    let mut args = vec![a.scene.display().to_string(), "--playtest".to_string(), "--out".to_string(), a.out.display().to_string()];
    let mut push = |flag: &str, value: Option<String>| {
        if let Some(v) = value {
            args.push(flag.to_string());
            args.push(v);
        }
    };
    push("--secs", a.secs.map(|s| s.to_string()));
    push("--shots", a.shots.map(|s| s.to_string()));
    push("--script", a.script.map(|p| p.display().to_string()));
    push("--connect", a.connect.map(str::to_string));
    push("--fill", a.fill.map(|f| f.to_string()));
    push("--bot-skill", a.bot_skill.map(str::to_string));
    push("--size", a.size.map(str::to_string));
    args
}

/// The verdict, in a few lines, from the client's `playtest.json`.
pub(crate) fn summarize(report: &Value) -> String {
    let num = |p: &str| report.pointer(p).and_then(Value::as_u64).unwrap_or(0);
    let failures: Vec<&str> = report["failures"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let mut s = format!("playtest {}: {}\n", report["map"].as_str().unwrap_or("?"), if failures.is_empty() { "OK" } else { "FAILED" });
    s.push_str(&format!(
        "  {:.0} s of game time, {} frames, {} picture(s){}\n",
        report["secs"].as_f64().unwrap_or(0.0),
        report["frame"].as_u64().unwrap_or(0),
        report["shots"].as_array().map_or(0, Vec::len),
        report["sheet"].as_str().map(|p| format!(", contact sheet {p}")).unwrap_or_default()
    ));
    if report["remote"].is_object() {
        s.push_str(&format!(
            "  other players: {} drawn of {} in view, {} undrawn, {} in another costume, {} hidden by interest management, {} not yet posed (roster: {})\n",
            num("/remote/drawn"),
            num("/remote/in_view"),
            num("/remote/undrawn"),
            num("/remote/standins"),
            num("/remote/hidden_by_interest"),
            num("/remote/unposed"),
            num("/remote/roster_others")
        ));
        let bodies: Vec<String> = report["remote"]["players"]
            .as_array()
            .map(|a| a.iter().map(|p| format!("{} as {}", p["id"], p["avatar"].as_str().unwrap_or("?"))).collect())
            .unwrap_or_default();
        if !bodies.is_empty() {
            s.push_str(&format!("  drawn: {}\n", bodies.join(", ")));
        }
    } else {
        s.push_str("  offline: no other players\n");
    }
    if let Some(counts) = report["cues"]["counts"].as_object().filter(|c| !c.is_empty()) {
        s.push_str(&format!("  heard: {}\n", counts.iter().map(|(k, v)| format!("{k} x{v}")).collect::<Vec<_>>().join(", ")));
    }
    s.push_str(&format!("  crosshair at the end: {}\n", report["crosshair"]["state"].as_str().unwrap_or("?")));
    for f in &failures {
        s.push_str(&format!("  FAILED: {f}\n"));
    }
    s
}

/// `playtest`: runs the client, prints its output, then the verdict.
pub(crate) fn run_playtest(a: &PlaytestArgs) -> Result<(), String> {
    let exe = sibling_exe("re2")
        .ok_or("cannot find the `re2` client next to this program: build both with `scripts/dev build` (or `cargo build --bins`; the client needs the default `gfx` feature)")?;
    std::fs::create_dir_all(a.out).map_err(|e| format!("{}: {e}", a.out.display()))?;
    let report_path = a.out.join("playtest.json");
    let _ = std::fs::remove_file(&report_path); // a stale report of an earlier run must not be read as this one's
    let output = std::process::Command::new(&exe).args(client_args(a)).output().map_err(|e| format!("{}: {e}", exe.display()))?;
    let (stdout, stderr) = (String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    let report: Option<Value> = std::fs::read_to_string(&report_path).ok().and_then(|t| serde_json::from_str(&t).ok());
    if envelope::capturing() {
        println!("{}", serde_json::to_string_pretty(&report.clone().unwrap_or(Value::Null)).unwrap_or_default());
    } else {
        for line in stdout.lines().filter(|l| l.starts_with("shot:") || l.starts_with("warning") || l.starts_with("script:")) {
            println!("{line}");
        }
        match &report {
            Some(r) => print!("{}", summarize(r)),
            None => {
                // The client did not get as far as writing its report: show why.
                print!("{stdout}");
                eprint!("{stderr}");
            }
        }
    }
    if report.is_none() {
        return Err(format!(
            "the client wrote no report ({}); run `{} --help` and check the map loads: `red_engine2 validate {}`",
            output.status,
            exe.display(),
            a.scene.display()
        ));
    }
    if output.status.success() {
        Ok(())
    } else {
        Err(String::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_client_gets_the_flags_that_were_given_and_only_those() {
        let a = PlaytestArgs {
            scene: Path::new("maps/arena.json"),
            secs: Some(45.0),
            shots: Some(8),
            out: Path::new("out/pt"),
            script: None,
            connect: None,
            fill: Some(6),
            bot_skill: Some("easy"),
            size: None,
        };
        assert_eq!(client_args(&a), ["maps/arena.json", "--playtest", "--out", "out/pt", "--secs", "45", "--shots", "8", "--fill", "6", "--bot-skill", "easy"]);
    }

    #[test]
    fn the_verdict_says_who_was_drawn_what_was_heard_and_what_failed() {
        let ok = json!({"map": "arena.json", "secs": 60.2, "frame": 3612, "shots": [1, 2, 3], "sheet": "out/pt/contact-sheet.png",
            "remote": {"drawn": 7, "in_view": 7, "undrawn": 0, "standins": 0, "hidden_by_interest": 0, "unposed": 0, "roster_others": 7,
                       "players": [{"id": 1, "avatar": "net_cowboy_0"}]},
            "cues": {"counts": {"Shot": 12, "Step": 40}}, "crosshair": {"state": "enemy"}, "failures": []});
        let s = summarize(&ok);
        assert!(s.starts_with("playtest arena.json: OK"), "{s}");
        assert!(
            s.contains("7 drawn of 7 in view, 0 undrawn")
                && s.contains("1 as net_cowboy_0")
                && s.contains("Shot x12")
                && s.contains("contact sheet out/pt/contact-sheet.png"),
            "{s}"
        );
        let bad = json!({"map": "m.json", "remote": {"drawn": 6, "in_view": 7, "undrawn": 1}, "failures": ["player 3 wears Robot and the avatar pool has nothing free for them"]});
        let s = summarize(&bad);
        assert!(s.contains("FAILED") && s.contains("1 undrawn") && s.contains("FAILED: player 3 wears Robot"), "{s}");
        assert!(summarize(&json!({"map": "solo.json", "failures": []})).contains("offline: no other players"));
    }
}
