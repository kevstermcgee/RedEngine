//! The `--json` envelope, exercised through the real `red_engine2` binary: every command answers in one stable
//! shape, failures carry coded diagnostics with fixes, and the compact `describe --brief` stays small.

use serde_json::Value;
use std::process::Command;

fn run(args: &[&str]) -> (Value, i32) {
    let out =
        Command::new(env!("CARGO_BIN_EXE_red_engine2")).current_dir(env!("CARGO_MANIFEST_DIR")).arg("--json").args(args).output().expect("run red_engine2");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let v: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("stdout is not one JSON document ({e}):\n{text}"));
    (v, out.status.code().unwrap_or(-1))
}

fn temp_scene(name: &str, json: &str) -> String {
    let dir = std::env::temp_dir().join("re2_cli_envelope");
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(name);
    std::fs::write(&p, json).unwrap();
    p.to_string_lossy().to_string()
}

#[test]
fn every_envelope_has_the_stable_shape() {
    for args in [vec!["validate", "examples/test_lab.json"], vec!["lint", "examples/test_lab.json"], vec!["describe", "--brief"], vec!["props"], vec!["recipe"]]
    {
        let (v, code) = run(&args);
        assert_eq!(v["schema"], 1, "{args:?}");
        assert_eq!(v["command"], args[0]);
        assert_eq!(v["ok"], code == 0, "{args:?}");
        assert_eq!(v["exit"], code);
        assert!(v.get("data").is_some() && v["diagnostics"].is_array() && v.get("stderr").is_some(), "{args:?}: {v}");
    }
}

#[test]
fn a_bad_scene_yields_coded_diagnostics_with_paths_and_fixes() {
    let scene = temp_scene(
        "bad.json",
        r##"{"camera":{"position":[0,2,8],"target":[0,0,0]},"objects":[
            {"id":"b","type":"box","pos":[0,3,0]},
            {"id":"s","type":"sphere","color":"#f00","radius":"big"},
            {"id":"p","type":"prop","prop":"crat"}]}"##,
    );
    let (v, code) = run(&["validate", &scene]);
    assert_eq!(code, 1);
    assert_eq!(v["ok"], false);
    let d = v["diagnostics"].as_array().unwrap();
    let find = |path: &str| d.iter().find(|x| x["path"] == path).unwrap_or_else(|| panic!("no diagnostic for {path}: {d:?}"));
    assert_eq!(find("b.pos")["code"], "unknown-field");
    assert_eq!(find("b.pos")["fix"], "position");
    assert_eq!(find("s.color")["code"], "unknown-field");
    assert_eq!(find("s.radius")["code"], "wrong-type");
    assert_eq!(find("p.prop")["code"], "unknown-name");
}

#[test]
fn a_failing_lint_still_returns_its_findings_as_data() {
    let scene = temp_scene(
        "floating.json",
        r##"{"camera":{"position":[0,1.7,0],"target":[0,1.7,-5]},"objects":[
            {"id":"floor","type":"plane","size":[20,20],"position":[0,0.01,0]},
            {"id":"crate_high","type":"prop","prop":"crate","position":[3,2.5,3]}]}"##,
    );
    let (v, _) = run(&["lint", &scene]);
    let findings = v["data"].as_array().expect("lint data is the findings array");
    assert!(findings.iter().any(|f| f["code"] == "floating"), "{findings:?}");
}

#[test]
fn describe_brief_is_small_and_lists_every_command_and_binary() {
    let (v, _) = run(&["describe", "--brief"]);
    let size = serde_json::to_string(&v["data"]).unwrap().len();
    assert!(size < 2500, "the first read must stay cheap: {size} bytes");
    let commands: Vec<&str> = v["data"]["commands"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    for c in ["validate", "lint", "reach", "walk", "verify", "search", "catalog", "describe"] {
        assert!(commands.contains(&c), "missing {c}");
    }
    let bins: Vec<&str> = v["data"]["binaries"].as_array().unwrap().iter().filter_map(|b| b["name"].as_str()).collect();
    assert_eq!(bins, ["red_engine2", "re2", "red_server", "red_bot"]);
}

#[test]
fn walk_and_search_have_native_json() {
    let (v, code) = run(&["walk", "examples/test_lab.json", "--path", "-22,-3; -18,-3"]);
    assert_eq!(code, 0);
    assert_eq!(v["data"]["ok"], true);
    assert_eq!(v["data"]["legs"].as_array().unwrap().len(), 2);
    let (v, _) = run(&["search", "how", "do", "stairs", "connect", "floors", "--limit", "3"]);
    assert!(!v["data"]["hits"].as_array().unwrap().is_empty());
    assert!(v["data"]["hits"][0]["loc"].is_string());
}

#[test]
fn the_diagnostics_topic_documents_every_code() {
    let (v, _) = run(&["describe", "diagnostics"]);
    let codes: Vec<&str> = v["data"]["codes"].as_array().unwrap().iter().filter_map(|c| c["code"].as_str()).collect();
    for c in ["unknown-field", "wrong-type", "missing-field", "unknown-name", "json-syntax"] {
        assert!(codes.contains(&c), "{c} is not documented");
    }
}
