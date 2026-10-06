//! `red_engine2 mcp` over its real stdio protocol (ADR 2026-10-06-a-native-mcp-server): the tool surface, a map proved and edited by path, images, refused commands,
//! and the one thing a stdio server must never do, which is print anything but protocol on stdout (the client panics on any other line).

use serde_json::json;
use std::path::PathBuf;

#[path = "support/mcp.rs"]
mod mcp;

const BIN: &str = env!("CARGO_BIN_EXE_red_engine2");

fn project(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("re2_mcp_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("recipes/rooms_and_door.json"), dir.join("map.json")).unwrap();
    dir
}

#[test]
fn the_server_lists_a_small_set_of_typed_tools() {
    let dir = project("list");
    let mut c = mcp::McpClient::start(BIN, &dir);
    let tools = c.list();
    let mut names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    names.sort();
    assert_eq!(names, ["analyze", "context", "describe", "lint", "patch", "run", "search", "sim", "validate", "verify", "view"]);
    for t in &tools {
        assert_eq!(t["inputSchema"]["type"], "object", "{t}");
        assert!(t["description"].as_str().is_some_and(|d| !d.is_empty()));
    }
    let patch = tools.iter().find(|t| t["name"] == "patch").unwrap();
    assert_eq!(patch["inputSchema"]["required"], json!(["file", "ops"]), "the argument struct is the schema");
    assert!(c.tools_list_bytes < 8_000, "tools/list is {} bytes (the Python adapter's was 21618)", c.tools_list_bytes);
}

#[test]
fn a_map_is_validated_linted_edited_and_looked_at_by_path_never_by_content() {
    let dir = project("flow");
    let mut c = mcp::McpClient::start(BIN, &dir);
    let r = c.call("validate", json!({"file": "map.json"}));
    assert!(!r.is_error && r.text.contains("OK"), "{}", r.text);
    let r = c.call("lint", json!({"file": "map.json"}));
    assert!(!r.is_error, "the recipe lints clean: {}", r.text);
    let before = std::fs::read_to_string(dir.join("map.json")).unwrap();
    let r = c.call("patch", json!({"file": "map.json", "ops": [{"op": "move", "id": "lamp_1", "by": [0.5, 0.0, 0.0]}]}));
    assert!(!r.is_error, "{}", r.text);
    let after = std::fs::read_to_string(dir.join("map.json")).unwrap();
    assert_ne!(before, after, "the patch landed in the file");
    // An invalid edit lands nowhere and says why.
    let r = c.call("patch", json!({"file": "map.json", "ops": [{"op": "move", "id": "no_such_object", "by": [1, 0, 0]}]}));
    assert!(r.is_error, "{}", r.text);
    assert_eq!(std::fs::read_to_string(dir.join("map.json")).unwrap(), after, "a failed patch changes nothing");
    // A preview prints through the capture, not onto the protocol stream.
    let r = c.call("patch", json!({"file": "map.json", "ops": [{"op": "move", "id": "lamp_1", "by": [1.0, 0.0, 0.0]}], "dry_run": true}));
    assert!(r.text.contains("dry run"), "{}", r.text);
    assert_eq!(std::fs::read_to_string(dir.join("map.json")).unwrap(), after);
    let r = c.call("verify", json!({"file": "map.json"}));
    assert!(!r.text.is_empty());
    let r = c.call("view", json!({"file": "map.json", "kind": "plan"}));
    assert!(!r.is_error && r.images.len() == 1, "a plan comes back as one image: {}", r.text);
    use base64::Engine as _;
    let png = base64::engine::general_purpose::STANDARD.decode(&r.images[0]).unwrap();
    assert_eq!(&png[..4], &[0x89, b'P', b'N', b'G'], "the image is a PNG");
    assert!(c.sent_bytes < 1_500, "the agent sent {} bytes for the whole flow: paths and edits, never the scene", c.sent_bytes);
}

#[test]
fn commands_that_never_return_are_refused_and_the_rest_of_the_cli_is_reachable() {
    let dir = project("run");
    let mut c = mcp::McpClient::start(BIN, &dir);
    for args in [json!(["game", "serve"]), json!(["game", "play-local"]), json!(["web", "serve", "out/web"]), json!(["portmap", "keep"]), json!(["mcp"])] {
        let r = c.call("run", json!({"args": args}));
        assert!(r.is_error && r.text.contains("runs until stopped"), "{args}: {}", r.text);
    }
    let r = c.call("run", json!({"args": ["catalog", "apple"]}));
    assert!(!r.is_error && r.text.to_lowercase().contains("apple"), "{}", r.text);
    let r = c.call("run", json!({"args": ["no-such-command"]}));
    assert!(r.is_error, "an unknown command is an error result, not a dead server");
    let r = c.call("describe", json!({}));
    assert!(r.text.starts_with("Red Engine 2"), "the manual answers after a failure: {}", r.text);
    let r = c.call("analyze", json!({"what": "teleport", "file": "map.json"}));
    assert!(r.is_error && r.text.contains("reach, walk, ray, ls, info"), "{}", r.text);
}

#[test]
fn repeating_a_question_gives_the_same_answer() {
    let dir = project("search");
    let mut c = mcp::McpClient::start(BIN, &dir);
    let a = c.call("search", json!({"query": "how do stairs work", "limit": 3}));
    let b = c.call("search", json!({"query": "how do stairs work", "limit": 3}));
    assert!(!a.is_error && !a.text.is_empty());
    assert_eq!(a.text, b.text, "the cached search index answers exactly as a fresh one");
}
