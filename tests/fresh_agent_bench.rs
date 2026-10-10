//! The fresh-agent benchmark (bench/fresh-agent/): the front door is executable by something that knows only what the engine prints, the scorer notices what it should, and the
//! command trace records how the engine was used. Everything here runs anywhere.

use std::path::{Path, PathBuf};
use std::process::Command;

fn engine() -> &'static str {
    env!("CARGO_BIN_EXE_red_engine2")
}

fn script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/agent_bench.py")
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("re2_bench_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn py(args: &[&str]) -> (bool, String) {
    let o = Command::new("python3").arg(script()).args(args).output().expect("python3");
    (o.status.success(), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
}

#[test]
fn the_reference_agent_passes_the_task_using_only_what_describe_2d_says_with_no_friction() {
    let work = scratch("reference");
    let (ok, out) = py(&["reference", work.to_str().unwrap(), "--engine", engine()]);
    assert!(ok, "{out}");
    assert!(
        out.contains("\"failed\": 0")
            && out.contains("\"cli_source_exploration\": 0")
            && out.contains("\"retries_of_a_failed_command_without_a_success_between\": 0"),
        "{out}"
    );
    assert!(out.contains("\"2d\"") && out.contains("\"used_verify\": true"), "the agent read the one page the brief points to, and verified the game: {out}");
    // The scorer's own evidence: the game is in the folder, and its checkpoint is the 25 second version.
    let before: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(work.join("star-dash/before.game2d.json")).unwrap()).unwrap();
    let after: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(work.join("star-dash/star-dash.game2d.json")).unwrap()).unwrap();
    assert_eq!((before["vars"]["timeleft"].clone(), after["vars"]["timeleft"].clone()), (serde_json::json!(25), serde_json::json!(15)));
}

#[test]
fn the_scorer_fails_what_the_task_asked_for_and_the_game_lacks() {
    let work = scratch("negative");
    let (ok, out) = py(&["reference", work.to_str().unwrap(), "--engine", engine()]);
    assert!(ok, "{out}");
    let game = work.join("star-dash/star-dash.game2d.json");
    let mut g: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&game).unwrap()).unwrap();
    // No losing playthrough, no saved value, the old countdown.
    g["checks"]["scenarios"].as_array_mut().unwrap().retain(|s| s["expect"].to_string().contains("win"));
    g["persist"] = serde_json::json!([]);
    g["vars"]["timeleft"] = serde_json::json!(25);
    std::fs::write(&game, g.to_string()).unwrap();
    let (ok, out) = py(&["score", work.join("star-dash").to_str().unwrap(), "--engine", engine()]);
    assert!(!ok, "{out}");
    for must in
        ["FAIL    a scripted playthrough loses", "FAIL    a value is saved between runs", "FAIL    the requested change is made: the countdown starts at 15"]
    {
        assert!(out.contains(must), "the scorecard must contain `{must}`:\n{out}");
    }
}

#[test]
fn every_command_can_leave_one_line_in_a_trace_and_the_summary_reads_the_friction() {
    let dir = scratch("trace");
    let trace = dir.join("trace.jsonl");
    let run = |args: &[&str]| Command::new(engine()).args(args).env("RED_TRACE", &trace).current_dir(&dir).output().unwrap();
    run(&["describe", "--brief"]);
    run(&["describe", "2d"]);
    run(&["search", "how do I save progress"]);
    let broken = dir.join("broken.game2d.json");
    std::fs::write(&broken, r#"{"game2d":1,"id":"broken"}"#).unwrap();
    assert!(!run(&["validate", broken.to_str().unwrap()]).status.success());
    assert!(!run(&["validate", broken.to_str().unwrap()]).status.success(), "the same failing command again");
    std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/2d/coin-dash.game2d.json"), &broken).unwrap();
    assert!(run(&["validate", broken.to_str().unwrap()]).status.success());
    let lines: Vec<serde_json::Value> = std::fs::read_to_string(&trace).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), 6);
    assert_eq!(lines[3]["exit"], 1);
    assert!(lines[3]["error"].as_str().unwrap().len() > 3 && lines[0]["argv"][0] == "describe", "{lines:?}");
    // A transcript with a model reading engine source with its own tools, and one game edit.
    let transcript = dir.join("session.jsonl");
    std::fs::write(
        &transcript,
        [
            r#"{"message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/x/RedEngine/src/sim/match_sim.rs"}}]}}"#,
            r#"{"message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"grep -rn gate /x/RedEngine/crates/red2d/src"}}]}}"#,
            r#"{"message":{"content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/work/star-dash/star-dash.game2d.json"}}]}}"#,
            "not json at all",
        ]
        .join("\n"),
    )
    .unwrap();
    let (ok, out) = py(&["summary", trace.to_str().unwrap(), "--transcript", transcript.to_str().unwrap()]);
    assert!(ok, "{out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["commands"], 6);
    assert_eq!(v["failed"], 2);
    assert_eq!(v["retries_of_a_failed_command_without_a_success_between"], 1);
    assert_eq!(v["repair_cycles"], 1);
    assert_eq!(v["documentation_topics_read"], serde_json::json!(["overview", "2d"]));
    assert_eq!(v["searches"][0], "how do I save progress");
    assert_eq!(v["transcript"]["engine_files_read_with_tools"], 1);
    assert_eq!(v["transcript"]["engine_files_read_with_the_shell"], 1);
    assert_eq!(v["transcript"]["edits"], 1);
}

#[test]
fn the_starter_survives_the_edits_the_benchmark_makes_to_it() {
    // Adding a gem, changing the clock and adding a hazard must not break a check written for the old numbers: the starter's expectations say what must hold, not how many there were.
    let dir = scratch("starter");
    assert!(Command::new(engine()).args(["new-game"]).arg(dir.join("g")).args(["--kind", "2d", "--name", "g"]).status().unwrap().success());
    let game = dir.join("g/g.game2d.json");
    let mut v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&game).unwrap()).unwrap();
    v["scene"].as_array_mut().unwrap().push(serde_json::json!({"prefab": "gem", "at": [100, 100]}));
    v["vars"]["gems_left"] = serde_json::json!(6);
    v["vars"]["timeleft"] = serde_json::json!(25);
    std::fs::write(&game, v.to_string()).unwrap();
    let o = Command::new(engine()).args(["verify"]).arg(&game).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
}
