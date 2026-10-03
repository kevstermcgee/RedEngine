//! Repository housekeeping commands: `preflight`, `adr`, `analysis` (ADR 2026-09-28-generated-bookkeeping).

use super::*;
use red_engine2::tools::{adr, analysis, features, preflight, status};

/// The repository to work on: `--root`, else the nearest parent of the current directory that is the engine's tree, else the tree this binary was built from.
fn repo_root(given: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(r) = given {
        return Ok(r.to_path_buf());
    }
    if let Ok(mut d) = std::env::current_dir() {
        loop {
            if d.join("src/lib.rs").is_file() && d.join("docs/adr").is_dir() {
                return Ok(d);
            }
            if !d.pop() {
                break;
            }
        }
    }
    symbols::find_root().ok_or_else(|| "cannot find the engine's source tree: run inside it, pass --root, or set RE2_SRC=<repo path>".to_string())
}

/// `preflight`: run the bookkeeping checks; with `--fix` apply the mechanical edits and check again.
pub(crate) fn run_preflight(fix: bool, no_fmt: bool, tree: bool, root: Option<&Path>) -> Result<(), String> {
    let root = repo_root(root)?;
    let commands = commands_json();
    let pairs: Vec<(String, String)> = commands
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            Some((c["name"].as_str()?.to_string(), c["about"].as_str().unwrap_or("").lines().next().unwrap_or("").trim().trim_end_matches('.').to_string()))
        })
        .collect();
    let sizes = describe::render("brief", &commands, false).ok().zip(describe::render("overview", &commands, false).ok()).map(|(b, o)| (b.len(), o.len()));
    let opts = preflight::Options { fmt: !no_fmt, commands: pairs, describe_sizes: sizes, tree };
    let mut report = preflight::run(&root, &opts);
    if fix && !report.problems.is_empty() {
        let fixes: Vec<_> = report.problems.iter().filter_map(|p| p.fix.clone()).collect();
        for line in preflight::apply(&root, &fixes)? {
            println!("fixed: {line}");
        }
        report = preflight::run(&root, &opts);
    }
    if envelope::capturing() {
        println!("{}", serde_json::to_string_pretty(&preflight::to_json(&report)).unwrap_or_default());
    } else {
        print!("{}", preflight::render(&report));
    }
    if report.problems.is_empty() {
        Ok(())
    } else {
        Err(String::new())
    }
}

/// `adr new|list|index`.
pub(crate) fn run_adr(cmd: AdrCmd, root: Option<&Path>) -> Result<(), String> {
    let root = repo_root(root)?;
    match cmd {
        AdrCmd::New { title, summary, status: st, slug } => {
            let path = adr::new_adr(&root, &title, summary.as_deref(), &st, slug.as_deref(), &status::today())?;
            let file = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            println!("created {} (index refreshed)", path.display());
            println!("refer to it as ADR {}", adr::id_of(&file).unwrap_or_default());
            if summary.is_none() {
                println!("now: write the `Summary:` line and the three sections; `red_engine2 preflight` fails until the Summary is real");
            }
            Ok(())
        }
        AdrCmd::List => {
            if envelope::capturing() {
                let list: Vec<Value> = adr::all(&adr::load_dir(&root))
                    .iter()
                    .map(|a| serde_json::json!({"id": a.id, "file": a.file, "title": a.title, "status": a.status, "summary": a.summary}))
                    .collect();
                println!("{}", serde_json::to_string_pretty(&list).unwrap_or_default());
            } else {
                print!("{}", adr::render_list(&adr::all(&adr::load_dir(&root))));
            }
            Ok(())
        }
        AdrCmd::Index { write } => {
            if write {
                println!("{}", if adr::write_index(&root)? { "docs/adr/README.md: index rewritten" } else { "docs/adr/README.md: index already current" });
                return Ok(());
            }
            let problems = adr::check(&root);
            if problems.is_empty() {
                println!("the ADR index and every record are in order");
                Ok(())
            } else {
                Err(problems.join("\n"))
            }
        }
    }
}

/// `analysis new|list`.
pub(crate) fn run_analysis(cmd: AnalysisCmd, root: Option<&Path>) -> Result<(), String> {
    let root = repo_root(root)?;
    match cmd {
        AnalysisCmd::New { title, from } => {
            let body = match from {
                Some(p) => Some(std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?),
                None => None,
            };
            let path = analysis::new_note(&root, &title, body.as_deref(), &status::today())?;
            println!("created {}", path.display());
            Ok(())
        }
        AnalysisCmd::List => {
            print!("{}", analysis::render_list(&analysis::list(&root)));
            Ok(())
        }
        AnalysisCmd::Digest { write } => {
            let feats = features::load_at(&root)?;
            if write {
                println!(
                    "{}",
                    if analysis::write_digest(&root, &feats)? {
                        "docs/analysis/README.md: digest rewritten"
                    } else {
                        "docs/analysis/README.md: digest already current"
                    }
                );
                return Ok(());
            }
            let problems = analysis::check_digest(&root, &feats);
            if problems.is_empty() {
                println!("the cross-game friction digest is current");
                Ok(())
            } else {
                Err(problems.join("\n"))
            }
        }
    }
}
