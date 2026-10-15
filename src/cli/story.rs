//! `red_engine2 story ...`: the story toolchain's commands (`tools::story`). `check` and `build` need only the story folder (and the game's
//! bindings file it names); `walk` runs the real game, so it runs in the game's own program (`<game>_server cli story walk ...`).

use crate::tools::envelope;
use crate::tools::story::{self, refs, walk, Built};
use clap::Subcommand;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Instant;

macro_rules! println {
    ($($t:tt)*) => {{ envelope::write_out(format_args!($($t)*)); envelope::write_out(format_args!("\n")); }};
}

/// The story toolchain.
#[derive(Subcommand, Debug)]
pub enum StoryCmd {
    /// Parse, bind (to the game's bindings and the scene's variables), type-check and lint a story: a folder of `.story` files, or one.
    /// Prints every problem as `file:line: error|warning[code]: message -> fix`; exit 1 on an error.
    Check { path: PathBuf },
    /// `check`, then write the compiled package the game loads (`<folder>/<story>.story.json`): a content build, no Rust compiled.
    /// `--check`: write nothing, fail if the package on disk is not what the sources compile to.
    Build {
        path: PathBuf,
        /// Fail if the package on disk is stale instead of writing it.
        #[arg(long)]
        check: bool,
        /// Write the package here instead.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Who reads, writes, tells or jumps to `id` (a fact, an info, a game query or command, a conversation, a line `conv/id`, a choice
    /// `conv/choice`, a bark), what a choice can change, why a conversation may be closed, what breaks if a line goes.
    Refs { path: PathBuf, id: String },
    /// The setup/payoff ledger, every choice's stakes (declared and traced from what it changes), facts' writers and readers, who can
    /// know what.
    Ledger { path: PathBuf },
    /// The editorial review sheet: the characters side by side, the choices and their stakes, the setups, and the questions a reader asks
    /// (nothing here is scored: it is for a person or a reviewing agent).
    Review { path: PathBuf },
    /// Run the story under the game's real match from each walk's script, branching at every `"choice": "*"`, within bounds; report what
    /// was reached, what was held back and why, what was never visited. Needs the game's program (`<game>_server cli story walk ...`).
    Walk {
        /// The scene that plays the game.
        scene: PathBuf,
        /// The walks: a JSON array of scenarios (with `"choice": "*"` steps and optional `branches`).
        walks: PathBuf,
        /// Most runs per walk.
        #[arg(long, default_value_t = 64)]
        max_paths: usize,
        /// Most branch points along one run.
        #[arg(long, default_value_t = 8)]
        max_depth: usize,
        /// Only walks whose name contains this.
        #[arg(long)]
        only: Option<String>,
    },
}

fn diags(b: &Built) -> Value {
    json!(b.diags.iter().map(|d| d.to_json(&b.ast.files)).collect::<Vec<_>>())
}

fn summary(b: &Built) -> String {
    let (e, w, a) = (b.errors(), b.warnings(), b.allowed());
    match &b.package {
        Some(p) => {
            let choices: usize = p.lines.iter().map(|l| l.choices.len()).sum();
            format!(
                "story {} v{}: {} speaker(s), {} conversation(s) ({} lines, {} choices), {} bark(s), {} director(s), {} fact(s), {} info(s); {e} error(s), {w} warning(s), {a} allowed",
                p.story,
                p.content_version,
                p.speakers.len(),
                p.conversations.len(),
                p.lines.len() - p.barks.iter().filter(|x| x.line.is_some()).count(),
                choices,
                p.barks.len(),
                p.directors.len(),
                p.facts.len(),
                p.infos.len()
            )
        }
        None => format!("story: {e} error(s), {w} warning(s), {a} allowed"),
    }
}

fn print_diags(b: &Built) {
    for d in &b.diags {
        println!("{}", d.render(&b.ast.files));
    }
}

fn load(path: &Path) -> Result<Built, String> {
    story::build(path)
}

/// Runs a `story` command.
pub fn run(cmd: StoryCmd) -> Result<(), String> {
    let json = envelope::capturing();
    match cmd {
        StoryCmd::Check { path } => {
            let t = Instant::now();
            let b = load(&path)?;
            if json {
                println!("{}", json!({"ok": b.errors() == 0, "summary": summary(&b), "diagnostics": diags(&b), "ms": t.elapsed().as_millis()}));
            } else {
                print_diags(&b);
                println!("{} ({} ms)", summary(&b), t.elapsed().as_millis());
            }
            if b.errors() > 0 {
                return Err(String::new());
            }
            Ok(())
        }
        StoryCmd::Build { path, check, out } => {
            let t = Instant::now();
            let b = load(&path)?;
            let Some(p) = &b.package else {
                if json {
                    println!("{}", json!({"ok": false, "summary": summary(&b), "diagnostics": diags(&b)}));
                } else {
                    print_diags(&b);
                    println!("{}", summary(&b));
                }
                return Err("not built: fix the errors above".into());
            };
            let dest = out.unwrap_or_else(|| story::package_path(&b.root, p));
            let text = story::package_text(p);
            let same = std::fs::read_to_string(&dest).ok().as_deref() == Some(text.as_str());
            if check {
                if !json {
                    print_diags(&b);
                }
                if !same {
                    return Err(format!("{} is stale: run `story build {}`", dest.display(), path.display()));
                }
            } else if !same {
                std::fs::write(&dest, &text).map_err(|e| format!("{}: {e}", dest.display()))?;
            }
            let ms = t.elapsed().as_millis();
            if json {
                println!(
                    "{}",
                    json!({"ok": true, "package": dest, "bytes": text.len(), "changed": !same, "summary": summary(&b), "diagnostics": diags(&b), "ms": ms})
                );
            } else {
                if !check {
                    print_diags(&b);
                }
                let what = if check {
                    "up to date"
                } else if same {
                    "unchanged"
                } else {
                    "written"
                };
                println!("{} -> {} ({what}, {} bytes, {ms} ms)", summary(&b), dest.display(), text.len());
            }
            Ok(())
        }
        StoryCmd::Refs { path, id } => {
            let b = load(&path)?;
            let v = refs::refs(&b, &id)?;
            println!("{}", if json { v.to_string() } else { serde_json::to_string_pretty(&v).unwrap_or_default() });
            Ok(())
        }
        StoryCmd::Ledger { path } => {
            let b = load(&path)?;
            let v = refs::ledger(&b)?;
            if json {
                println!("{v}");
            } else {
                println!("{}", ledger_text(&v));
            }
            Ok(())
        }
        StoryCmd::Review { path } => {
            let b = load(&path)?;
            println!("{}", review_text(&b)?);
            Ok(())
        }
        StoryCmd::Walk { scene, walks, max_paths, max_depth, only } => {
            let report = walk::walk_file(&scene, &walks, walk::Bounds { max_paths, max_depth }, only.as_deref())?;
            if json {
                println!("{}", report.to_json());
            } else {
                println!("{}", report.render());
            }
            if report.failed() > 0 {
                return Err(String::new());
            }
            Ok(())
        }
    }
}

fn ledger_text(v: &Value) -> String {
    let mut out = String::new();
    let arr = |k: &str| v.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
    out.push_str("SETUPS\n");
    for s in arr("setups") {
        out.push_str(&format!("  {} ({}): {}\n", s["id"].as_str().unwrap_or(""), s["status"].as_str().unwrap_or(""), s["what"].as_str().unwrap_or("")));
        for e in s["established_by"].as_array().into_iter().flatten() {
            out.push_str(&format!("    set up at {} when {}\n", e["at"].as_str().unwrap_or(""), e["when"].as_str().unwrap_or("?")));
        }
        for o in s["outcomes"].as_array().into_iter().flatten() {
            out.push_str(&format!(
                "    {:<11} {} when {}\n",
                o["kind"].as_str().unwrap_or(""),
                o["at"].as_str().unwrap_or(""),
                o["available_when"].as_str().unwrap_or("?")
            ));
        }
    }
    out.push_str("CHOICES (stakes: declared or traced; why)\n");
    for c in arr("choices") {
        let mark = if c["declared"].as_bool().unwrap_or(false) { "declared" } else { "traced" };
        out.push_str(&format!(
            "  {:<28} {:<13} ({mark}) {}\n",
            c["id"].as_str().unwrap_or(""),
            c["stakes"].as_str().unwrap_or(""),
            c["why"].as_str().unwrap_or("")
        ));
    }
    out.push_str("FACTS (writes / reads)\n");
    for f in arr("facts") {
        out.push_str(&format!("  {:<20} {:<8} {} / {}\n", f["id"].as_str().unwrap_or(""), f["scope"].as_str().unwrap_or(""), f["writes"], f["reads"]));
    }
    out.push_str("KNOWLEDGE\n");
    for k in arr("knowledge") {
        let start: Vec<String> = k["known_at_start_by"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect();
        let told: Vec<String> = k["told_by"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect();
        out.push_str(&format!("  {}: known by [{}]; told by [{}]\n", k["info"].as_str().unwrap_or(""), start.join(", "), told.join("; ")));
    }
    out
}

/// The review sheet: what a reader needs side by side, and the questions only a reader can answer.
fn review_text(b: &Built) -> Result<String, String> {
    let p = b.package.as_ref().ok_or("the story has errors: `story check` first")?;
    let mut out = format!("EDITORIAL REVIEW: {} v{} (a checklist for a reader; nothing here is scored)\n\nCHARACTERS\n", p.story, p.content_version);
    for c in &b.ast.characters {
        out.push_str(&format!("  {} \"{}\"{}\n", c.id, c.name, if c.minor { " (extra)" } else { "" }));
        for (k, v) in &c.fields {
            if !matches!(k.as_str(), "present" | "cooldown") {
                out.push_str(&format!("    {k}: {}\n", v.text));
            }
        }
        let lines: Vec<String> =
            p.lines.iter().filter(|l| l.speaker.is_some_and(|s| p.speakers[s as usize].id == c.id)).take(4).map(|l| piece_text(&l.text)).collect();
        out.push_str(&format!("    says, for instance: {}\n", lines.iter().map(|l| format!("\"{l}\"")).collect::<Vec<_>>().join(" / ")));
    }
    out.push_str("\nCHOICES\n");
    for (ci, li, k, stakes, why) in story::lint::trace_stakes(p, &b.index, &b.catalog) {
        let ch = &p.lines[li as usize].choices[k];
        out.push_str(&format!("  {}/{} \"{}\" [{:?}] {why}\n", p.conversations[ci as usize].id, ch.id, piece_text(&ch.text), stakes));
    }
    out.push_str("\nSETUPS\n");
    for s in &b.ast.setups {
        out.push_str(&format!("  {}: {} ({} outcome(s))\n", s.id, s.doc, s.pays.len()));
    }
    out.push_str(
        "\nQUESTIONS FOR THE READER (judgement, not lint; answer with line ids and patch only those)\n\
  1. Premise: is it specific to this game's verbs and places, or could it be any game's?\n\
  2. Motivation: does every major character want something now, and does a scene put that want under pressure?\n\
  3. Voice: cover the speaker names; can you still tell who says each line? Is the difference perspective, not a catchphrase?\n\
  4. Exposition: is anything explained that the player could see, do, or be left to infer? Is a silence better?\n\
  5. Reversals: is every change of heart earned by something the player did or learned on screen?\n\
  6. Consequences: for each consequential choice, what does the player perceive afterwards, and when? (`story ledger`)\n\
  7. Callbacks: does a later line name what this player actually did, rather than a generic 'you'?\n\
  8. Refusal and failure: are refusing, delaying and failing playable answers with their own aftermath?\n\
  9. Pacing: does the talk fit the game's rhythm (barks short in action, conversations where the world allows)?\n\
 10. Ending: does what is left unresolved feel chosen (a `pays leave`) rather than forgotten?\n",
    );
    Ok(out)
}

fn piece_text(t: &[crate::sim::story::package::Piece]) -> String {
    t.iter()
        .map(|x| match x {
            crate::sim::story::package::Piece::Text(s) => s.clone(),
            crate::sim::story::package::Piece::Var { .. } => "{..}".to_string(),
        })
        .collect()
}
