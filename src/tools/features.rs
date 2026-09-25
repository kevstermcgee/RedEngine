//! `red_engine2 features` and `red_engine2 impact`: which parts of the engine exist, which files belong to each, and **what must pass when a
//! file changes** (ADR 0033). The index is `docs/features.json`, compiled into the binary, and it cannot rot: `features --check` (also a
//! test) fails when a listed file, test suite, doc or ADR does not exist, or when a source file belongs to no feature at all.
//!
//! `impact` is the part a plain "feature -> files -> checks" table cannot do: give it the changed files (or `--git` for the working tree)
//! and it names the features they belong to, the features that *depend* on those (their tests can break too), and prints the exact
//! `cargo test` commands and verification commands to run, cheapest first, plus the docs and decisions to read and update.

use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

/// The index, compiled in.
pub const INDEX: &str = include_str!("../../docs/features.json");

/// One feature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feature {
    /// Its short name (`match_flow`).
    pub name: String,
    /// One sentence.
    pub summary: String,
    /// File patterns (`src/net/server.rs`, `src/sim/**`, `docs/adr/0029-*.md`).
    pub files: Vec<String>,
    /// Integration test targets (`net_flow`) and library test filters (`lib:sim::flow`).
    pub tests: Vec<String>,
    /// Verification commands worth running for this feature.
    pub commands: Vec<String>,
    /// Docs and decisions to read and keep true.
    pub docs: Vec<String>,
    /// Features this one is built on: a change there can break this one.
    pub depends_on: Vec<String>,
}

/// Parses the compiled-in index.
pub fn load() -> Result<Vec<Feature>, String> {
    parse(INDEX)
}

/// Parses an index document.
pub fn parse(text: &str) -> Result<Vec<Feature>, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("features.json is not JSON: {e}"))?;
    let map = v.get("features").and_then(Value::as_object).ok_or("features.json needs a top-level \"features\" object")?;
    let strings = |f: &Value, key: &str, name: &str| -> Result<Vec<String>, String> {
        match f.get(key) {
            None => Ok(Vec::new()),
            Some(a) => a
                .as_array()
                .ok_or(format!("features.{name}.{key} must be an array"))?
                .iter()
                .map(|x| x.as_str().map(str::to_string).ok_or(format!("features.{name}.{key} must hold strings")))
                .collect(),
        }
    };
    let mut out = Vec::new();
    for (name, f) in map {
        out.push(Feature {
            name: name.clone(),
            summary: f.get("summary").and_then(Value::as_str).ok_or(format!("features.{name}.summary is required"))?.to_string(),
            files: strings(f, "files", name)?,
            tests: strings(f, "tests", name)?,
            commands: strings(f, "commands", name)?,
            docs: strings(f, "docs", name)?,
            depends_on: strings(f, "depends_on", name)?,
        });
    }
    Ok(out)
}

/// Glob match of `path` against `pattern`: `*` matches within one path segment, `**` any number of whole segments, a trailing `/` a whole directory.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    fn seg(p: &[u8], s: &[u8]) -> bool {
        match p.split_first() {
            None => s.is_empty(),
            Some((b'*', rest)) => (0..=s.len()).any(|i| seg(rest, &s[i..])),
            Some((c, rest)) => s.first() == Some(c) && seg(rest, &s[1..]),
        }
    }
    fn segs(p: &[&str], s: &[&str]) -> bool {
        match p.split_first() {
            None => s.is_empty(),
            Some((&"**", rest)) => (0..=s.len()).any(|i| segs(rest, &s[i..])),
            Some((first, rest)) => s.split_first().is_some_and(|(sf, sr)| seg(first.as_bytes(), sf.as_bytes()) && segs(rest, sr)),
        }
    }
    let pattern = pattern.replace('\\', "/");
    let path = path.replace('\\', "/");
    let pattern = if pattern.ends_with('/') { format!("{pattern}**") } else { pattern };
    let (p, s): (Vec<&str>, Vec<&str>) = (pattern.split('/').collect(), path.split('/').collect());
    segs(&p, &s)
}

/// The features that own `path`: a file matching one of its `files` patterns, or the integration test file of a suite it runs
/// (`tests/net_flow.rs` belongs to every feature that lists `net_flow`, so editing a test says which features it can break).
pub fn owners<'a>(features: &'a [Feature], path: &str) -> Vec<&'a Feature> {
    let path = path.replace('\\', "/");
    let suite = path.strip_prefix("tests/").and_then(|p| p.strip_suffix(".rs")).filter(|s| !s.contains('/'));
    features.iter().filter(|f| f.files.iter().any(|p| glob_match(p, &path)) || suite.is_some_and(|s| f.tests.iter().any(|t| t == s))).collect()
}

/// What changing some files affects.
#[derive(Debug, Clone, Default)]
pub struct Impact {
    /// Features owning a changed file, with the files.
    pub direct: BTreeMap<String, Vec<String>>,
    /// Features that depend (transitively) on a directly affected one.
    pub downstream: BTreeSet<String>,
    /// Changed files no feature owns.
    pub unowned: Vec<String>,
    /// Library test filters to run (`sim::flow`).
    pub lib_filters: BTreeSet<String>,
    /// Integration test targets to run (`net_flow`).
    pub suites: BTreeSet<String>,
    /// Verification commands.
    pub commands: BTreeSet<String>,
    /// Docs and decisions to read and update.
    pub docs: BTreeSet<String>,
}

impl Impact {
    /// The one-line `cargo test` invocations that cover [`Impact::lib_filters`] and [`Impact::suites`].
    pub fn cargo_commands(&self) -> Vec<String> {
        let mut v = Vec::new();
        if !self.lib_filters.is_empty() {
            v.push(format!("cargo test --lib -- {}", self.lib_filters.iter().cloned().collect::<Vec<_>>().join(" ")));
        }
        if !self.suites.is_empty() {
            v.push(format!("cargo test {}", self.suites.iter().map(|s| format!("--test {s}")).collect::<Vec<_>>().join(" ")));
        }
        v
    }
}

/// Computes the impact of changing `changed` files.
pub fn impact(features: &[Feature], changed: &[String]) -> Impact {
    let mut out = Impact::default();
    for path in changed {
        let path = path.replace('\\', "/");
        let owned = owners(features, &path);
        if owned.is_empty() {
            out.unowned.push(path);
            continue;
        }
        for f in owned {
            out.direct.entry(f.name.clone()).or_default().push(path.clone());
        }
    }
    // Everything built on an affected feature is affected too.
    let mut affected: BTreeSet<String> = out.direct.keys().cloned().collect();
    loop {
        let more: Vec<String> =
            features.iter().filter(|f| !affected.contains(&f.name) && f.depends_on.iter().any(|d| affected.contains(d))).map(|f| f.name.clone()).collect();
        if more.is_empty() {
            break;
        }
        out.downstream.extend(more.iter().cloned());
        affected.extend(more);
    }
    for f in features.iter().filter(|f| affected.contains(&f.name)) {
        for t in &f.tests {
            match t.strip_prefix("lib:") {
                Some(filter) => out.lib_filters.insert(filter.to_string()),
                None => out.suites.insert(t.clone()),
            };
        }
        out.commands.extend(f.commands.iter().cloned());
        out.docs.extend(f.docs.iter().cloned());
    }
    out
}

/// The files changed in the working tree relative to `base` (default `HEAD`), plus untracked files.
pub fn changed_files(root: &Path, base: &str) -> Result<Vec<String>, String> {
    let run = |args: &[&str]| -> Result<String, String> {
        let out = Command::new("git").args(args).current_dir(root).output().map_err(|e| format!("cannot run git: {e}"))?;
        if !out.status.success() {
            return Err(format!("git {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    };
    let mut files: BTreeSet<String> = run(&["diff", "--name-only", base])?.lines().map(str::to_string).collect();
    files.extend(run(&["ls-files", "--others", "--exclude-standard"])?.lines().map(str::to_string));
    Ok(files.into_iter().filter(|f| !f.starts_with("out/") && !f.starts_with("target/")).collect())
}

/// Every file under `root/dir` with the extension, as paths relative to `root` using `/`.
fn walk(root: &Path, dir: &str, ext: &str, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(root.join(dir)) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let rel = format!("{dir}/{name}");
        if e.path().is_dir() {
            walk(root, &rel, ext, out);
        } else if name.ends_with(ext) {
            out.push(rel);
        }
    }
}

/// Problems with the index: a listed thing that does not exist, or a source file no feature owns. Empty = the index is true.
pub fn check(features: &[Feature], root: &Path) -> Vec<String> {
    let mut problems = Vec::new();
    let mut all_files = Vec::new();
    for dir in ["src", "tests", "benches", "docs", "assets", "recipes", "examples", "scripts", "deploy", ".github"] {
        walk(root, dir, "", &mut all_files);
    }
    for top in [
        "Cargo.toml",
        "Cargo.lock",
        "Dockerfile",
        "docker-compose.yml",
        "mcp_server.py",
        "README.md",
        "SPEC.md",
        "AGENTS.md",
        "CLAUDE.md",
        "LICENSE",
        "rustfmt.toml",
    ] {
        if root.join(top).exists() {
            all_files.push(top.to_string());
        }
    }
    let names: BTreeSet<&str> = features.iter().map(|f| f.name.as_str()).collect();
    for f in features {
        for p in &f.files {
            if !all_files.iter().any(|a| glob_match(p, a)) {
                problems.push(format!("feature '{}': file pattern '{p}' matches nothing", f.name));
            }
        }
        for t in &f.tests {
            match t.strip_prefix("lib:") {
                Some(filter) => {
                    // `sim::flow` must be a real module path: src/sim/flow.rs or src/sim/flow/mod.rs (or a directory for a prefix).
                    let rel = filter.replace("::", "/");
                    let head = rel.trim_end_matches('/');
                    let exists = ["", ".rs", "/mod.rs"].iter().any(|suffix| root.join("src").join(format!("{head}{suffix}")).exists());
                    if !exists {
                        problems.push(format!("feature '{}': library test filter '{filter}' is not a module under src/", f.name));
                    }
                }
                None => {
                    if !root.join("tests").join(format!("{t}.rs")).exists() {
                        problems.push(format!("feature '{}': test suite '{t}' has no tests/{t}.rs", f.name));
                    }
                }
            }
        }
        for d in &f.docs {
            if !root.join(d).exists() {
                problems.push(format!("feature '{}': doc '{d}' does not exist", f.name));
            }
        }
        for d in &f.depends_on {
            if !names.contains(d.as_str()) {
                problems.push(format!("feature '{}': depends_on unknown feature '{d}'", f.name));
            }
        }
        if f.files.is_empty() {
            problems.push(format!("feature '{}' owns no files", f.name));
        }
    }
    // Every source file, test suite and bench belongs to a feature (otherwise `impact` cannot say what a change there affects).
    for file in all_files.iter().filter(|f| {
        (f.starts_with("src/") && f.ends_with(".rs")) || (f.starts_with("tests/") && f.ends_with(".rs")) || (f.starts_with("benches/") && f.ends_with(".rs"))
    }) {
        if owners(features, file).is_empty() {
            problems.push(format!("{file} belongs to no feature: add it to docs/features.json"));
        }
    }
    problems
}

/// `features` as text: the list, one line per feature.
pub fn render_list(features: &[Feature]) -> String {
    let w = features.iter().map(|f| f.name.len()).max().unwrap_or(0);
    features.iter().map(|f| format!("{:<w$}  {}\n", f.name, f.summary)).collect()
}

/// One feature in full.
pub fn render_one(f: &Feature) -> String {
    let list = |title: &str, v: &[String]| {
        if v.is_empty() {
            String::new()
        } else {
            format!("{title}:\n{}", v.iter().map(|x| format!("  {x}\n")).collect::<String>())
        }
    };
    format!(
        "{}: {}\n{}{}{}{}{}",
        f.name,
        f.summary,
        list("files", &f.files),
        list("tests", &f.tests),
        list("verify with", &f.commands),
        list("docs", &f.docs),
        list("depends on", &f.depends_on)
    )
}

/// Features whose name, summary or files mention every word of `query`.
pub fn find<'a>(features: &'a [Feature], query: &str) -> Vec<&'a Feature> {
    let words: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_string).collect();
    features
        .iter()
        .filter(|f| {
            let hay = format!("{} {} {} {}", f.name, f.summary, f.files.join(" "), f.commands.join(" ")).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect()
}

/// The impact as text.
pub fn render_impact(i: &Impact, changed: usize) -> String {
    let mut s = format!("{changed} changed file(s)\n");
    if i.direct.is_empty() {
        s.push_str("no feature owns any of them\n");
    }
    for (f, files) in &i.direct {
        s.push_str(&format!("feature {f}: {}\n", files.join(", ")));
    }
    if !i.downstream.is_empty() {
        s.push_str(&format!("also affected (they depend on the above): {}\n", i.downstream.iter().cloned().collect::<Vec<_>>().join(", ")));
    }
    if !i.unowned.is_empty() {
        s.push_str(&format!("owned by no feature (add to docs/features.json): {}\n", i.unowned.join(", ")));
    }
    let cmds = i.cargo_commands();
    if !cmds.is_empty() {
        s.push_str("run:\n");
        for c in &cmds {
            s.push_str(&format!("  {c}\n"));
        }
    }
    if !i.commands.is_empty() {
        s.push_str("and verify:\n");
        for c in &i.commands {
            s.push_str(&format!("  {c}\n"));
        }
    }
    if !i.docs.is_empty() {
        s.push_str("read and keep true:\n");
        for d in &i.docs {
            s.push_str(&format!("  {d}\n"));
        }
    }
    s
}

/// The impact as JSON.
pub fn impact_json(i: &Impact, changed: &[String]) -> Value {
    json!({
        "changed": changed,
        "direct": i.direct,
        "downstream": i.downstream,
        "unowned": i.unowned,
        "cargo": i.cargo_commands(),
        "lib_filters": i.lib_filters,
        "suites": i.suites,
        "verify": i.commands,
        "docs": i.docs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(name: &str, files: &[&str], tests: &[&str], deps: &[&str]) -> Feature {
        Feature {
            name: name.into(),
            summary: format!("{name} summary"),
            files: files.iter().map(|s| s.to_string()).collect(),
            tests: tests.iter().map(|s| s.to_string()).collect(),
            commands: vec![format!("verify {name}")],
            docs: vec![],
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn globs_match_segments_directories_and_double_stars() {
        assert!(glob_match("src/net/server.rs", "src/net/server.rs"));
        assert!(!glob_match("src/net/server.rs", "src/net/client.rs"));
        assert!(glob_match("src/net/*.rs", "src/net/client.rs"));
        assert!(!glob_match("src/net/*.rs", "src/net/deep/x.rs"), "* stays inside one directory");
        assert!(glob_match("src/**", "src/a/b/c.rs"));
        assert!(glob_match("src/**/*.wgsl", "src/shaders/scene.wgsl"));
        assert!(glob_match("docs/adr/", "docs/adr/0001-x.md"), "a trailing slash is a whole directory");
        assert!(glob_match("docs/adr/0029-*.md", "docs/adr/0029-match-flow.md"));
        assert!(glob_match("tests\\net_flow.rs", "tests/net_flow.rs"), "either slash");
    }

    #[test]
    fn a_change_names_its_feature_its_dependents_and_the_exact_commands() {
        let fs = vec![
            f("sim_core", &["src/sim/clock.rs"], &["lib:sim::clock", "alloc_budget"], &[]),
            f("net", &["src/net/**"], &["net_e2e"], &["sim_core"]),
            f("flow", &["src/sim/flow.rs"], &["lib:sim::flow", "net_flow"], &["net"]),
            f("docs", &["README.md"], &["repo_hygiene"], &[]),
        ];
        let i = impact(&fs, &["src/sim/clock.rs".to_string(), "mystery.txt".to_string()]);
        assert_eq!(i.direct.keys().cloned().collect::<Vec<_>>(), vec!["sim_core"]);
        assert_eq!(i.downstream.iter().cloned().collect::<Vec<_>>(), vec!["flow", "net"], "net is built on sim_core, and flow on net");
        assert_eq!(i.unowned, vec!["mystery.txt"]);
        assert_eq!(i.cargo_commands(), vec!["cargo test --lib -- sim::clock sim::flow", "cargo test --test alloc_budget --test net_e2e --test net_flow"]);
        assert!(!i.suites.contains("repo_hygiene"), "an unrelated feature is not dragged in");
        assert!(render_impact(&i, 2).contains("owned by no feature"));
        let quiet = impact(&fs, &["README.md".to_string()]);
        assert!(quiet.downstream.is_empty() && quiet.suites.iter().collect::<Vec<_>>() == vec!["repo_hygiene"]);
    }

    #[test]
    fn parsing_rejects_a_malformed_index_and_find_searches_words() {
        assert!(parse("not json").is_err());
        assert!(parse(r#"{"features":{"x":{"files":[]}}}"#).unwrap_err().contains("summary"));
        assert!(parse(r#"{"features":{"x":{"summary":"s","files":"src"}}}"#).is_err());
        let fs = vec![f("match_flow", &["src/sim/flow.rs"], &[], &[]), f("perf", &["src/tools/perf.rs"], &[], &[])];
        assert_eq!(find(&fs, "flow").len(), 1);
        assert_eq!(find(&fs, "summary").len(), 2);
        assert_eq!(find(&fs, "zzz").len(), 0);
        assert!(render_list(&fs).contains("match_flow") && render_one(&fs[0]).contains("src/sim/flow.rs"));
    }
}
