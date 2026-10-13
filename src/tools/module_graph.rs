//! Module dependency edges read from the source text, and what they answer for the crate split: which modules form rings
//! (`src deps --cycles`) and which could move into `red_core` now (`src deps --core`).
//!
//! An edge is a path a file names through `crate::`, `super::`, `self::`, `red_core::` or `red_engine2::`, groups included (`use crate::{a,
//! b::{c, d}}` names `a`, `b::c` and `b::d`), resolved to the longest known module path of the crate it is in (the core mirrors the engine's
//! paths, so `super::` in a core file names a core module and `red_core::` always does). Comments, string contents and test modules are left
//! out: a test that needs the engine moves to the engine's suites (ADR 2026-10-13-red-core-is-one-crate-the-engine-and-its-games-share). A
//! path through a re-export at a crate root (`crate::Vec3`) names the root, not the module behind it, and is not counted.
use std::collections::{BTreeMap, BTreeSet};

/// What one file names: the modules it uses and the external crates (from the engine's `Cargo.toml`) it names.
#[derive(Debug, Default)]
pub struct FileEdges {
    pub uses: BTreeSet<String>,
    pub crates: BTreeSet<String>,
}

/// One file to read: its path from the repo root, its module name (`sim::clock`), its lines, the 1-based line spans to skip (test modules)
/// and its other inline modules (`mod x { ... }`: first line, last line, name), which `super::` and `self::` inside them start from.
pub struct FileSource<'a> {
    pub rel: &'a str,
    pub module: &'a str,
    pub lines: &'a [String],
    pub skip: &'a [(usize, usize)],
    pub inline: &'a [(usize, usize, String)],
}

/// The module path `crate::` names in a file: `bin::<name>` inside a binary under `src/bin/`, else the library's root (`""`).
fn crate_root(rel: &str) -> String {
    match rel.strip_prefix("src/bin/") {
        Some(r) => format!("bin::{}", r.split('/').next().unwrap_or("").trim_end_matches(".rs")),
        None => String::new(),
    }
}

/// The module path `self::` names at a file's top level: its crate root for a root file (`lib.rs`, `main.rs`, a binary's), else its module.
fn file_path(rel: &str, module: &str) -> String {
    let bin_root = rel.strip_prefix("src/bin/").is_some_and(|r| !r.contains('/') || r.ends_with("/main.rs"));
    if bin_root || matches!(rel, "src/lib.rs" | "src/main.rs" | "crates/red_core/src/lib.rs") {
        crate_root(rel)
    } else {
        module.to_string()
    }
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// A line without its `//` comment and with the contents of its string and char literals removed (so `"crate::x"` in a message is no edge).
pub fn code_only(line: &str) -> String {
    let c: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    let mut in_str = false;
    while i < c.len() {
        if in_str {
            match c[i] {
                '\\' => i += 1,
                '"' => {
                    in_str = false;
                    out.push('"');
                }
                _ => {}
            }
        } else if c[i] == '"' {
            in_str = true;
            out.push('"');
        } else if c[i] == '/' && c.get(i + 1) == Some(&'/') {
            break;
        } else if c[i] == '\'' && c.get(i + 1) == Some(&'\\') {
            // '\n', '\'', '\u{..}': skip to the closing quote
            i += 2;
            while i < c.len() && c[i] != '\'' {
                i += 1;
            }
        } else if c[i] == '\'' && c.get(i + 2) == Some(&'\'') {
            i += 2; // 'x' (a lifetime 'a has no closing quote)
        } else {
            out.push(c[i]);
        }
        i += 1;
    }
    out
}

fn ident(t: &[char], i: &mut usize) -> String {
    let s = *i;
    while *i < t.len() && is_ident(t[*i]) {
        *i += 1;
    }
    t[s..*i].iter().collect()
}

fn skip_ws(t: &[char], i: &mut usize) {
    while *i < t.len() && t[*i].is_whitespace() {
        *i += 1;
    }
}

fn at_path_sep(t: &[char], i: usize) -> bool {
    t.get(i) == Some(&':') && t.get(i + 1) == Some(&':')
}

/// Reads a use tree after `prefix::` (`a::b`, `a::{b, c::{d, self}}`, `a::*`, `a as b`) and pushes every full path it names.
fn parse_tree(t: &[char], i: &mut usize, prefix: &[String], out: &mut Vec<Vec<String>>) {
    let mut segs = prefix.to_vec();
    loop {
        if t.get(*i) == Some(&'{') {
            *i += 1;
            loop {
                skip_ws(t, i);
                match t.get(*i) {
                    None => return,
                    Some('}') => {
                        *i += 1;
                        return;
                    }
                    Some(',') => *i += 1,
                    _ => {
                        let before = *i;
                        parse_tree(t, i, &segs, out);
                        skip_ws(t, i);
                        let mut j = *i;
                        if ident(t, &mut j) == "as" {
                            skip_ws(t, &mut j);
                            ident(t, &mut j);
                            *i = j;
                        }
                        if *i == before {
                            *i += 1; // something this reader does not know: step over it
                        }
                    }
                }
            }
        }
        let id = ident(t, i);
        if id.is_empty() {
            break; // `*`, or the end of the path
        }
        if id != "self" {
            segs.push(id);
        }
        if !at_path_sep(t, *i) {
            break;
        }
        *i += 2;
    }
    out.push(segs);
}

/// Module paths to module names, for one crate: each module's own name, and an inline module of a crate root (`red_core`'s `pub mod sim
/// { ... }`) names that root.
pub type Known = BTreeMap<String, String>;

/// Where paths resolve: `own` for `crate::`, `self::` and `super::` in this file's crate, `core` for `red_core::`, `lib` for `red_engine2::`.
pub struct Resolver<'a> {
    pub own: &'a Known,
    pub core: &'a Known,
    pub lib: &'a Known,
}

/// The module the longest known prefix of a path names.
fn resolve(path: &[String], known: &Known) -> Option<String> {
    (1..=path.len()).rev().find_map(|k| known.get(&path[..k].join("::")).cloned())
}

fn segments(path: &str) -> Vec<String> {
    path.split("::").filter(|s| !s.is_empty()).map(str::to_string).collect()
}

/// What one file uses: `known` resolves its paths, `crates` are the external crates to look for (`wgpu`, `thiserror`, ...).
pub fn file_edges(f: &FileSource, known: &Resolver, crates: &BTreeSet<String>) -> FileEdges {
    let mut text: Vec<char> = Vec::new();
    let mut line_at: Vec<usize> = Vec::new();
    for (k, l) in f.lines.iter().enumerate() {
        let n = k + 1;
        if f.skip.iter().any(|&(a, b)| a <= n && n <= b) {
            continue;
        }
        for c in code_only(l).chars().chain(std::iter::once('\n')) {
            text.push(c);
            line_at.push(n);
        }
    }
    let root = crate_root(f.rel);
    let top = file_path(f.rel, f.module);
    let mut out = FileEdges::default();
    let mut i = 0;
    while i < text.len() {
        let start = i;
        let after_sep = start >= 2 && text[start - 1] == ':' && text[start - 2] == ':';
        if !is_ident(text[i]) || (i > 0 && is_ident(text[i - 1])) {
            i += 1;
            continue;
        }
        let word = ident(&text, &mut i);
        if after_sep || !at_path_sep(&text, i) {
            continue;
        }
        let map = match word.as_str() {
            "red_core" => known.core,
            "red_engine2" => known.lib,
            _ => known.own,
        };
        let base = match word.as_str() {
            "crate" => segments(&root),
            "red_core" | "red_engine2" => Vec::new(),
            "self" | "super" => {
                let n = line_at[start];
                let mut p = segments(&top);
                p.extend(f.inline.iter().filter(|(a, b, _)| *a < n && n <= *b).map(|(_, _, m)| m.clone()));
                if word == "super" {
                    p.pop();
                }
                p
            }
            w => {
                if crates.contains(w) {
                    out.crates.insert(w.to_string());
                }
                continue;
            }
        };
        i += 2;
        let mut base = base;
        if word == "super" {
            loop {
                let mut j = i;
                if ident(&text, &mut j) != "super" || !at_path_sep(&text, j) {
                    break;
                }
                base.pop();
                i = j + 2;
            }
        }
        let mut paths = Vec::new();
        parse_tree(&text, &mut i, &base, &mut paths);
        for p in paths {
            if let Some(m) = resolve(&p, map).filter(|m| m != f.module) {
                out.uses.insert(m);
            }
        }
    }
    out
}

/// The crates a `Cargo.toml` depends on: its `[dependencies]` and target-specific dependency tables, `-` read as `_`.
pub fn cargo_deps(toml: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut on = false;
    for l in toml.lines().map(str::trim) {
        if l.starts_with('[') {
            on = l == "[dependencies]" || (l.starts_with("[target.") && l.ends_with(".dependencies]"));
        } else if on && !l.starts_with('#') {
            if let Some((k, _)) = l.split_once('=') {
                let k = k.split('.').next().unwrap_or("").trim().trim_matches('"');
                if !k.is_empty() && k.chars().all(|c| is_ident(c) || c == '-') {
                    out.insert(k.replace('-', "_"));
                }
            }
        }
    }
    out
}

/// Rings of modules that use one another (each reaches every other through `uses`), largest first; a module in no ring is left out.
pub fn cycles(uses: &BTreeMap<String, BTreeSet<String>>) -> Vec<Vec<String>> {
    // Tarjan's strongly connected components.
    struct T<'a> {
        uses: &'a BTreeMap<String, BTreeSet<String>>,
        index: BTreeMap<&'a str, usize>,
        low: BTreeMap<&'a str, usize>,
        stack: Vec<&'a str>,
        on: BTreeSet<&'a str>,
        out: Vec<Vec<String>>,
    }
    fn visit<'a>(t: &mut T<'a>, v: &'a str) {
        let n = t.index.len();
        t.index.insert(v, n);
        t.low.insert(v, n);
        t.stack.push(v);
        t.on.insert(v);
        let uses = t.uses;
        for w in uses.get(v).into_iter().flatten() {
            let Some((w, _)) = uses.get_key_value(w.as_str()) else { continue };
            let w = w.as_str();
            if !t.index.contains_key(w) {
                visit(t, w);
                let lw = t.low[w];
                let lv = t.low[v];
                t.low.insert(v, lv.min(lw));
            } else if t.on.contains(w) {
                let iw = t.index[w];
                let lv = t.low[v];
                t.low.insert(v, lv.min(iw));
            }
        }
        if t.low[v] == t.index[v] {
            let mut ring = Vec::new();
            while let Some(w) = t.stack.pop() {
                t.on.remove(w);
                ring.push(w.to_string());
                if w == v {
                    break;
                }
            }
            if ring.len() > 1 {
                ring.sort();
                t.out.push(ring);
            }
        }
    }
    let mut t = T { uses, index: BTreeMap::new(), low: BTreeMap::new(), stack: Vec::new(), on: BTreeSet::new(), out: Vec::new() };
    for v in uses.keys() {
        if !t.index.contains_key(v.as_str()) {
            visit(&mut t, v);
        }
    }
    let mut out = t.out;
    out.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
    out
}

/// What some modules reach through `uses`, and the edges by which they reach the modules they should not.
#[derive(Debug, Default, PartialEq)]
pub struct Reach {
    /// Every module the roots reach, the roots included.
    pub reached: BTreeSet<String>,
    /// `(from, to)`: `from` is reached and not outside, `to` is outside.
    pub crossings: Vec<(String, String)>,
    /// What the roots reach once the crossings are cut.
    pub without: BTreeSet<String>,
}

/// Follows `uses` from `roots`; `outside` says which modules the roots should not reach (`src deps --reach A,B --into C,D`).
pub fn reach(uses: &BTreeMap<String, BTreeSet<String>>, roots: &[String], outside: impl Fn(&str) -> bool) -> Reach {
    let walk = |enter: &dyn Fn(&str) -> bool| {
        let mut seen = BTreeSet::new();
        let mut todo: Vec<&str> = roots.iter().map(String::as_str).filter(|m| uses.contains_key(*m)).collect();
        while let Some(m) = todo.pop() {
            if seen.insert(m.to_string()) {
                todo.extend(uses[m].iter().map(String::as_str).filter(|u| uses.contains_key(*u) && enter(u)));
            }
        }
        seen
    };
    let reached = walk(&|_| true);
    let crossings = reached
        .iter()
        .filter(|a| !outside(a))
        .flat_map(|a| uses[a.as_str()].iter().filter(|b| outside(b) && uses.contains_key(b.as_str())).map(move |b| (a.clone(), b.clone())))
        .collect();
    let without = walk(&|m| !outside(m));
    Reach { reached, crossings, without }
}

/// One module as the core frontier sees it.
pub struct Node<'a> {
    pub name: &'a str,
    pub in_core: bool,
    pub uses: &'a BTreeSet<String>,
    pub crates: &'a BTreeSet<String>,
}

/// What could move into `red_core` without cutting an edge.
#[derive(Debug, Default, PartialEq)]
pub struct Frontier {
    /// Round k: modules that use only the core and the rounds before k.
    pub rounds: Vec<Vec<String>>,
    /// Modules whose every module edge is satisfied but that name crates the core does not depend on: (module, crates).
    pub need_crates: Vec<(String, Vec<String>)>,
}

/// Places every module that can move into the core, round by round. `candidates` says which modules may move at all (not a binary, the
/// CLI or a crate root); `core_crates` are the crates `red_core` depends on.
pub fn core_frontier(nodes: &[Node], candidates: impl Fn(&str) -> bool, core_crates: &BTreeSet<String>) -> Frontier {
    let mut placed: BTreeSet<&str> = nodes.iter().filter(|n| n.in_core).map(|n| n.name).collect();
    let mut f = Frontier::default();
    let mut blocked: BTreeSet<&str> = BTreeSet::new();
    loop {
        let mut round = Vec::new();
        for n in nodes {
            if placed.contains(n.name) || blocked.contains(n.name) || !candidates(n.name) {
                continue;
            }
            if n.uses.iter().all(|u| placed.contains(u.as_str())) {
                let missing: Vec<String> = n.crates.iter().filter(|c| !core_crates.contains(*c)).cloned().collect();
                if missing.is_empty() {
                    round.push(n.name);
                } else {
                    blocked.insert(n.name);
                    f.need_crates.push((n.name.to_string(), missing));
                }
            }
        }
        if round.is_empty() {
            break;
        }
        placed.extend(round.iter().copied());
        f.rounds.push(round.into_iter().map(str::to_string).collect());
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(v: &[&str]) -> BTreeSet<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn names(v: &[&str]) -> Known {
        v.iter().map(|s| (s.to_string(), s.to_string())).collect()
    }

    fn edges(rel: &str, module: &str, src: &str, inline: &[(usize, usize, String)], skip: &[(usize, usize)]) -> FileEdges {
        let lines: Vec<String> = src.lines().map(str::to_string).collect();
        let known = names(&["a", "b", "b::c", "b::d", "sim", "sim::clock", "sim::scratch", "net", "net::auth", "bin::re2::frame", "x"]);
        file_edges(&FileSource { rel, module, lines: &lines, skip, inline }, &Resolver { own: &known, core: &known, lib: &known }, &set(&["wgpu", "thiserror"]))
    }

    #[test]
    fn grouped_nested_and_renamed_imports_are_all_edges() {
        let e = edges("src/x.rs", "x", "use crate::{a, b::{c, d as dd, self}};\nuse crate::sim::*;\nfn f() { crate::net::auth::check(); }\n", &[], &[]);
        assert_eq!(e.uses, set(&["a", "b", "b::c", "b::d", "sim", "net::auth"]));
        let e = edges("src/x.rs", "x", "use crate::{\n    a,\n    sim::{\n        clock::Clock,\n        scratch,\n    },\n};\n", &[], &[]);
        assert_eq!(e.uses, set(&["a", "sim::clock", "sim::scratch"]), "a group may span lines");
    }

    #[test]
    fn super_and_self_resolve_from_the_file_and_its_inline_modules() {
        let e = edges("src/sim/clock.rs", "sim::clock", "use super::scratch::Scratch;\nfn f() { super::super::a::g(); }\n", &[], &[]);
        assert_eq!(e.uses, set(&["sim::scratch", "a"]));
        let e = edges("src/sim/mod.rs", "sim", "pub use self::clock::Clock;\nuse super::net::auth;\n", &[], &[]);
        assert_eq!(e.uses, set(&["sim::clock", "net::auth"]), "in mod.rs, self is the module and super its parent");
        let e =
            edges("crates/red_core/src/lib.rs", "red_core", "pub mod sim {\n    use super::a;\n    use self::clock::Tick;\n}\n", &[(1, 4, "sim".into())], &[]);
        assert_eq!(e.uses, set(&["a", "sim::clock"]), "an inline module is where super and self start");
    }

    #[test]
    fn comments_strings_tests_and_other_paths_are_not_edges() {
        let src = "// use crate::a;\nconst M: &str = \"crate::b\";\nlet q = '\"'; use crate::sim::clock;\nfn g() { std::crate_x::a(); }\n#[cfg(test)]\nmod tests {\n    use crate::net;\n}\n";
        let e = edges("src/x.rs", "x", src, &[], &[(6, 8)]);
        assert_eq!(e.uses, set(&["sim::clock"]));
        let e = edges("src/x.rs", "x", "use wgpu::Device;\nfn f() -> thiserror::Error { foo::wgpu::x() }\n", &[], &[]);
        assert_eq!(e.crates, set(&["wgpu", "thiserror"]), "a crate named mid-path is not that crate");
    }

    #[test]
    fn a_core_file_resolves_in_the_core_although_the_engine_has_the_same_names() {
        let engine = names(&["net", "net::join_code", "net::auth"]);
        let mut core = names(&["net::join_code", "net::auth"]);
        core.insert("net".into(), "red_core".into()); // `pub mod net { ... }` inline in red_core's lib.rs
        let lines: Vec<String> = ["use super::DEFAULT_PORT;", "use super::auth::Key;"].map(String::from).to_vec();
        let f = FileSource { rel: "crates/red_core/src/net/join_code.rs", module: "net::join_code", lines: &lines, skip: &[], inline: &[] };
        let e = file_edges(&f, &Resolver { own: &core, core: &core, lib: &engine }, &BTreeSet::new());
        assert_eq!(e.uses, set(&["red_core", "net::auth"]));
        let lines: Vec<String> = ["pub use red_core::net::DEFAULT_PORT;"].map(String::from).to_vec();
        let f = FileSource { rel: "src/net/mod.rs", module: "net", lines: &lines, skip: &[], inline: &[] };
        assert_eq!(file_edges(&f, &Resolver { own: &engine, core: &core, lib: &engine }, &BTreeSet::new()).uses, set(&["red_core"]));
    }

    #[test]
    fn a_binary_names_its_own_crate_and_the_library_by_name() {
        let e = edges("src/bin/re2/main.rs", "bin::re2::main", "mod frame;\nuse crate::frame::draw;\nuse red_engine2::sim::clock;\n", &[], &[]);
        assert_eq!(e.uses, set(&["bin::re2::frame", "sim::clock"]));
    }

    #[test]
    fn rings_are_found_largest_first_and_a_lone_module_is_none() {
        let mut g: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (k, v) in [("a", &["b"][..]), ("b", &["c"]), ("c", &["a", "d"]), ("d", &[]), ("e", &["f"]), ("f", &["e"]), ("g", &["a"])] {
            g.insert(k.into(), set(v));
        }
        assert_eq!(cycles(&g), vec![vec!["a".to_string(), "b".into(), "c".into()], vec!["e".to_string(), "f".into()]]);
    }

    #[test]
    fn the_frontier_moves_in_rounds_and_stops_at_a_missing_crate() {
        let none = BTreeSet::new();
        let (core, u_a, u_b, u_c, u_d) = (set(&[]), set(&["core1"]), set(&["a"]), set(&["b", "gfx"]), set(&["core1"]));
        let wgpu = set(&["wgpu"]);
        let nodes = [
            Node { name: "core1", in_core: true, uses: &core, crates: &none },
            Node { name: "a", in_core: false, uses: &u_a, crates: &none },
            Node { name: "b", in_core: false, uses: &u_b, crates: &none },
            Node { name: "c", in_core: false, uses: &u_c, crates: &none },
            Node { name: "d", in_core: false, uses: &u_d, crates: &wgpu },
            Node { name: "gfx", in_core: false, uses: &u_d, crates: &none },
        ];
        let f = core_frontier(&nodes, |m| m != "gfx", &set(&["glam"]));
        assert_eq!(f.rounds, vec![vec!["a".to_string()], vec!["b".to_string()]]);
        assert_eq!(f.need_crates, vec![("d".to_string(), vec!["wgpu".to_string()])]);
    }

    #[test]
    fn reach_lists_the_edges_into_the_outside_and_what_is_left_without_them() {
        let mut g: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (k, v) in [("srv", &["sim", "ui"][..]), ("sim", &["geo", "tools::x"]), ("geo", &[]), ("ui", &["geo"]), ("tools::x", &["sim"]), ("lone", &[])] {
            g.insert(k.into(), set(v));
        }
        let r = reach(&g, &["srv".to_string()], |m| m == "ui" || m.starts_with("tools::"));
        assert_eq!(r.reached, set(&["srv", "sim", "geo", "ui", "tools::x"]));
        assert_eq!(r.crossings, vec![("sim".to_string(), "tools::x".to_string()), ("srv".to_string(), "ui".to_string())]);
        assert_eq!(r.without, set(&["srv", "sim", "geo"]));
    }

    #[test]
    fn cargo_dependencies_include_target_tables_and_skip_the_others() {
        let toml = "[package]\nname = \"x\"\n[dependencies]\nserde = { workspace = true }\nffmpeg-sidecar = \"2\"\n# wgpu = \"1\"\nglam.workspace = true\n[target.'cfg(windows)'.dependencies]\nwindows-sys = \"0.59\"\n[dev-dependencies]\ntempfile = \"3\"\n[features]\ngfx = [\"dep:wgpu\"]\n";
        assert_eq!(cargo_deps(toml), set(&["ffmpeg_sidecar", "glam", "serde", "windows_sys"]));
    }
}
