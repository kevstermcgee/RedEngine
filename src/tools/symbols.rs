//! `red_engine2 src ...`: explore the engine's own Rust without reading it. A dependency-free
//! scanner (no rust-analyzer, no database to go stale — 12k lines scan in a few milliseconds)
//! builds the symbol table on demand and answers the questions an AI would otherwise spend
//! repository-wide greps on:
//!
//! * `src map`            every module: one-line purpose, size, pub items, what it depends on
//! * `src find <words>`   where is X defined? ranked, with signature + first doc line
//! * `src outline <file>` the items of one file (pub by default)
//! * `src show <symbol>`  just that item's source (bounded), not the whole file
//! * `src refs <symbol>`  who uses it, grouped by file, with the enclosing function
//! * `src deps [module]`  module dependency edges (uses / used by)
//! * `src coverage`       public items still missing a `///` doc (what find/show/search print)
//!
//! The scanner is line-based: it understands `fn/struct/enum/trait/type/const/static/mod/impl`,
//! doc comments, and brace-matched item extents. It is deliberately approximate (macros, exotic
//! formatting) — good enough to *locate* code; then `src show` or a targeted read finishes the job.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// One scanned Rust item: file, line span, kind, name, visibility, signature and doc comment.
#[derive(Debug, Clone)]
pub struct Symbol {
    pub file: String,
    pub line: usize,
    pub end: usize,
    pub kind: &'static str,
    pub name: String,
    pub container: Option<String>,
    pub public: bool,
    pub in_tests: bool,
    pub sig: String,
    pub doc: String,
}

impl Symbol {
    /// `Container::name` for methods, else the bare name.
    pub fn qualified(&self) -> String {
        match &self.container {
            Some(c) => format!("{c}::{}", self.name),
            None => self.name.clone(),
        }
    }
}

/// A scanned source file: relative path and its lines.
pub struct SourceFile {
    pub rel: String,
    pub lines: Vec<String>,
}

/// The on-demand source index: every file and symbol (built fresh each run, so it cannot go stale).
pub struct Index {
    pub root: PathBuf,
    pub files: Vec<SourceFile>,
    pub symbols: Vec<Symbol>,
}

/// Finds the crate root: `$RE2_SRC`, else the manifest dir this binary was built from, else the
/// current directory or a parent of it.
pub fn find_root() -> Option<PathBuf> {
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
    cands.into_iter().find(|c| c.join("src").join("lib.rs").is_file())
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            collect_rs(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

impl Index {
    /// Scans the tree under `root`.
    pub fn build(root: &Path) -> Index {
        let mut paths = Vec::new();
        collect_rs(&root.join("src"), &mut paths);
        collect_rs(&root.join("tests"), &mut paths);
        let mut files = Vec::new();
        let mut symbols = Vec::new();
        for p in paths {
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            let lines: Vec<String> = text.lines().map(str::to_string).collect();
            scan(&rel, &lines, &mut symbols);
            files.push(SourceFile { rel, lines });
        }
        Index { root: root.to_path_buf(), files, symbols }
    }

    /// A scanned file by relative path.
    pub fn file(&self, rel: &str) -> Option<&SourceFile> {
        self.files.iter().find(|f| f.rel == rel)
    }
}

/// Net brace depth change of `line`, ignoring braces inside strings, chars and `//` comments.
fn brace_delta(line: &str) -> i32 {
    let b: Vec<char> = line.chars().collect();
    let (mut d, mut i) = (0i32, 0usize);
    let mut in_str = false;
    while i < b.len() {
        let c = b[i];
        if in_str {
            if c == '\\' {
                i += 1;
            } else if c == '"' {
                in_str = false;
            }
        } else {
            match c {
                '"' => in_str = true,
                '/' if b.get(i + 1) == Some(&'/') => break,
                '\'' => {
                    // char literal like '{' or '\n' (skip), but not a lifetime like 'a
                    if b.get(i + 2) == Some(&'\'') {
                        i += 2;
                    } else if b.get(i + 1) == Some(&'\\') && b.get(i + 3) == Some(&'\'') {
                        i += 3;
                    }
                }
                '{' => d += 1,
                '}' => d -= 1,
                _ => {}
            }
        }
        i += 1;
    }
    d
}

fn ident_after<'a>(s: &'a str, kw: &str) -> Option<&'a str> {
    let rest = s.strip_prefix(kw)?.trim_start();
    let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

fn scan(rel: &str, lines: &[String], out: &mut Vec<Symbol>) {
    // (container name, end line) for impl/mod/trait blocks we are inside
    let mut stack: Vec<(String, usize, bool)> = Vec::new();
    for (i, raw) in lines.iter().enumerate() {
        let n = i + 1;
        while stack.last().is_some_and(|(_, end, _)| *end < n) {
            stack.pop();
        }
        let t = raw.trim_start();
        let mut s = t;
        let mut public = false;
        if let Some(r) = s.strip_prefix("pub(crate) ").or_else(|| s.strip_prefix("pub(super) ")) {
            s = r;
            public = false;
        } else if let Some(r) = s.strip_prefix("pub ") {
            s = r;
            public = true;
        }
        for q in ["async ", "unsafe ", "const fn ", "extern \"C\" "] {
            if let Some(r) = s.strip_prefix(q) {
                s = if q == "const fn " { &t[t.find("fn ").unwrap_or(0)..] } else { r };
            }
        }
        let (kind, name): (&'static str, String) = if let Some(nm) = ident_after(s, "fn ") {
            ("fn", nm.to_string())
        } else if let Some(nm) = ident_after(s, "struct ") {
            ("struct", nm.to_string())
        } else if let Some(nm) = ident_after(s, "enum ") {
            ("enum", nm.to_string())
        } else if let Some(nm) = ident_after(s, "trait ") {
            ("trait", nm.to_string())
        } else if let Some(nm) = ident_after(s, "type ") {
            ("type", nm.to_string())
        } else if let Some(nm) = ident_after(s, "const ").filter(|_| !s.starts_with("const fn")) {
            ("const", nm.to_string())
        } else if let Some(nm) = ident_after(s, "static ") {
            ("static", nm.to_string())
        } else if let Some(nm) = ident_after(s, "mod ") {
            ("mod", nm.to_string())
        } else if s.starts_with("impl") && (s[4..].starts_with('<') || s[4..].starts_with(' ')) {
            let head = s[4..].split('{').next().unwrap_or("").trim();
            let head = if head.starts_with('<') { head.split_once('>').map(|x| x.1).unwrap_or(head).trim() } else { head };
            let target = head.split(" for ").last().unwrap_or(head).split(" where").next().unwrap_or(head).trim();
            let ty: String = target.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            ("impl", if ty.is_empty() { head.to_string() } else { ty })
        } else {
            continue;
        };
        if kind == "mod" && s.trim_end().ends_with(';') {
            continue; // `mod foo;` declaration
        }
        // extent
        let mut depth = 0i32;
        let mut end = n;
        let mut opened = false;
        for (j, l) in lines.iter().enumerate().skip(i) {
            let head = if j == i { t } else { l.as_str() };
            let d = brace_delta(head);
            if head.contains('{') {
                opened = true;
            }
            depth += d;
            end = j + 1;
            if !opened && head.trim_end().ends_with(';') {
                break;
            }
            if opened && depth <= 0 {
                break;
            }
            if j - i > 4000 {
                break;
            }
        }
        // doc comment: contiguous `///` lines above (skipping attributes)
        let mut doc = String::new();
        let mut k = i;
        while k > 0 {
            let p = lines[k - 1].trim();
            if p.starts_with("#[") || p.starts_with("#!") {
                k -= 1;
            } else if let Some(d) = p.strip_prefix("///") {
                doc = d.trim().to_string(); // keep walking up; the top-most line wins
                k -= 1;
            } else {
                break;
            }
        }
        let mut sig = t.to_string();
        let mut j = i;
        while !sig.contains('{') && !sig.trim_end().ends_with(';') && j + 1 < lines.len() && j - i < 4 {
            j += 1;
            sig.push(' ');
            sig.push_str(lines[j].trim());
        }
        let sig: String = sig.split('{').next().unwrap_or("").trim().trim_end_matches(';').split_whitespace().collect::<Vec<_>>().join(" ");
        let sig = if sig.chars().count() > 150 { format!("{}…", sig.chars().take(150).collect::<String>()) } else { sig };
        let container = stack.iter().rev().find(|(_, _, is_type)| *is_type).map(|(c, _, _)| c.clone());
        let in_tests = stack.iter().any(|(c, _, _)| c == "tests" || c.ends_with("_tests"))
            || rel.starts_with("tests/")
            || (kind == "mod" && (name == "tests" || name.ends_with("_tests")));
        if matches!(kind, "impl" | "mod" | "trait") {
            stack.push((name.clone(), end, kind != "mod"));
        }
        out.push(Symbol { file: rel.to_string(), line: n, end, kind, name, container, public, in_tests, sig, doc });
    }
}

// ---------------------------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------------------------

fn words(q: &str) -> Vec<String> {
    q.to_lowercase().split(|c: char| c.is_whitespace() || c == ':').filter(|w| !w.is_empty()).map(str::to_string).collect()
}

fn score(s: &Symbol, ws: &[String]) -> u32 {
    let name = s.name.to_lowercase();
    let cont = s.container.as_deref().unwrap_or("").to_lowercase();
    let file = s.file.to_lowercase();
    let doc = s.doc.to_lowercase();
    let sig = s.sig.to_lowercase();
    let mut total: u32 = 0;
    for w in ws {
        let sc: u32 = if name == *w {
            100
        } else if name.starts_with(w.as_str()) {
            60
        } else if name.contains(w.as_str()) {
            40
        } else if cont == *w {
            30
        } else if cont.contains(w.as_str()) || file.contains(w.as_str()) {
            15
        } else if doc.contains(w.as_str()) {
            10
        } else if sig.contains(w.as_str()) {
            5
        } else {
            return 0;
        };
        total += sc;
    }
    if s.public {
        total += 3;
    }
    if matches!(s.kind, "impl" | "mod") {
        total = total.saturating_sub(20);
    }
    total
}

/// Ranked symbol search by name/signature/doc words, optionally restricted by kind or file.
pub fn find<'a>(ix: &'a Index, query: &str, kind: Option<&str>, file: Option<&str>, tests: bool, limit: usize) -> Vec<&'a Symbol> {
    let ws = words(query);
    let mut hits: Vec<(u32, &Symbol)> = ix
        .symbols
        .iter()
        .filter(|s| tests || !s.in_tests)
        .filter(|s| kind.is_none_or(|k| s.kind == k))
        .filter(|s| file.is_none_or(|f| s.file.contains(f)))
        .map(|s| (score(s, &ws), s))
        .filter(|(sc, _)| *sc > 0)
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.file.cmp(&b.1.file)).then(a.1.line.cmp(&b.1.line)));
    hits.into_iter().take(limit).map(|(_, s)| s).collect()
}

/// Formats symbols as `file:start-end  signature` plus the doc line.
pub fn render_symbols(rows: &[&Symbol]) -> String {
    if rows.is_empty() {
        return "no symbols match (try fewer words, or `src map` for the module overview)\n".to_string();
    }
    let mut out = String::new();
    for s in rows {
        out.push_str(&format!("{}:{}-{}  {}\n", s.file, s.line, s.end, s.sig));
        if !s.doc.is_empty() {
            out.push_str(&format!("    {}\n", s.doc));
        }
    }
    out
}

/// Lists the items of one file (public only unless `all`).
pub fn outline(ix: &Index, file: &str, all: bool) -> Result<String, String> {
    let matches: Vec<&SourceFile> = ix.files.iter().filter(|f| f.rel == file || f.rel.ends_with(file) || f.rel.contains(file)).collect();
    let f = match matches.as_slice() {
        [] => return Err(format!("no source file matches '{file}' (try `src map`)")),
        [f] => *f,
        many => many
            .iter()
            .find(|f| f.rel.ends_with(&format!("/{file}")) || f.rel.ends_with(&format!("/{file}.rs")))
            .copied()
            .ok_or_else(|| format!("'{file}' is ambiguous: {}", many.iter().map(|f| f.rel.as_str()).collect::<Vec<_>>().join(", ")))?,
    };
    let mut out = format!("{} ({} lines)\n", f.rel, f.lines.len());
    for s in ix.symbols.iter().filter(|s| s.file == f.rel && (all || (s.public && !s.in_tests) || s.kind == "impl") && !(s.kind == "mod" && s.name == "tests"))
    {
        let indent = if s.container.is_some() { "    " } else { "" };
        if s.kind == "impl" {
            if all {
                out.push_str(&format!("  {}:{}  impl {}\n", s.line, s.end, s.name));
            }
            continue;
        }
        out.push_str(&format!("  {}{:>5}  {}\n", indent, s.line, s.sig));
    }
    if !all {
        out.push_str("  (pub items only; --all for everything)\n");
    }
    Ok(out)
}

/// Prints one item's source, truncated to `max_lines`.
pub fn show(ix: &Index, query: &str, max_lines: usize) -> Result<String, String> {
    let ws = words(query);
    let hits = find(ix, query, None, None, true, 8);
    // Prefer an exact qualified-name / name match.
    let exact: Vec<&&Symbol> =
        hits.iter().filter(|s| ws.iter().all(|w| s.name.to_lowercase() == *w || s.container.as_deref().is_some_and(|c| c.to_lowercase() == *w))).collect();
    let pick: &Symbol = match (exact.as_slice(), hits.as_slice()) {
        ([one], _) => one,
        ([first, ..], _) => {
            let others: Vec<String> = exact.iter().skip(1).map(|s| format!("{}:{}", s.file, s.line)).collect();
            let mut msg = String::new();
            msg.push_str(&format!("(also defined at {})\n", others.join(", ")));
            return show_symbol(ix, first, max_lines).map(|b| msg + &b);
        }
        (_, [first, ..]) => first,
        _ => return Err(format!("no symbol matches '{query}' (try `src find {query}`)")),
    };
    show_symbol(ix, pick, max_lines)
}

fn show_symbol(ix: &Index, s: &Symbol, max_lines: usize) -> Result<String, String> {
    let f = ix.file(&s.file).ok_or("file vanished")?;
    let end = s.end.min(s.line + max_lines - 1);
    let mut out = format!(
        "{}:{}-{}{}\n",
        s.file,
        s.line,
        s.end,
        if end < s.end { format!("  (showing {max_lines} of {} lines; --lines N for more)", s.end - s.line + 1) } else { String::new() }
    );
    for l in s.line..=end {
        out.push_str(&format!("{l:>5}  {}\n", f.lines[l - 1]));
    }
    Ok(out)
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Word-boundary occurrences of `name` outside comments, with the innermost enclosing fn.
pub fn refs(ix: &Index, name: &str, limit: usize) -> String {
    let defs: BTreeSet<(String, usize)> = ix.symbols.iter().filter(|s| s.name == name).map(|s| (s.file.clone(), s.line)).collect();
    let mut by_file: BTreeMap<&str, Vec<(usize, String, String)>> = BTreeMap::new();
    let mut total = 0;
    for f in &ix.files {
        for (i, l) in f.lines.iter().enumerate() {
            let code = l.split("//").next().unwrap_or("");
            let mut from = 0;
            while let Some(p) = code[from..].find(name) {
                let a = from + p;
                let b = a + name.len();
                let before = code[..a].chars().next_back();
                let after = code[b..].chars().next();
                from = b;
                if before.is_some_and(is_ident) || after.is_some_and(is_ident) {
                    continue;
                }
                if defs.contains(&(f.rel.clone(), i + 1)) {
                    break;
                }
                let encl = ix
                    .symbols
                    .iter()
                    .filter(|s| s.file == f.rel && s.kind == "fn" && s.line <= i + 1 && i < s.end)
                    .min_by_key(|s| s.end - s.line)
                    .map(|s| s.qualified())
                    .unwrap_or_default();
                by_file.entry(f.rel.as_str()).or_default().push((i + 1, l.trim().chars().take(110).collect(), encl));
                total += 1;
                break;
            }
        }
    }
    if total == 0 {
        return format!("no references to `{name}` outside its definition\n");
    }
    let mut out = format!("{total} reference(s) to `{name}` in {} file(s):\n", by_file.len());
    let mut shown = 0;
    for (file, rows) in &by_file {
        out.push_str(&format!("{file}\n"));
        for (n, text, encl) in rows {
            if shown >= limit {
                out.push_str("  … (more; raise --limit)\n");
                return out;
            }
            out.push_str(&format!("  {n:>5}  {text}{}\n", if encl.is_empty() { String::new() } else { format!("   [in {encl}]") }));
            shown += 1;
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Module map + dependencies
// ---------------------------------------------------------------------------------------------

/// `src/tools/lint.rs` -> `tools::lint`; `src/lib.rs` -> `lib`; `src/bin/re2.rs` -> `bin::re2`.
fn module_of(rel: &str) -> String {
    let p = rel.trim_start_matches("src/").trim_end_matches(".rs");
    let p = p.strip_suffix("/mod").unwrap_or(p);
    p.replace('/', "::")
}

/// One module's summary row for `src map`: name, size, purpose and dependencies.
pub struct ModInfo {
    pub name: String,
    pub file: String,
    pub lines: usize,
    pub purpose: String,
    pub pub_items: usize,
    pub uses: BTreeSet<String>,
}

/// Summarizes every module (purpose from its `//!` header).
pub fn modules(ix: &Index) -> Vec<ModInfo> {
    let known: BTreeSet<String> = ix.files.iter().filter(|f| f.rel.starts_with("src/")).map(|f| module_of(&f.rel)).collect();
    let mut out = Vec::new();
    for f in ix.files.iter().filter(|f| f.rel.starts_with("src/")) {
        let name = module_of(&f.rel);
        let purpose = f
            .lines
            .iter()
            .take_while(|l| l.trim_start().starts_with("//!") || l.trim().is_empty())
            .find_map(|l| l.trim_start().strip_prefix("//!").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()))
            .unwrap_or_default();
        let mut uses = BTreeSet::new();
        let in_tools = name.starts_with("tools::");
        for l in &f.lines {
            let code = l.split("//").next().unwrap_or("");
            let mut rest = code;
            while let Some(p) = rest.find("crate::") {
                rest = &rest[p + 7..];
                let seg: Vec<&str> =
                    rest.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':')).next().unwrap_or("").split("::").filter(|s| !s.is_empty()).collect();
                if let Some(first) = seg.first() {
                    let cand2 = seg.get(1).map(|s| format!("{first}::{s}"));
                    let m = cand2.filter(|c| known.contains(c)).unwrap_or_else(|| first.to_string());
                    if known.contains(&m) && m != name {
                        uses.insert(m);
                    }
                }
            }
            if in_tools {
                if let Some(r) = code.trim_start().strip_prefix("use super::") {
                    let first: String = r.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                    let m = format!("tools::{first}");
                    if known.contains(&m) && m != name {
                        uses.insert(m);
                    }
                }
            }
        }
        let pub_items = ix.symbols.iter().filter(|s| s.file == f.rel && s.public && !s.in_tests && s.container.is_none()).count();
        out.push(ModInfo { name, file: f.rel.clone(), lines: f.lines.len(), purpose, pub_items, uses });
    }
    out
}

/// Renders `src map`.
pub fn render_map(ix: &Index) -> String {
    let mods = modules(ix);
    let mut out =
        format!("{} source files, {} symbols. `src outline <mod>` lists a file, `src show <symbol>` prints one item.\n\n", ix.files.len(), ix.symbols.len());
    let w = mods.iter().map(|m| m.name.len()).max().unwrap_or(8);
    for m in &mods {
        out.push_str(&format!("{:<w$} {:>5}L {:>3}pub  {}\n", m.name, m.lines, m.pub_items, m.purpose.chars().take(96).collect::<String>(), w = w));
    }
    out
}

/// Renders `src deps` for all modules or one.
pub fn render_deps(ix: &Index, module: Option<&str>) -> Result<String, String> {
    let mods = modules(ix);
    let mut used_by: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for m in &mods {
        for u in &m.uses {
            used_by.entry(u.as_str()).or_default().insert(m.name.as_str());
        }
    }
    let mut out = String::new();
    let sel: Vec<&ModInfo> = match module {
        Some(q) => {
            let v: Vec<&ModInfo> = mods.iter().filter(|m| m.name == q || m.name.ends_with(&format!("::{q}"))).collect();
            if v.is_empty() {
                return Err(format!("no module '{q}' (try `src map`)"));
            }
            v
        }
        None => mods.iter().collect(),
    };
    for m in sel {
        let ub: Vec<&str> = used_by.get(m.name.as_str()).map(|s| s.iter().copied().collect()).unwrap_or_default();
        out.push_str(&format!(
            "{}\n    uses:    {}\n    used by: {}\n",
            m.name,
            if m.uses.is_empty() { "-".to_string() } else { m.uses.iter().cloned().collect::<Vec<_>>().join(", ") },
            if ub.is_empty() { "-".to_string() } else { ub.join(", ") }
        ));
    }
    Ok(out)
}

/// Doc-comment coverage of public items (`src coverage`): the total, per-file counts for files with
/// gaps, and the undocumented items themselves (up to `limit`). `///` docs are what `src find/show`
/// and `search` show instead of source, so a public item without one is invisible to an AI.
pub fn coverage(ix: &Index, file: Option<&str>, limit: usize) -> String {
    let items: Vec<&Symbol> =
        ix.symbols.iter().filter(|s| s.public && !s.in_tests && !matches!(s.kind, "impl" | "mod") && file.is_none_or(|f| s.file.contains(f))).collect();
    let missing: Vec<&Symbol> = items.iter().copied().filter(|s| s.doc.trim().is_empty()).collect();
    let pct = if items.is_empty() { 100.0 } else { 100.0 * (items.len() - missing.len()) as f32 / items.len() as f32 };
    let mut out = format!("{} of {} public items documented ({pct:.0}%).\n", items.len() - missing.len(), items.len());
    let mut per_file: BTreeMap<&str, usize> = BTreeMap::new();
    for s in &missing {
        *per_file.entry(s.file.as_str()).or_default() += 1;
    }
    if !per_file.is_empty() {
        out.push_str("\nundocumented per file:\n");
        for (f, n) in &per_file {
            out.push_str(&format!("  {n:>3}  {f}\n"));
        }
        out.push_str("\nitems (add a `///` line above each):\n");
        for s in missing.iter().take(limit) {
            out.push_str(&format!("  {}:{}  {}\n", s.file, s.line, s.sig));
        }
        if missing.len() > limit {
            out.push_str(&format!("  ... {} more (raise --limit, or narrow with --file)\n", missing.len() - limit));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym(src: &str) -> Vec<Symbol> {
        let lines: Vec<String> = src.lines().map(str::to_string).collect();
        let mut out = Vec::new();
        scan("src/x.rs", &lines, &mut out);
        out
    }

    #[test]
    fn scanner_finds_items_docs_containers_and_extents() {
        let s = sym("/// Adds numbers.\n/// second line\npub fn add(a: i32,\n    b: i32) -> i32 {\n    a + b\n}\n\nstruct P { x: f32 }\n\nimpl P {\n    pub fn new() -> P { P { x: 0.0 } }\n    fn hidden(&self) {\n        let _s = \"}{\";\n    }\n}\n");
        let add = s.iter().find(|x| x.name == "add").unwrap();
        assert!(add.public && add.line == 3 && add.end == 6, "{add:?}");
        assert_eq!(add.doc, "Adds numbers.");
        assert!(add.sig.contains("b: i32) -> i32"), "{}", add.sig);
        let new = s.iter().find(|x| x.name == "new").unwrap();
        assert_eq!(new.container.as_deref(), Some("P"));
        let hidden = s.iter().find(|x| x.name == "hidden").unwrap();
        assert_eq!((hidden.line, hidden.end), (12, 14), "string braces must not confuse extents: {hidden:?}");
        assert!(!hidden.public);
    }

    #[test]
    fn test_modules_are_flagged() {
        let s = sym("fn a() {}\n#[cfg(test)]\nmod tests {\n    fn t() {}\n}\n");
        assert!(s.iter().find(|x| x.name == "t").unwrap().in_tests);
        assert!(!s.iter().find(|x| x.name == "a").unwrap().in_tests);
    }

    #[test]
    fn the_real_repo_indexes_and_answers_queries() {
        let root = find_root().expect("crate root");
        let ix = Index::build(&root);
        assert!(ix.symbols.len() > 500, "expected a real symbol table, got {}", ix.symbols.len());
        let hit = find(&ix, "lint", Some("fn"), Some("tools/lint"), false, 5);
        assert!(hit.iter().any(|s| s.name == "lint" && s.file == "src/tools/lint.rs"), "{hit:?}");
        let shown = show(&ix, "ground_height_at", 12).unwrap();
        assert!(shown.contains("fn ground_height_at"), "{shown}");
        let r = refs(&ix, "step_horizontal", 20);
        assert!(r.contains("src/tools/walk.rs"), "{r}");
        let deps = render_deps(&ix, Some("tools::lint")).unwrap();
        assert!(deps.contains("tools::world"), "{deps}");
        assert!(render_map(&ix).contains("tools::verify"));
    }
}
