//! What the front-door documents claim about the repository, checked against the repository.
//!
//! ADR 0007/0025 keep derived numbers current (`<!--fact:...-->`) and ban hand-written test counts. A review (2026-10-09) found the claims a
//! reader acts on that went stale anyway: a protocol version written as a number, a feature still described as missing after it was built, a
//! file's size, an MCP tool that does not exist, a command that needs a cargo feature the default build lacks. Each check here is a pure
//! function from the repository to sentences; `preflight` prints them with the edit to make and `tests/docs_fresh.rs` fails on them.

use super::status;
use std::path::Path;

/// The documents whose claims are checked: what an outsider or an agent opens first, and the two long references that repeat facts about the build.
pub const CLAIM_DOCS: &[&str] =
    &["README.md", "AGENTS.md", "CLAUDE.md", "docs/AGENT_REFERENCE.md", "docs/ENGINE_OVERVIEW.md", "docs/VIEWER_HISTORY.md", "docs/HOSTING.md", "SPEC.md"];

fn read(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel)).unwrap_or_default().replace("\r\n", "\n")
}

/// `text` without the values inside `<!--fact:name-->...<!--/fact-->` (those are checked against the code by the inline-fact check itself).
fn without_facts(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find(status::INLINE_OPEN) {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        match tail.find(status::INLINE_CLOSE) {
            Some(end) => rest = &tail[end + status::INLINE_CLOSE.len()..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// A protocol version written as a plain number that is not the current one (`protocol v5` while the code says 15). "protocol v7 and later" says when something
/// began and stays true, so it is allowed; a current version should be written as `<!--fact:protocol-->N<!--/fact-->`, which `preflight --fix` keeps right.
pub fn protocol_claims(root: &Path) -> Vec<String> {
    let Some(current) = status::protocol_version(root) else { return Vec::new() };
    let mut out = Vec::new();
    for doc in CLAIM_DOCS {
        let text = without_facts(&read(root, doc));
        let lower = text.to_lowercase();
        let mut from = 0;
        while let Some(i) = lower[from..].find("protocol v") {
            let at = from + i + "protocol v".len();
            let digits: String = lower[at..].chars().take_while(char::is_ascii_digit).collect();
            from = at;
            let Ok(n) = digits.parse::<u32>() else { continue };
            let after = lower[at + digits.len()..].trim_start();
            if n == current || after.starts_with("and later") || after.starts_with("or later") {
                continue;
            }
            out.push(format!(
                "{doc}: says `protocol v{n}`, src/net/protocol.rs has v{current}; write the current version as <!--fact:protocol-->{current}<!--/fact--> (or \"vN and later\" for when something began)"
            ));
        }
    }
    out
}

/// A sentence the code has disproved: `(file, text that is in the file while the feature exists, phrases that say it is missing, what is true)`.
const DISPROVED: &[(&str, &str, &[&str], &str)] = &[
    (
        "src/sim/match_sim.rs",
        "pub const HISTORY_TICKS",
        &["no lag compensation", "have: lag compensation", "lag compensation for hitscan yet", "without lag compensation", "lacks lag compensation"],
        "lag compensation is built (`HISTORY_TICKS` in src/sim/match_sim.rs, ADR 0053)",
    ),
    (
        "AGENTS.md",
        "Do not open SPEC.md",
        &["written to be read once", "read once and used directly"],
        "AGENTS.md says never to open SPEC.md whole (use `search`)",
    ),
];

/// Documents that still say a feature is missing while the code that implements it is in the tree.
pub fn disproved_claims(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for (file, marker, phrases, truth) in DISPROVED {
        if !read(root, file).contains(marker) {
            continue;
        }
        for doc in CLAIM_DOCS {
            let lower = read(root, doc).to_lowercase();
            for p in *phrases {
                if lower.contains(p) {
                    out.push(format!("{doc}: still says `{p}`; {truth}"));
                }
            }
        }
    }
    out
}

/// The document name written immediately before `at` on its line (`CLAUDE.md`, `` `docs/HOSTING.md` ``, or a markdown link to one), if there is one.
fn md_name_before(line: &str, at: usize) -> Option<String> {
    let mut head = line[..at].trim_end().trim_end_matches('`').trim_end();
    if head.ends_with(')') {
        // `[`CLAUDE.md`](CLAUDE.md)`: the link target names the file
        head = &head[head.rfind("](")? + 2..head.len() - 1];
    }
    if !head.ends_with(".md") {
        return None;
    }
    let tok: String = head.chars().rev().take_while(|c| c.is_ascii_alphanumeric() || "_-./".contains(*c)).collect::<Vec<_>>().into_iter().rev().collect();
    (tok.len() > 3).then_some(tok)
}

/// A size written next to a document's name (`CLAUDE.md (4 KB)`) that is not close to the file's size (within a third, or a kilobyte).
pub fn size_claims(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for doc in CLAIM_DOCS {
        for line in read(root, doc).lines() {
            let mut from = 0;
            while let Some(i) = line[from..].find(" KB)") {
                let close = from + i;
                from = close + 1;
                let Some(open) = line[..close].rfind('(') else { continue };
                let digits = line[open + 1..close].trim_start_matches('~');
                let Ok(claimed) = digits.parse::<f64>() else { continue };
                let Some(name) = md_name_before(line, open) else { continue };
                let Ok(meta) = std::fs::metadata(root.join(&name)) else { continue };
                let actual = meta.len() as f64 / 1024.0;
                if (actual - claimed).abs() > (claimed / 3.0).max(1.0) {
                    out.push(format!("{doc}: says `{name} ({digits} KB)`, the file is {actual:.1} KB; drop the size or correct it"));
                }
            }
        }
    }
    out
}

/// The tools `red_engine2 mcp` lists, read from `src/cli/mcp.rs` (every `#[tool(...)]` method), so the check needs no build and no running server.
pub fn mcp_tool_names(root: &Path) -> Vec<String> {
    let text = read(root, "src/cli/mcp.rs");
    let mut out = Vec::new();
    let mut lines = text.lines();
    while let Some(l) = lines.next() {
        if !l.trim_start().starts_with("#[tool(") {
            continue;
        }
        for next in lines.by_ref() {
            if let Some(rest) = next.trim_start().strip_prefix("async fn ").or_else(|| next.trim_start().strip_prefix("fn ")) {
                out.push(rest.split('(').next().unwrap_or("").to_string());
                break;
            }
        }
    }
    out
}

const NUMBER_WORDS: &[&str] =
    &["one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen"];

/// Lines about the MCP server that name a tool it does not have, or state how many tools it has as a plain number (write `<!--fact:mcp-tools-->N<!--/fact-->`).
pub fn mcp_claims(root: &Path) -> Vec<String> {
    let tools = mcp_tool_names(root);
    if tools.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for doc in CLAIM_DOCS {
        for line in without_facts(&read(root, doc)).lines() {
            if !line.to_lowercase().contains("mcp") {
                continue;
            }
            // "the `start` tool", "`view` tool"
            let spans: Vec<&str> = line.split('`').collect();
            for (n, span) in spans.iter().enumerate().skip(1).step_by(2) {
                let followed = spans.get(n + 1).copied().unwrap_or("");
                let is_tool = followed.strip_prefix(" tool").is_some_and(|r| !r.starts_with(|c: char| c.is_alphabetic()));
                if is_tool && !tools.iter().any(|t| t == span) {
                    out.push(format!("{doc}: names an MCP tool `{span}`; `red_engine2 mcp` lists {} (src/cli/mcp.rs)", tools.join(", ")));
                }
            }
            // "eleven tools (`describe`, `search`, ...)": a count written by hand, and a list that must be the real one
            if let Some(i) = line.find(" tools (`") {
                let before = line[..i].split_whitespace().last().unwrap_or("").to_lowercase();
                if before.parse::<u32>().is_ok() || NUMBER_WORDS.contains(&before.as_str()) {
                    out.push(format!("{doc}: counts the MCP tools by hand (`{before} tools`); write the count as <!--fact:mcp-tools-->N<!--/fact-->"));
                }
                let list = &line[i + " tools (".len()..];
                for name in list.split(')').next().unwrap_or("").split('`').skip(1).step_by(2) {
                    if !tools.iter().any(|t| t == name) {
                        out.push(format!("{doc}: lists an MCP tool `{name}`; `red_engine2 mcp` lists {}", tools.join(", ")));
                    }
                }
            }
        }
    }
    out
}

/// Whether `feature` is in the default feature set of Cargo.toml (`default = ["gfx", "mcp"]`).
fn default_feature(root: &Path, feature: &str) -> bool {
    read(root, "Cargo.toml")
        .lines()
        .find_map(|l| l.trim().strip_prefix("default = ["))
        .is_some_and(|rest| rest.split(']').next().unwrap_or("").split(',').any(|f| f.trim().trim_matches('"') == feature))
}

/// A `render` command (MP4 export) shown without the cargo feature that builds it: `red_engine2 render` fails in the default build.
pub fn video_claims(root: &Path) -> Vec<String> {
    if default_feature(root, "video") {
        return Vec::new();
    }
    let mut out = Vec::new();
    for doc in CLAIM_DOCS {
        for block in read(root, doc).split("\n\n") {
            let shows_render = block.lines().any(|l| {
                let t = l.trim_start().trim_start_matches('$').trim_start();
                !t.starts_with('|')
                    && ["red_engine2 render ", "$R render ", "scripts/dev red render "].iter().any(|p| t.starts_with(p) || t.contains(&format!("`{p}")))
            });
            if shows_render && !block.contains("features video") && !block.contains("features=video") {
                out.push(format!(
                    "{doc}: shows `render` without `--features video` nearby; the default build cannot export MP4 (Cargo.toml: `video` is not in `default`)"
                ));
            }
        }
    }
    out
}

/// Every claim check, as sentences (empty when the documents are true).
pub fn all(root: &Path) -> Vec<String> {
    let mut out = protocol_claims(root);
    out.extend(disproved_claims(root));
    out.extend(size_claims(root));
    out.extend(mcp_claims(root));
    out.extend(video_claims(root));
    out
}

/// A derived value for `<!--fact:name-->` that this module knows (`mcp-tools`, `brief-kb`).
pub fn inline_fact(root: &Path, name: &str) -> Option<String> {
    match name {
        "mcp-tools" => Some(mcp_tool_names(root).len().to_string()).filter(|n| n != "0"),
        // the size `describe --brief` may not exceed, in kilobytes: "under 2.5 KB"
        "brief-kb" => Some(format!("{:.1}", super::describe::BRIEF_BUDGET as f64 / 1000.0).trim_end_matches(".0").to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(files: &[(&str, &str)]) -> tempfile_dir::Dir {
        let d = tempfile_dir::Dir::new();
        for (p, t) in files {
            let path = d.path().join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, t).unwrap();
        }
        d
    }

    /// A scratch directory removed on drop (the crate has no tempfile dependency).
    mod tempfile_dir {
        pub struct Dir(std::path::PathBuf);
        impl Dir {
            pub fn new() -> Dir {
                use std::sync::atomic::{AtomicUsize, Ordering};
                static N: AtomicUsize = AtomicUsize::new(0);
                let p = std::env::temp_dir().join(format!("doc_claims_{}_{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
                std::fs::create_dir_all(&p).unwrap();
                Dir(p)
            }
            pub fn path(&self) -> &std::path::Path {
                &self.0
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    const PROTOCOL_RS: (&str, &str) = ("src/net/protocol.rs", "pub const PROTOCOL_VERSION: u16 = 15;\n");

    #[test]
    fn a_stale_protocol_number_is_found_and_a_fact_or_a_since_clause_is_not() {
        let d = tree(&[
            PROTOCOL_RS,
            ("README.md", "Online uses protocol v5 here.\n"),
            ("AGENTS.md", "Protocol v<!--fact:protocol-->15<!--/fact--> now; the field exists from protocol v7 and later.\n"),
        ]);
        let found = protocol_claims(d.path());
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].starts_with("README.md") && found[0].contains("v5") && found[0].contains("v15"));
    }

    #[test]
    fn a_missing_feature_is_only_disproved_while_its_code_is_there() {
        let sentence = "What it does **not** have: lag compensation for hitscan.\n";
        let with_code = tree(&[("README.md", sentence), ("src/sim/match_sim.rs", "pub const HISTORY_TICKS: usize = 16;\n")]);
        let found = disproved_claims(with_code.path());
        assert!(found.iter().any(|f| f.contains("lag compensation is built")), "{found:?}");
        let without_code = tree(&[("README.md", sentence), ("src/sim/match_sim.rs", "// nothing\n")]);
        assert!(disproved_claims(without_code.path()).is_empty());
        let spec = tree(&[("AGENTS.md", "Do not open SPEC.md whole.\n"), ("README.md", "Both are written to be read once and used directly.\n")]);
        assert!(disproved_claims(spec.path()).iter().any(|f| f.starts_with("README.md") && f.contains("never to open SPEC.md whole")));
    }

    #[test]
    fn a_size_next_to_a_document_name_must_be_near_the_real_size() {
        let big = "x".repeat(8 * 1024);
        let d = tree(&[
            ("CLAUDE.md", &big),
            ("README.md", "Read `CLAUDE.md` (4 KB) first, and AGENTS.md (~8 KB), but `describe --brief` (1 KB) is a command.\n"),
            ("AGENTS.md", &big),
        ]);
        let found = size_claims(d.path());
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("CLAUDE.md (4 KB)") && found[0].contains("8.0 KB"), "{found:?}");
    }

    #[test]
    fn an_mcp_tool_that_does_not_exist_and_a_hand_counted_total_are_found() {
        let mcp = "    #[tool(description = \"a\")]\n    async fn describe(&self) {}\n    #[tool(description = \"b\")]\n    async fn search(&self) {}\n";
        let d = tree(&[
            ("src/cli/mcp.rs", mcp),
            ("AGENTS.md", "(in a game project `scripts/red start`; MCP: the `start` tool)\n"),
            (
                "README.md",
                "MCP server: two tools (`describe`, `search`, `view`)\nMCP server: <!--fact:mcp-tools-->2<!--/fact--> tools (`describe`, `search`)\n",
            ),
        ]);
        assert_eq!(mcp_tool_names(d.path()), ["describe", "search"]);
        let found = mcp_claims(d.path());
        assert!(found.iter().any(|f| f.starts_with("AGENTS.md") && f.contains("`start`")), "{found:?}");
        assert!(found.iter().any(|f| f.contains("counts the MCP tools by hand")), "{found:?}");
        assert!(found.iter().any(|f| f.contains("lists an MCP tool `view`")), "{found:?}");
        assert_eq!(found.len(), 3, "{found:?}");
    }

    #[test]
    fn render_needs_its_feature_shown_unless_the_default_build_has_it() {
        let doc = "```bash\nred_engine2 render a.json out.mp4\n```\n\nThen `cargo build --features video` and `red_engine2 render a.json b.mp4`.\n";
        let d = tree(&[("Cargo.toml", "default = [\"gfx\", \"mcp\"]\n"), ("README.md", doc)]);
        assert_eq!(video_claims(d.path()).len(), 1, "{:?}", video_claims(d.path()));
        let default_video = tree(&[("Cargo.toml", "default = [\"gfx\", \"video\"]\n"), ("README.md", doc)]);
        assert!(video_claims(default_video.path()).is_empty());
    }

    #[test]
    fn the_derived_values_are_the_tool_count_and_the_brief_budget() {
        let d = tree(&[("src/cli/mcp.rs", "#[tool(description = \"a\")]\nasync fn describe() {}\n")]);
        assert_eq!(inline_fact(d.path(), "mcp-tools").as_deref(), Some("1"));
        assert_eq!(inline_fact(d.path(), "brief-kb").as_deref(), Some("2.5"));
        assert_eq!(inline_fact(d.path(), "nope"), None);
    }
}
