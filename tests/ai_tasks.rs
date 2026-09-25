//! The AI-usability benchmark: canonical engine tasks an AI agent is asked to do, completed **only through the CLI** the way the
//! docs tell an agent to work (describe / search / catalog / recipe / add / validate / lint / sim / verify), with the context each
//! step costs added up. A task fails if a step fails, if the result is not actually verified by the engine's own checks, or if it
//! took more context than its budget — so unnecessary reading (or a missing capability that forces reading Rust) is caught as a
//! regression, not discovered by the next agent.
//!
//! The "agent" here is a script standing in for a well-behaved AI: it reads `describe --brief` first, asks `search` for the task,
//! copies snippets, and lets the tools check its work. Budgets are bytes of tool output read.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

const CLI: &str = env!("CARGO_BIN_EXE_red_engine2");

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Everything the agent has read so far, and what it did.
struct Agent {
    context: usize,
    log: Vec<String>,
    dir: PathBuf,
}

impl Agent {
    fn new(task: &str) -> Agent {
        let dir = std::env::temp_dir().join("re2_ai_tasks").join(task);
        std::fs::create_dir_all(&dir).unwrap();
        let mut a = Agent { context: 0, log: Vec::new(), dir };
        // Step zero of every task: the compact orientation.
        let brief = a.run(&["describe", "--brief"]).1;
        assert!(brief.len() < 2_500, "`describe --brief` must stay a cheap first read ({} bytes)", brief.len());
        a
    }

    /// Runs the CLI in text mode (what an agent reads); returns (success, stdout+stderr).
    fn run(&mut self, args: &[&str]) -> (bool, String) {
        let out = Command::new(CLI).current_dir(root()).args(args).output().expect("run red_engine2");
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        self.context += text.len();
        self.log.push(format!("$ red_engine2 {}  ({} bytes)", args.join(" "), text.len()));
        (out.status.success(), text)
    }

    /// Runs the CLI with `--json` (an agent parsing a result); counts what it reads as compact JSON.
    fn json(&mut self, args: &[&str]) -> Value {
        let mut full = vec!["--json"];
        full.extend_from_slice(args);
        let out = Command::new(CLI).current_dir(root()).args(&full).output().expect("run red_engine2");
        let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{args:?}: not JSON ({e})"));
        self.context += serde_json::to_string(&v).unwrap().len();
        self.log.push(format!("$ red_engine2 --json {}", args.join(" ")));
        v
    }

    fn file(&self, name: &str) -> String {
        self.dir.join(name).to_string_lossy().to_string()
    }

    fn write(&mut self, name: &str, v: &Value) -> String {
        let p = self.file(name);
        std::fs::write(&p, serde_json::to_string_pretty(v).unwrap()).unwrap();
        p
    }

    fn within(&self, budget: usize) {
        assert!(self.context <= budget, "task read {} bytes of tool output (budget {budget}):\n  {}", self.context, self.log.join("\n  "));
        println!("  used {} of {budget} bytes across {} step(s)", self.context, self.log.len());
    }
}

fn read_json(p: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

/// The two big docs an agent would otherwise read whole.
fn docs_bytes() -> usize {
    ["AGENTS.md", "SPEC.md"].iter().map(|f| std::fs::metadata(root().join(f)).unwrap().len() as usize).sum()
}

#[test]
fn the_first_read_is_a_small_fraction_of_the_docs() {
    let mut a = Agent::new("orientation");
    let overview = a.run(&["describe"]).1;
    assert!(overview.len() < 7_000, "`describe` overview is {} bytes", overview.len());
    assert!(a.context * 8 < docs_bytes(), "orientation costs {} bytes vs {} bytes of AGENTS.md + SPEC.md", a.context, docs_bytes());
    a.within(9_500);
}

#[test]
fn task_create_a_room_and_a_spawn_group() {
    let mut a = Agent::new("room_spawn");
    let hits = a.run(&["search", "start", "a", "new", "walkable", "room", "map", "--limit", "3"]).1;
    assert!(hits.contains("recipe"), "search points at a recipe to copy from:\n{hits}");
    let file = a.file("room.json");
    let _ = std::fs::remove_file(&file);
    assert!(a.run(&["recipe", "rooms_and_door", "--new", &file]).0);
    let mut scene = read_json(&file);
    scene["spawns"] = json!([{"id": "s1", "position": [-2.0, 0.0, 0.0], "yaw_deg": 90, "group": "arena"}, {"id": "s2", "position": [2.0, 0.0, 0.0], "yaw_deg": 270, "group": "arena"}]);
    a.write("room.json", &scene);
    let (ok, out) = a.run(&["validate", &file]);
    assert!(ok, "{out}");
    let (ok, out) = a.run(&["lint", &file]);
    assert!(ok, "the recipe plus two spawns lints clean: {out}");
    let text = std::fs::read_to_string(&file).unwrap();
    let spawns = red_engine2::sim::spawns::parse_spawns(&text).unwrap();
    assert_eq!(spawns.iter().filter(|s| s.group == "arena").count(), 2, "the spawn group exists as data");
    a.within(9_000);
}

#[test]
fn task_create_a_reusable_prefab() {
    let mut a = Agent::new("prefab");
    let hits = a.run(&["search", "define", "my", "own", "prefab", "reusable", "asset", "--limit", "3"]).1;
    assert!(hits.contains("prefab"), "{hits}");
    let scene = json!({
        "camera": {"position": [0, 2, 8], "target": [0, 1, 0]},
        "prefabs": {"lamp_post": {"desc": "A street lamp", "tags": ["light", "street"], "params": {"h": 3.0},
            "objects": [{"id": "pole", "type": "cylinder", "radius": 0.06, "height": "$h", "position": [0, "=$h/2", 0], "material": {"color": "#333333"}},
                        {"id": "bulb", "type": "sphere", "radius": 0.15, "position": [0, "$h", 0], "material": {"color": "#ffffff", "emissive": "#ffdd88"}}]}},
        "objects": [
            {"id": "floor", "type": "plane", "size": [20, 20], "position": [0, 0.01, 0]},
            {"id": "lamp_1", "type": "prefab", "prefab": "lamp_post", "position": [-2, 0, 0]},
            {"id": "lamp_2", "type": "prefab", "prefab": "lamp_post", "position": [2, 0, 0], "params": {"h": 4.5}}
        ]
    });
    let file = a.write("prefab.json", &scene);
    let (ok, out) = a.run(&["validate", &file]);
    assert!(ok, "{out}");
    let listed = a.json(&["ls", &file]);
    let ids: Vec<&str> = listed["data"].as_array().unwrap().iter().filter_map(|o| o["id"].as_str()).collect();
    assert!(ids.contains(&"lamp_1") && ids.contains(&"lamp_2"), "{ids:?}");
    // A mistake in an instance is explained, not ignored.
    let mut bad = scene.clone();
    bad["objects"][2]["params"] = json!({"height": 4.5});
    let bad_file = a.write("prefab_bad.json", &bad);
    let (ok, out) = a.run(&["validate", &bad_file]);
    assert!(!ok && out.contains("lamp_post") && out.contains("no such param"), "{out}");
    a.within(7_000);
}

#[test]
fn task_add_a_dynamic_prop_by_copying_the_catalog_snippet() {
    let mut a = Agent::new("dynamic_prop");
    let file = a.file("props.json");
    let _ = std::fs::remove_file(&file);
    assert!(a.run(&["recipe", "rooms_and_door", "--new", &file]).0);
    let entry = a.run(&["catalog", "barrel"]).1;
    // The catalogue prints a paste-ready command: `red_engine2 add <scene> '<json>'`.
    let json_text = entry
        .lines()
        .find_map(|l| l.split_once("add <scene> '").map(|(_, r)| r.trim_end().trim_end_matches('\'').to_string()))
        .expect("a snippet in the catalogue entry");
    let mut obj: Value = serde_json::from_str(&json_text).unwrap();
    obj["position"] = json!([-2.0, 0.0, 1.0]);
    let (ok, out) = a.run(&["add", &file, &obj.to_string()]);
    assert!(ok, "{out}");
    let (ok, out) = a.run(&["lint", &file]);
    assert!(ok, "a barrel on the floor is fine: {out}");
    // ... and it really is a physics prop the server will simulate as loose.
    let scene = red_engine2::load_scene(Path::new(&file)).unwrap();
    let world = red_engine2::physics::PropWorld::new(&scene, None);
    assert!(
        scene.objects.iter().position(|o| o.id == obj["id"].as_str().unwrap()).and_then(|i| world.prop_of_object(i)).is_some(),
        "the barrel is a loose prop"
    );
    a.within(9_000);
}

#[test]
fn task_make_a_collectible_and_a_win_condition_and_prove_it() {
    let mut a = Agent::new("coin_win");
    let hits =
        a.run(&["search", "how", "do", "I", "make", "a", "coin", "that", "adds", "to", "the", "score", "and", "a", "win", "condition", "--limit", "3"]).1;
    assert!(hits.contains("rules") || hits.contains("coin_run"), "search finds the rules docs / the recipe:\n{hits}");
    // Read the topic once; it carries a complete, runnable example scene.
    let topic = a.json(&["describe", "rules"]);
    let example = topic["data"]["example"].clone();
    assert!(example["rules"].is_array(), "the topic has a runnable example");
    let file = a.write("coin.json", &example);
    let (ok, out) = a.run(&["sim", &file]);
    assert!(ok && out.contains("PASS coin then goal"), "{out}");
    // The agent changes the game (needs two coins to win) and the scenario tells it exactly what no longer holds.
    let mut harder = example.clone();
    harder["rules"][1]["if"] = json!("score >= 2");
    let file2 = a.write("coin2.json", &harder);
    let (ok, out) = a.run(&["sim", &file2]);
    assert!(!ok && out.contains("[FAIL] ended `victory`") && out.contains("did not end"), "a failing scenario names the broken expectation:\n{out}");
    // A typo in a rule is caught at validate time, with the fix.
    let mut typo = example.clone();
    typo["rules"][1]["if"] = json!("scor >= 1");
    let typo_file = a.write("coin3.json", &typo);
    let (ok, out) = a.run(&["validate", &typo_file]);
    assert!(!ok && out.contains("unknown variable `scor`") && out.contains("did you mean `score`"), "{out}");
    a.within(14_000);
}

#[test]
fn task_add_a_replicated_interaction_and_prove_it() {
    let mut a = Agent::new("shoot");
    let hits = a.run(&["search", "player", "shoots", "another", "player", "damage", "health", "--limit", "3"]).1;
    assert!(hits.contains("interact") || hits.contains("weapons") || hits.contains("revolver") || hits.contains("Multiplayer"), "{hits}");
    let topic = a.run(&["describe", "sim"]).1;
    assert!(topic.contains("scenario = {") && topic.contains("hold"), "the `sim` topic explains scenarios and the hold step");
    // A duel in the Test Lab: switch to the revolver, fire twice, expect two hits and no kill.
    let scenario = json!({
        "name": "revolver duel", "spawn_group": "duel",
        "players": [{"id": "a", "spawn": "spawn_a"}, {"id": "b", "spawn": "spawn_b"}],
        "script": [
            {"player": "a", "hold": {"switch": true, "seconds": 0.05}}, {"player": "a", "wait": 0.5},
            {"player": "a", "hold": {"attack": true, "seconds": 0.05}}, {"player": "a", "wait": 0.6},
            {"player": "a", "hold": {"attack": true, "seconds": 0.05}}, {"player": "a", "wait": 0.6}
        ],
        "expect": [{"event": "shot", "count": 2}, {"event": "hit", "count": 2}, {"no_event": "kill"}]
    });
    let sfile = a.write("duel.json", &scenario);
    let (ok, out) = a.run(&["sim", "examples/test_lab.json", "--scenario", &sfile]);
    assert!(ok, "{out}");
    a.within(12_000);
}

#[test]
fn task_diagnose_and_fix_an_unreachable_area() {
    let mut a = Agent::new("unreachable");
    let file = a.file("sealed.json");
    let _ = std::fs::remove_file(&file);
    assert!(a.run(&["recipe", "rooms_and_door", "--new", &file]).0);
    let mut scene = read_json(&file);
    // Seal the doorway between the two rooms: find the wall with the door and remove its openings.
    let mut sealed = false;
    for o in scene["objects"].as_array_mut().unwrap() {
        if o["type"] == "wall" && o["openings"].as_array().is_some_and(|op| op.iter().any(|x| x["kind"] == "door")) && o["from"][0] == o["to"][0] {
            o["x-original-openings"] = o["openings"].take();
            o["openings"] = json!([]);
            sealed = true;
        }
    }
    assert!(sealed, "the recipe has a door wall to seal");
    a.write("sealed.json", &scene);
    let lint = a.json(&["lint", &file]);
    let codes: Vec<&str> = lint["data"].as_array().unwrap().iter().filter_map(|f| f["code"].as_str()).collect();
    assert!(lint["ok"] == false && (codes.contains(&"zone") || codes.contains(&"unreachable")), "lint finds the sealed room: {codes:?}");
    let reach = a.run(&["reach", &file, "--to", "3,0"]).1;
    assert!(
        reach.to_lowercase().contains("not") || reach.contains("unreachable") || reach.contains("NOT"),
        "reach says the far room cannot be reached:\n{reach}"
    );
    // The fix: put the door back. The tools agree it is fixed.
    for o in scene["objects"].as_array_mut().unwrap() {
        if o.get("x-original-openings").is_some() {
            o["openings"] = o["x-original-openings"].take();
        }
    }
    a.write("sealed.json", &scene);
    let (ok, out) = a.run(&["lint", &file]);
    assert!(ok, "fixed: {out}");
    a.within(10_000);
}

#[test]
fn task_start_a_server_and_two_scripted_clients() {
    let mut a = Agent::new("server_bots");
    let hits = a.run(&["search", "start", "a", "server", "and", "connect", "bots", "--limit", "3"]).1;
    assert!(hits.contains("red_server") || hits.contains("Multiplayer"), "{hits}");
    let mut server = Command::new(env!("CARGO_BIN_EXE_red_server"))
        .current_dir(root())
        .args(["--map", "examples/test_lab.json", "--spawn-group", "duel", "--port", "0", "--run-for", "6", "--stats-secs", "0"])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("start red_server");
    use std::io::{BufRead, BufReader};
    let mut lines = BufReader::new(server.stdout.take().unwrap()).lines();
    let addr = loop {
        let l = lines.next().expect("server output").unwrap();
        if let Some(rest) = l.strip_prefix("LISTENING ") {
            break rest.trim().to_string();
        }
    };
    // Keep draining the server's output so it never blocks or fails on a closed pipe.
    std::thread::spawn(move || lines.for_each(drop));
    let bot = |behavior: &str| {
        Command::new(env!("CARGO_BIN_EXE_red_bot"))
            .current_dir(root())
            .args([
                "--server",
                &addr.replace("0.0.0.0", "127.0.0.1"),
                "--map",
                "examples/test_lab.json",
                "--behavior",
                behavior,
                "--duration",
                "2",
                "--report-every",
                "1",
            ])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("start red_bot")
    };
    let (b1, b2) = (bot("forward:90"), bot("idle"));
    for child in [b1, b2] {
        let out = child.wait_with_output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        let summary = text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).find(|v| v.get("summary").is_some()).expect("a summary line");
        assert_eq!(summary["summary"]["conn"], "Connected", "{text}");
        a.context += text.len();
    }
    let _ = server.kill();
    let _ = server.wait();
    a.within(9_000);
}

#[test]
fn task_change_a_movement_rule_safely() {
    let mut a = Agent::new("movement");
    let hits = a.run(&["search", "walk", "speed", "movement", "rule", "--limit", "3"]).1;
    assert!(hits.contains("WALK_SPEED") || hits.contains("walk_speed"), "search finds where the number lives without reading a file:\n{hits}");
    let physics = a.run(&["describe", "physics"]).1;
    assert!(physics.contains("walk_speed_mps"), "the live constants are described");
    let shown = a.run(&["src", "show", "step_player"]).1;
    assert!(shown.lines().count() < 60 && shown.contains("fn step_player"), "one function, bounded, not the file");
    let refs = a.run(&["src", "refs", "WALK_SPEED", "--limit", "10"]).1;
    assert!(refs.contains("player.rs"), "{refs}");
    // The safety net for the change exists as commands too: the walk checks and the real-UDP prediction tests replay this movement.
    let (ok, out) = a.run(&["verify", "examples/test_lab.json", "--no-views", "--only", "walk"]);
    assert!(ok && out.contains("PASS walk"), "{out}");
    let player_rs =
        std::fs::metadata(root().join("src/player.rs")).unwrap().len() as usize + std::fs::metadata(root().join("src/sim/player.rs")).unwrap().len() as usize;
    assert!(a.context * 2 < player_rs + 12_000, "the whole task read {} bytes; the two player files alone are {player_rs}", a.context);
    a.within(9_000);
}
