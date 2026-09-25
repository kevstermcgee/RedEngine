//! What a stranger sees when they open the repository (found by an outside review of the public repo): a LICENSE (without one nobody may
//! legally reuse the code), no README sentence that contradicts what the engine does, and no markdown link that points outside the repo.
//! `docs_fresh.rs` guards the AI-facing docs; this guards the front door.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &Path) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{}: {e}", rel.display())).replace("\r\n", "\n")
}

/// Every tracked-looking markdown file (top level and `docs/`, `deploy/`, `recipes/`, `benches/`), never `target/` or `out/`.
fn markdown_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(root().join(dir)) else { return };
        for e in rd.flatten() {
            let rel = dir.join(e.file_name());
            let name = e.file_name().to_string_lossy().to_string();
            if e.path().is_dir() {
                if !matches!(name.as_str(), "target" | "out" | ".git" | "node_modules" | "history") {
                    walk(&rel, out);
                }
            } else if name.ends_with(".md") {
                out.push(rel);
            }
        }
    }
    let mut out = Vec::new();
    walk(Path::new(""), &mut out);
    out
}

#[test]
fn the_repository_has_a_license_and_cargo_says_which() {
    let license = read(Path::new("LICENSE"));
    assert!(license.contains("Permission is hereby granted"), "LICENSE must be a real licence text");
    let cargo = read(Path::new("Cargo.toml"));
    assert!(cargo.contains("license = \"MIT\""), "Cargo.toml must name the licence of LICENSE (`license = \"MIT\"`)");
}

#[test]
fn the_front_door_docs_state_nothing_the_engine_disproves() {
    let banned = [
        "no online multiplayer",
        "networking is not built",
        "no networking",
        "multiplayer is not built",
        "no on-screen 2d text/ui overlay",
        "that's the next thing planned on top of this fork",
    ];
    for f in ["README.md", "SPEC.md", "AGENTS.md", "CLAUDE.md"] {
        let text = read(Path::new(f)).to_lowercase();
        for b in banned {
            assert!(!text.contains(b), "{f} still says `{b}`, which is no longer true (see ADR 0016, 0022, 0026)");
        }
    }
}

#[test]
fn every_relative_markdown_link_stays_inside_the_repo_and_resolves() {
    let mut problems = Vec::new();
    for f in markdown_files() {
        let text = read(&f);
        let mut rest = text.as_str();
        while let Some(i) = rest.find("](") {
            rest = &rest[i + 2..];
            let Some(end) = rest.find(')') else { break };
            let target = &rest[..end];
            rest = &rest[end..];
            let path = target.split(['#', ' ']).next().unwrap_or("");
            if path.is_empty() || path.contains("://") || path.starts_with("mailto:") || path.contains(['<', '{', '$']) {
                continue;
            }
            let dir = f.parent().unwrap_or(Path::new(""));
            let resolved = root().join(dir).join(path);
            let canon_root = root().canonicalize().unwrap_or_else(|_| root());
            match resolved.canonicalize() {
                Ok(p) if p.starts_with(&canon_root) => {}
                Ok(_) => problems.push(format!("{}: link `{target}` leaves the repository", f.display())),
                Err(_) => problems.push(format!("{}: link `{target}` does not exist", f.display())),
            }
        }
    }
    assert!(problems.is_empty(), "broken markdown links:\n{}", problems.join("\n"));
}
