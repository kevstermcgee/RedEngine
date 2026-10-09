//! Docs must not lie (ADR 0025, extending ADR 0007). The AI-facing prose is where a stale sentence does the most damage
//! ("multiplayer is not built", "95+ tests" stayed in a fork's CLAUDE.md long after they were false), so it is checked:
//! derived facts are current, prose states no test counts, every path a doc points at exists, every blueprint key is
//! documented and every CLI command is mentioned where an agent looks.
//!
//! The checks themselves are `red_engine2::tools::preflight` functions (one implementation): `red_engine2 preflight` runs them in a second and prints the
//! exact edit for each problem, so a bounce here is found before a full test cycle.

use red_engine2::tools::{blueprint, doc_claims, preflight, status};
use std::path::PathBuf;
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}")).replace("\r\n", "\n")
}

#[test]
fn claude_md_facts_block_matches_the_repo() {
    let doc = read("CLAUDE.md");
    let have = status::current_facts(&doc).expect("CLAUDE.md needs a <!-- facts:begin --> ... <!-- facts:end --> block");
    let want = status::facts_block(&root()).trim().to_string();
    assert_eq!(have, want, "CLAUDE.md's derived facts are stale: run `red_engine2 preflight --fix` (or `red_engine2 status --sync-docs CLAUDE.md`)");
}

#[test]
fn inline_facts_in_the_docs_are_current() {
    let mut stale = Vec::new();
    for doc in doc_claims::CLAIM_DOCS {
        for (name, have, want) in status::inline_facts(&root(), &read(doc)) {
            if want.as_deref() != Some(have.as_str()) {
                stale.push(format!("{doc}: <!--fact:{name}--> says {have}, the code says {want:?}"));
            }
        }
    }
    assert!(stale.is_empty(), "a number derived from the code is stale: run `red_engine2 preflight --fix`\n  {}", stale.join("\n  "));
    assert!(
        read("SPEC.md").contains("<!--fact:protocol-->"),
        "SPEC.md states the wire protocol version as a derived fact, so a protocol bump cannot leave it behind"
    );
}

#[test]
fn prose_states_no_test_counts_and_no_stale_claims() {
    let mut bad = preflight::test_count_claims(&root());
    bad.extend(preflight::stale_claims(&root()));
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// The claims a reader acts on, checked against the repository (src/tools/doc_claims.rs): a protocol version written as a number, a feature described as
/// missing after it was built, a document's size, an MCP tool that does not exist, a `render` command shown without the cargo feature it needs.
#[test]
fn the_front_door_documents_make_no_claim_the_repository_disproves() {
    let bad = doc_claims::all(&root());
    assert!(bad.is_empty(), "a document says something the repository disproves (`red_engine2 preflight` prints the same list):\n  {}", bad.join("\n  "));
}

#[test]
fn every_path_the_ai_docs_mention_exists() {
    let missing = preflight::missing_referenced_paths(&root());
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

#[test]
fn every_blueprint_key_is_documented_in_the_spec() {
    let spec = read("SPEC.md");
    let section = &spec[spec.find("## Blueprints").expect("SPEC.md needs a Blueprints section")..];
    let section = &section[..section.find("\n## Validation").unwrap_or(section.len())];
    for key in blueprint::TOP_KEYS.iter().chain(blueprint::ROOM_KEYS).chain(blueprint::DOOR_KEYS).chain(blueprint::SPAWN_KEYS).chain(blueprint::FILL_KEYS) {
        assert!(section.contains(&format!("`{key}")), "blueprint key `{key}` is accepted by the compiler but not documented in SPEC.md's Blueprints section");
    }
}

#[test]
fn every_cli_command_is_mentioned_in_agents_md() {
    let out = Command::new(env!("CARGO_BIN_EXE_red_engine2")).args(["--json", "describe", "brief"]).output().expect("run red_engine2");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("--json prints one document");
    let commands: Vec<(String, String)> = v["data"]["commands"]
        .as_array()
        .expect("describe brief lists the commands")
        .iter()
        .filter_map(|c| c.as_str().map(|s| (s.to_string(), String::new())))
        .collect();
    assert!(commands.len() > 25, "{commands:?}");
    let missing: Vec<String> = preflight::unmentioned_commands(&root(), &commands).into_iter().map(|(c, _)| c).collect();
    assert!(
        missing.is_empty(),
        "AGENTS.md / docs/AGENT_REFERENCE.md do not mention these commands (add them to the tool table in the reference; `red_engine2 preflight --fix` adds a row): {missing:?}"
    );
}
