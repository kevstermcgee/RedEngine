//! Docs must not lie (ADR 0025, extending ADR 0007). The AI-facing prose is where a stale sentence does the most damage
//! ("multiplayer is not built", "95+ tests" stayed in a fork's CLAUDE.md long after they were false), so it is checked:
//! derived facts are current, prose states no test counts, every path a doc points at exists, every blueprint key is
//! documented and every CLI command is mentioned where an agent looks.

use red_engine2::tools::{blueprint, status};
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}")).replace("\r\n", "\n")
}

/// The docs an agent reads first.
const AI_DOCS: &[&str] = &["CLAUDE.md", "AGENTS.md"];

#[test]
fn claude_md_facts_block_matches_the_repo() {
    let doc = read("CLAUDE.md");
    let have = status::current_facts(&doc).expect("CLAUDE.md needs a <!-- facts:begin --> ... <!-- facts:end --> block");
    let want = status::facts_block(&root()).trim().to_string();
    assert_eq!(have, want, "CLAUDE.md's derived facts are stale: run `red_engine2 status --sync-docs CLAUDE.md`");
}

#[test]
fn prose_states_no_test_counts_and_no_stale_claims() {
    for f in AI_DOCS.iter().chain(["README.md"].iter()) {
        let text = read(f);
        let words: Vec<&str> = text.split_whitespace().collect();
        for (i, w) in words.iter().enumerate() {
            let next = words.get(i + 1).map(|n| n.trim_matches(|c: char| !c.is_alphanumeric())).unwrap_or("");
            let next2 = words.get(i + 2).map(|n| n.trim_matches(|c: char| !c.is_alphanumeric())).unwrap_or("");
            let counted = w.trim_end_matches('+').parse::<u32>().is_ok();
            let about_tests = next == "tests" || (next == "unit" && next2 == "tests");
            assert!(!(counted && about_tests), "{f}: `{w} {next}...` is a hand-written test count; it goes stale. Say \"run scripts/dev test\" instead");
        }
    }
    for f in AI_DOCS {
        let text = read(f).to_lowercase();
        for banned in ["multiplayer is **not built**", "**not built** (adr 0010)", "no networking at all", "a headless server is not built"] {
            assert!(!text.contains(banned), "{f} still says `{banned}`; multiplayer is built (ADR 0016, 0017, 0022)");
        }
    }
}

/// Inline-code tokens like `scripts/dev` or `docs/adr/0024-...md` that name a file this repo should contain.
fn referenced_paths(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (n, span) in text.split('`').enumerate() {
        if n % 2 == 0 {
            continue; // outside backticks
        }
        for tok in span.split_whitespace() {
            let tok = tok.trim_matches(|c: char| ",.;:)(".contains(c));
            let known = ["scripts/", "docs/", "deploy/", "tests/", "recipes/", "assets/"].iter().any(|p| tok.starts_with(p));
            let skip = tok.contains(['*', '<', '>', '{', '$', '|']) || tok.contains("...") || tok.contains("NNNN") || tok.starts_with("scripts/red");
            if known && !skip {
                out.push(tok.to_string());
            }
        }
    }
    out
}

#[test]
fn every_path_the_ai_docs_mention_exists() {
    for f in AI_DOCS.iter().chain(["docs/HOSTING.md"].iter()) {
        for p in referenced_paths(&read(f)) {
            assert!(
                root().join(&p).exists() || Path::new(&p).extension().is_none() && root().join(format!("{p}.md")).exists(),
                "{f} mentions `{p}`, which does not exist"
            );
        }
    }
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
    let commands: Vec<String> =
        v["data"]["commands"].as_array().expect("describe brief lists the commands").iter().filter_map(|c| c.as_str().map(str::to_string)).collect();
    assert!(commands.len() > 25, "{commands:?}");
    let agents = read("AGENTS.md");
    let missing: Vec<&String> = commands.iter().filter(|c| !agents.contains(&format!("`{c}")) && !agents.contains(&format!("| `{c}"))).collect();
    assert!(missing.is_empty(), "AGENTS.md does not mention these commands (add them to the tool table): {missing:?}");
}
