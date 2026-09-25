//! The self-description commands: describe, search, src, recipe, catalog (the AI-facing surface).

use super::*;

/// The CLI's own definition as JSON (name, about, args): the source of truth `describe`/`search` read,
/// so command docs can never drift from the real flags.
pub(crate) fn commands_json() -> Value {
    use clap::CommandFactory;
    let cmd = Cli::command();
    Value::Array(
        cmd.get_subcommands()
            .map(|c| {
                let args: Vec<Value> = c
                    .get_arguments()
                    .filter(|a| a.get_id() != "help")
                    .map(|a| {
                        serde_json::json!({
                            "name": a.get_id().as_str(),
                            "positional": a.is_positional(),
                            "required": a.is_required_set(),
                            "flag": !a.get_action().takes_values(),
                            "help": a.get_help().map(|s| s.to_string()).unwrap_or_default(),
                        })
                    })
                    .collect();
                let subs: Vec<String> = c.get_subcommands().map(|s| s.get_name().to_string()).collect();
                serde_json::json!({"name": c.get_name(), "about": c.get_about().map(|s| s.to_string()).unwrap_or_default(), "args": args, "subcommands": subs})
            })
            .collect(),
    )
}

pub(crate) fn run_describe(topic: Option<&str>, brief: bool, json: bool) -> Result<(), String> {
    let topic = if brief { "brief" } else { topic.unwrap_or("overview") };
    print!("{}", describe::render(topic, &commands_json(), json)?);
    Ok(())
}

pub(crate) fn run_search(query: &str, kind: Option<&str>, limit: usize) -> Result<(), String> {
    if query.trim().is_empty() {
        return Err("give some words to search for: `red_engine2 search how do stairs work`".to_string());
    }
    let docs = search::corpus(&commands_json());
    let hits = search::search(&docs, query, kind, limit);
    if envelope::capturing() {
        println!("{}", search::to_json(&hits, query));
    } else {
        print!("{}", search::render(&hits, query));
    }
    Ok(())
}

pub(crate) fn run_src(cmd: SrcCmd) -> Result<(), String> {
    let root = symbols::find_root().ok_or("cannot find the source tree (run inside the repo or set RE2_SRC=<repo path>)")?;
    let ix = symbols::Index::build(&root);
    match cmd {
        SrcCmd::Map => print!("{}", symbols::render_map(&ix)),
        SrcCmd::Find { query, kind, file, tests, limit } => {
            let rows = symbols::find(&ix, &query.join(" "), kind.as_deref(), file.as_deref(), tests, limit);
            print!("{}", symbols::render_symbols(&rows));
        }
        SrcCmd::Outline { file, all } => print!("{}", symbols::outline(&ix, &file, all)?),
        SrcCmd::Show { symbol, lines } => print!("{}", symbols::show(&ix, &symbol.join(" "), lines.max(1))?),
        SrcCmd::Refs { symbol, limit } => print!("{}", symbols::refs(&ix, &symbol, limit)),
        SrcCmd::Deps { module } => print!("{}", symbols::render_deps(&ix, module.as_deref())?),
        SrcCmd::Coverage { file, limit } => print!("{}", symbols::coverage(&ix, file.as_deref(), limit)),
    }
    Ok(())
}

pub(crate) fn run_recipe(name: Option<&str>, new: Option<&Path>, print: bool) -> Result<(), String> {
    let Some(name) = name else {
        print!("{}", recipes::render_list());
        return Ok(());
    };
    let text = recipes::text_of(name).ok_or_else(|| recipes::render_one(name).err().unwrap_or_default())?;
    if let Some(out) = new {
        if out.exists() {
            return Err(format!("{} already exists (refusing to overwrite)", out.display()));
        }
        if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(out, text).map_err(|e| e.to_string())?;
        println!("wrote {} - next: red_engine2 lint {0} ; plan ; tour ; verify", out.display());
    } else if print {
        print!("{text}");
    } else {
        print!("{}", recipes::render_one(name)?);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_catalog(
    query: &[String],
    tag: Option<&str>,
    category: Option<&str>,
    kind: Option<&str>,
    long: bool,
    json: bool,
    sheet: Option<&Path>,
    cols: u32,
) -> Result<(), String> {
    let all = catalog::entries();
    if let (Some(name), None, None, None, None) = (query.first().filter(|_| query.len() == 1), tag, category, kind, sheet) {
        if let Some(e) = catalog::find(&all, name) {
            print!("{}", catalog::render_detail(e, json));
            return Ok(());
        }
    }
    let q = query.join(" ");
    let rows = catalog::filter(&all, if q.is_empty() { None } else { Some(&q) }, tag, category, kind);
    if let Some(out) = sheet {
        let started = Instant::now();
        catalog::sheet(&rows, out, cols, (480, 360))?;
        println!("wrote {} ({} assets, {:.1}s)", out.display(), rows.len(), started.elapsed().as_secs_f32());
        return Ok(());
    }
    if rows.is_empty() && query.len() == 1 {
        let hint = red_engine2::prefabs::suggest(&query[0], all.iter().map(|e| e.name.as_str()));
        if !hint.is_empty() {
            println!("no match for '{}' — did you mean: {}?", query[0], hint.join(", "));
            return Ok(());
        }
    }
    print!("{}", catalog::render_list(&rows, json, long));
    Ok(())
}

pub(crate) fn run_status(root: &Path, init: bool, note: Option<&str>, section: &str, facts: bool, sync_docs: &[PathBuf]) -> Result<(), String> {
    use red_engine2::tools::status;
    if init {
        let p = status::init(root)?;
        println!("STATUS.md ready at {}", p.display());
    }
    if let Some(text) = note {
        let p = status::add_note(root, section, text)?;
        println!("noted under '{section}' in {}", p.display());
    }
    if !sync_docs.is_empty() {
        let block = status::facts_block(root);
        for doc in sync_docs {
            let text = std::fs::read_to_string(doc).map_err(|e| format!("{}: {e}", doc.display()))?;
            let new = status::sync_facts(&text, &block).map_err(|e| format!("{}: {e}", doc.display()))?;
            if new != text {
                std::fs::write(doc, new).map_err(|e| format!("{}: {e}", doc.display()))?;
                println!("updated facts in {}", doc.display());
            } else {
                println!("{} already current", doc.display());
            }
        }
        return Ok(());
    }
    if facts {
        print!("{}", status::facts_block(root));
    } else if !init && note.is_none() {
        print!("{}", status::render(root));
    }
    Ok(())
}

pub(crate) fn run_build(blueprint: Option<&Path>, out: Option<&Path>, check: bool, example: bool) -> Result<(), String> {
    use red_engine2::tools::blueprint;
    if example {
        print!("{}", blueprint::example());
        return Ok(());
    }
    let bp_path = blueprint.ok_or("give a blueprint file, or `build --example` to print a starter")?;
    let text = std::fs::read_to_string(bp_path).map_err(|e| format!("{}: {e}", bp_path.display()))?;
    let value: Value = serde_json::from_str(&text).map_err(|e| format!("{}: not valid JSON: {e}", bp_path.display()))?;
    let built = blueprint::compile(&value).map_err(|errs| format!("{}: the blueprint has {} problem(s):\n  {}", bp_path.display(), errs.len(), errs.join("\n  ")))?;
    let dest = out.map(Path::to_path_buf).unwrap_or_else(|| red_engine2::tools::game::blueprint_out(bp_path));
    for line in &built.summary {
        println!("{line}");
    }
    for (sev, code, msg) in built.findings.iter().take(8) {
        println!("  lint {} [{code}] {msg}", if *sev == Severity::Error { "ERROR" } else { "warn " });
    }
    if check {
        let existing = std::fs::read_to_string(&dest).map_err(|e| format!("{}: {e} (run `red_engine2 build {}` to create it)", dest.display(), bp_path.display()))?;
        if existing.replace("\r\n", "\n") != built.scene_text.replace("\r\n", "\n") {
            return Err(format!("STALE: {} differs from what {} builds. Run `red_engine2 build {}` (or, if you meant to hand-edit the map, delete the blueprint).", dest.display(), bp_path.display(), bp_path.display()));
        }
        println!("{} is up to date with {}", dest.display(), bp_path.display());
        return Ok(());
    }
    if let Some(parent) = dest.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(&dest, &built.scene_text).map_err(|e| format!("{}: {e}", dest.display()))?;
    println!("wrote {}", dest.display());
    println!("next: red_engine2 verify {0} --no-views   |   red_engine2 plan {0}   |   red_engine2 tour {0} out/tour.png", dest.display());
    if built.errors() > 0 {
        return Err(format!("{} lint error(s) in the built map: change the blueprint (bigger rooms, fewer fill items) and rebuild", built.errors()));
    }
    Ok(())
}

pub(crate) fn run_new_game(dir: &Path, name: Option<&str>, engine_path: Option<String>, engine_git: Option<String>, engine_ref: Option<String>) -> Result<(), String> {
    use red_engine2::tools::{game::EngineRef, newgame};
    let name = match name {
        Some(n) => n.to_string(),
        None => std::fs::canonicalize(dir).ok().or_else(|| Some(dir.to_path_buf())).and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string())).unwrap_or_else(|| "game".into()),
    };
    let engine = EngineRef { git: engine_git, git_ref: engine_ref, path: engine_path };
    let files = newgame::scaffold(dir, &name, &engine)?;
    println!("created game project '{name}' in {} ({} files)", dir.display(), files.len());
    println!("next:");
    println!("  cd {}", dir.display());
    println!("  scripts/red check            # (scripts\\red.ps1 on Windows) fetches + builds the engine on first use, then verifies the starter map");
    println!("  scripts/red plan maps/main.json   # look at it; then edit blueprints/main.blueprint.json and `scripts/red build-all`");
    Ok(())
}

pub(crate) fn run_game(dir: &Path, cmd: GameCmd) -> Result<(), String> {
    use red_engine2::tools::game;
    let cfg = game::load(dir).map_err(|e| e.join("
"))?;
    match cmd {
        GameCmd::Info => {
            print!("{}", game::info(&cfg));
            Ok(())
        }
        GameCmd::BuildAll => {
            let lines = game::build_all(&cfg);
            for l in &lines {
                println!("{} {}", if l.failed { "FAIL" } else { "ok  " }, l.text);
            }
            if lines.iter().any(|l| l.failed) {
                return Err(String::new());
            }
            Ok(())
        }
        GameCmd::Check { views } => {
            let report = game::check(&cfg, views);
            print!("{}", report.render());
            if report.failed() > 0 {
                return Err(String::new());
            }
            Ok(())
        }
        GameCmd::Serve { extra } => {
            let exe = game::sibling_exe("red_server").ok_or("red_server is not built next to red_engine2 (cargo build --bin red_server, or use `scripts/red serve`)")?;
            let mut args = game::server_args(&cfg);
            args.extend(extra);
            launch(&exe, &args)
        }
        GameCmd::Play { addr } => {
            let exe = game::sibling_exe("re2").ok_or("re2 (the game client) is not built next to red_engine2 (cargo build --bin re2, or use `scripts/red play`)")?;
            let addr = addr.unwrap_or_else(|| format!("127.0.0.1:{}", cfg.server.port));
            launch(&exe, &["--connect".to_string(), addr, cfg.dir.join(&cfg.server.map).display().to_string()])
        }
    }
}

/// Runs a sibling program with inherited stdio and passes its exit code on.
fn launch(exe: &Path, args: &[String]) -> Result<(), String> {
    let status = std::process::Command::new(exe).args(args).status().map_err(|e| format!("{}: {e}", exe.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(String::new())
    }
}
