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
