//! The engine's own documents as `search`, `describe` and the planner read them: SPEC, AGENTS, the agent reference, PLAY_2D, the glossary, the
//! ADRs and their index, the analysis notes and `docs/features.json`. They are read from the engine checkout this binary runs in (or was built
//! from), so an edit to one is seen by the next command and compiles nothing. Only release-type builds (`--release`, the `fast` and `bench`
//! profiles: `build.rs` sets `red_embed_docs` for them, or `RED_EMBED_DOCS=1` for any) also embed a copy, the fallback of an installed binary that
//! runs outside a checkout (ADR 2026-10-13-the-engine-reads-its-own-docs-from-the-checkout).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The documents, as one process reads them.
#[derive(Debug, Clone, Default)]
pub struct Docs {
    /// The checkout they were read from; `None` for the embedded copies (or when there was nothing to read).
    pub root: Option<PathBuf>,
    /// `SPEC.md`.
    pub spec: String,
    /// `AGENTS.md`.
    pub agents: String,
    /// `docs/AGENT_REFERENCE.md`.
    pub agent_reference: String,
    /// `docs/PLAY_2D.md`.
    pub play_2d: String,
    /// `docs/GLOSSARY.md` (`describe glossary`, `search --kind glossary`).
    pub glossary: String,
    /// `docs/adr/README.md`: the generated index table (`describe decisions`).
    pub adr_index: String,
    /// `(file name, text)` of every `docs/adr/*.md` but the README, sorted by name.
    pub adrs: Vec<(String, String)>,
    /// `(file name, text)` of every `docs/analysis/*.md`, sorted by name.
    pub analyses: Vec<(String, String)>,
    /// `docs/features.json`: which feature owns which files (ADR 0033).
    pub features: String,
}

/// Whether `dir` is an engine checkout with its docs: the sources and SPEC.md and `docs/adr/`, so a game project (which has none of them) is never taken for one.
pub fn is_checkout(dir: &Path) -> bool {
    dir.join("src/lib.rs").is_file() && dir.join("SPEC.md").is_file() && dir.join("docs/adr").is_dir()
}

/// The engine checkout to read: `$RE2_SRC`, else the tree this binary was built from, else the current directory or the nearest parent that is one.
/// `None` for an installed binary run outside any checkout.
pub fn checkout() -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Ok(p) = std::env::var("RE2_SRC") {
        cands.push(PathBuf::from(p));
    }
    cands.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    if let Ok(mut d) = std::env::current_dir() {
        loop {
            cands.push(d.clone());
            if !d.pop() {
                break;
            }
        }
    }
    cands.into_iter().find(|c| is_checkout(c))
}

/// The documents this process uses, read once: the checkout's when there is one, else the embedded copies (release-type builds), else empty.
pub fn docs() -> &'static Docs {
    static DOCS: OnceLock<Docs> = OnceLock::new();
    DOCS.get_or_init(|| match checkout() {
        Some(root) => read_from(&root),
        None => embedded().unwrap_or_default(),
    })
}

/// The documents of the checkout at `root`. A missing file reads as empty (the docs tests say which one is missing).
pub fn read_from(root: &Path) -> Docs {
    let text = |rel: &str| std::fs::read_to_string(root.join(rel)).unwrap_or_default();
    Docs {
        root: Some(root.to_path_buf()),
        spec: text("SPEC.md"),
        agents: text("AGENTS.md"),
        agent_reference: text("docs/AGENT_REFERENCE.md"),
        play_2d: text("docs/PLAY_2D.md"),
        glossary: text("docs/GLOSSARY.md"),
        adr_index: text("docs/adr/README.md"),
        adrs: md_files(&root.join("docs/adr")),
        analyses: md_files(&root.join("docs/analysis")),
        features: text("docs/features.json"),
    }
}

/// `(file name, text)` of the `*.md` files directly in `dir` except `README.md`, sorted by name.
fn md_files(dir: &Path) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    (name.ends_with(".md") && name != "README.md").then(|| std::fs::read_to_string(e.path()).ok().map(|t| (name, t))).flatten()
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// The copies embedded at build time: `Some` only in release-type builds (see the module docs).
pub fn embedded() -> Option<Docs> {
    #[cfg(red_embed_docs)]
    {
        let own = |list: &[(&str, &str)]| list.iter().map(|(n, t)| (n.to_string(), t.to_string())).collect();
        Some(Docs {
            root: None,
            spec: copies::SPEC.to_string(),
            agents: copies::AGENTS.to_string(),
            agent_reference: copies::AGENT_REFERENCE.to_string(),
            play_2d: copies::PLAY_2D.to_string(),
            glossary: copies::GLOSSARY.to_string(),
            adr_index: copies::ADR_INDEX.to_string(),
            adrs: own(copies::ADR_FILES),
            analyses: own(copies::ANALYSIS_FILES),
            features: copies::FEATURES.to_string(),
        })
    }
    #[cfg(not(red_embed_docs))]
    {
        None
    }
}

/// The embedded copies (release-type builds only: a dev build never compiles these, so editing a doc never recompiles it).
#[cfg(red_embed_docs)]
mod copies {
    pub const SPEC: &str = include_str!("../../SPEC.md");
    pub const AGENTS: &str = include_str!("../../AGENTS.md");
    pub const AGENT_REFERENCE: &str = include_str!("../../docs/AGENT_REFERENCE.md");
    pub const PLAY_2D: &str = include_str!("../../docs/PLAY_2D.md");
    pub const GLOSSARY: &str = include_str!("../../docs/GLOSSARY.md");
    pub const ADR_INDEX: &str = include_str!("../../docs/adr/README.md");
    pub const FEATURES: &str = include_str!("../../docs/features.json");
    include!(concat!(env!("OUT_DIR"), "/adr_files.rs"));
    include!(concat!(env!("OUT_DIR"), "/analysis_files.rs"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dev_build_reads_its_docs_from_the_checkout_and_embeds_none() {
        let d = docs();
        assert_eq!(d.root.as_deref().map(is_checkout), Some(true), "tests run in the checkout");
        assert!(d.spec.contains("scene language"), "SPEC.md is read");
        assert!(d.agents.contains("AGENTS") || d.agents.contains("Red Engine"), "AGENTS.md is read");
        assert!(!d.adrs.is_empty() && !d.analyses.is_empty(), "the ADRs and the analysis notes are read");
        assert!(d.features.contains("\"features\""), "docs/features.json is read");
        if !cfg!(red_embed_docs) {
            assert!(embedded().is_none(), "a dev build carries no copies, so a doc edit compiles nothing");
        }
    }

    /// A release-type build (`RED_EMBED_DOCS=1` forces one) carries copies equal to the checkout it was built from.
    #[cfg(red_embed_docs)]
    #[test]
    fn the_embedded_copies_are_the_checkout_s_documents() {
        let (e, d) = (embedded().expect("a release-type build embeds the docs"), read_from(Path::new(env!("CARGO_MANIFEST_DIR"))));
        assert_eq!((e.spec.len(), e.glossary.len(), e.features.len()), (d.spec.len(), d.glossary.len(), d.features.len()));
        assert_eq!(e.adrs.iter().map(|(n, _)| n).collect::<Vec<_>>(), d.adrs.iter().map(|(n, _)| n).collect::<Vec<_>>());
        assert_eq!(e.analyses.len(), d.analyses.len());
    }

    #[test]
    fn a_new_adr_in_a_checkout_is_read_without_a_rebuild_and_a_game_project_is_not_a_checkout() {
        let dir = std::env::temp_dir().join(format!("re2_own_docs_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docs/adr")).unwrap();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        // A game project has docs of its own but no engine sources: not a checkout.
        std::fs::write(dir.join("SPEC.md"), "# spec\n").unwrap();
        assert!(!is_checkout(&dir));
        std::fs::write(dir.join("src/lib.rs"), "").unwrap();
        assert!(is_checkout(&dir));
        std::fs::write(dir.join("docs/adr/2099-01-01-written-after-the-build.md"), "# 2099-01-01. Written after the build\n").unwrap();
        std::fs::write(dir.join("docs/adr/README.md"), "index").unwrap();
        let d = read_from(&dir);
        assert_eq!(d.adrs.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["2099-01-01-written-after-the-build.md"], "README is the index, not an ADR");
        assert_eq!((d.adr_index.as_str(), d.glossary.as_str()), ("index", ""), "a missing file reads as empty");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
