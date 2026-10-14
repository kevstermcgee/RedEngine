//! The game-systems workflow as an agent runs it (the benchmark pattern of the engine's `tests/ai_tasks.rs`, here because Workshop's blocks
//! are read by Workshop's own program): discover -> inspect -> modify (a third machine through content, with one repair) -> verify -> inspect
//! the result, only through `workshop_server cli`, with every byte of tool output counted against a budget. A scripted agent, not a model:
//! it proves the steps exist, are bounded and answer, not that a fresh agent finds them (ADR 2026-10-14-game-systems-compose-under-one-match-authority).

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

const CLI: &str = env!("CARGO_BIN_EXE_workshop_server");

struct Agent {
    dir: PathBuf,
    context: usize,
    log: Vec<String>,
    errors: usize,
}

impl Agent {
    fn new() -> Agent {
        let dir = std::env::temp_dir().join(format!("workshop-agent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../content");
        for f in ["workshop.json", "defs/base.json"] {
            std::fs::create_dir_all(dir.join(f).parent().unwrap()).unwrap();
            std::fs::copy(src.join(f), dir.join(f)).unwrap();
        }
        Agent { dir, context: 0, log: Vec::new(), errors: 0 }
    }

    fn run(&mut self, args: &[&str]) -> (bool, String) {
        let out = Command::new(CLI).current_dir(&self.dir).arg("cli").args(args).output().expect("run workshop_server");
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        self.context += text.len();
        self.log.push(format!("$ workshop_server cli {}  ({} bytes)", args.join(" "), text.len()));
        if !out.status.success() {
            self.errors += 1;
        }
        (out.status.success(), text)
    }

    fn json(&mut self, args: &[&str]) -> Value {
        let mut full = vec!["--json"];
        full.extend_from_slice(args);
        let (_, text) = self.run(&full);
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{args:?}: not JSON ({e}): {text}"))
    }

    fn write(&mut self, name: &str, text: &str) {
        std::fs::write(self.dir.join(name), text).unwrap();
        self.log.push(format!("wrote {name} ({} bytes)", text.len()));
    }
}

/// The game's answers from a `sim --json --inspect` run: target -> answer.
fn answers(v: &Value) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    for s in v["data"]["scenarios"].as_array().into_iter().flatten() {
        for i in s["inspected"].as_array().into_iter().flatten() {
            out.push((i["target"].as_str().unwrap_or("").to_string(), i["answer"].clone()));
        }
    }
    out
}

#[test]
fn discover_inspect_add_a_machine_verify_and_look_at_it() {
    let mut a = Agent::new();
    let quick = "without the lever";
    // Discover: which systems run, in what order, from which files.
    let v = a.json(&["sim", "workshop.json", "--only", quick, "--inspect", "systems", "--inspect", "ws:furnace"]);
    let got = answers(&v);
    let systems = &got.iter().find(|(t, _)| t == "systems").unwrap().1;
    assert_eq!(systems["order"], serde_json::json!(["ws:supply", "ws:machines", "ws:shipping"]));
    // Inspect a definition: the behavior it uses, its parameters, the instances an edit to it would affect.
    let furnace = &got.iter().find(|(t, _)| t == "ws:furnace").unwrap().1;
    assert_eq!((furnace["behavior"].as_str(), furnace["used_by"][0].as_str()), (Some("ws:machines"), Some("ws:furnace_a")));
    // Modify: a second furnace in a file of its own, listed in the block. First with a typo; the validator names it and the fix.
    let line = r#"{"namespace": "lb", "instances": [{"id": "furnace_b", "machine": "ws:furnance", "from": "ws:storage", "to": "ws:storage"}]}"#;
    a.write("defs/line_b.json", line);
    let scene = std::fs::read_to_string(a.dir.join("workshop.json")).unwrap().replace(r#""defs/base.json""#, r#""defs/base.json", "defs/line_b.json""#);
    a.write("workshop.json", &scene);
    let (ok, why) = a.run(&["validate", "workshop.json"]);
    assert!(!ok && why.contains("defs/line_b.json#/instances/0/machine") && why.contains("did you mean `ws:furnace`?"), "{why}");
    a.write("defs/line_b.json", &line.replace("ws:furnance", "ws:furnace"));
    let (ok, why) = a.run(&["validate", "workshop.json"]);
    assert!(ok, "{why}");
    // Verify: the scene's own scenarios still pass with the extra machine.
    let (ok, report) = a.run(&["sim", "workshop.json"]);
    assert!(ok, "{report}");
    // Look at the result: the new machine works, from the file it was written in, on the shared behavior.
    let v = a.json(&["sim", "workshop.json", "--only", quick, "--inspect", "lb:furnace_b"]);
    let found = answers(&v);
    let b = &found.iter().find(|(t, _)| t == "lb:furnace_b").unwrap().1;
    assert_eq!(
        (b["system"].as_str(), b["definition"].as_str(), b["instance_at"]["file"].as_str()),
        (Some("ws:machines"), Some("ws:furnace"), Some("defs/line_b.json"))
    );
    assert!(b["state"]["made"].as_u64().unwrap() > 0, "{b}");
    println!("  read {} bytes in {} steps, {} failed command(s) (the planted typo)\n  {}", a.context, a.log.len(), a.errors, a.log.join("\n  "));
    assert_eq!(a.errors, 1, "one repair loop: the planted typo");
    assert!(a.context <= 40_000, "the task read {} bytes:\n  {}", a.context, a.log.join("\n  "));
    let _ = std::fs::remove_dir_all(&a.dir);
}
