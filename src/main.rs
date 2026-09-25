//! The `red_engine2` CLI: render commands plus every map-analysis/edit/search command (clap definitions and dispatch).

use clap::{Args, Parser, Subcommand};
use glam::{Vec2, Vec3};
use red_engine2::props::PropKind;
use red_engine2::tools::edit::SceneFile;
use red_engine2::tools::envelope;
use red_engine2::tools::gen::{self, LineParams, ScatterParams};
use red_engine2::tools::inspect;
use red_engine2::tools::lint::{self, Severity};
use red_engine2::tools::plan::{self, Labels, PlanOptions};
use red_engine2::tools::reach::{self, ReachParams};
use red_engine2::tools::shots::{self, FrameOpts, View};
use red_engine2::tools::world::{load_or_report, MapWorld};
use red_engine2::tools::{catalog, describe, recipes, search, simrun, symbols, verify};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Instant;

// Every print in this file goes through the envelope capture, so `--json` can wrap any command's output
// (see `tools::envelope`). Without `--json` they are ordinary prints.
macro_rules! print {
    ($($t:tt)*) => { envelope::write_out(format_args!($($t)*)) };
}
macro_rules! println {
    () => { envelope::write_out(format_args!("\n")) };
    ($($t:tt)*) => {{ envelope::write_out(format_args!($($t)*)); envelope::write_out(format_args!("\n")); }};
}
macro_rules! eprintln {
    () => { envelope::write_err(format_args!("\n")) };
    ($($t:tt)*) => {{ envelope::write_err(format_args!($($t)*)); envelope::write_err(format_args!("\n")); }};
}

#[path = "cli/analyze.rs"]
mod analyze;
#[path = "cli/args.rs"]
mod args;
#[path = "cli/editing.rs"]
mod editing;
#[path = "cli/info.rs"]
mod info;
#[path = "cli/render_cmds.rs"]
mod render_cmds;
#[path = "cli/util.rs"]
mod util;

use analyze::*;
use args::*;
use editing::*;
use info::*;
use render_cmds::*;
use util::*;

fn main() {
    use clap::{CommandFactory, FromArgMatches};
    let matches = Cli::command().get_matches();
    let command_name = matches.subcommand_name().unwrap_or("").to_string();
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(c) => c,
        Err(e) => e.exit(),
    };
    if cli.json_output {
        envelope::begin_capture();
    }
    let result = run(cli.command);
    let code = match &result {
        Ok(()) => 0,
        Err(msg) => {
            if !msg.is_empty() {
                eprintln!("{msg}");
            }
            1
        }
    };
    if cli.json_output {
        let (out, err) = envelope::end_capture();
        let doc = serde_json::to_string_pretty(&envelope::envelope(&command_name, code, &out, &err)).unwrap_or_default();
        std::io::Write::write_all(&mut std::io::stdout(), format!("{doc}\n").as_bytes()).ok();
    }
    std::process::exit(code);
}

fn run(command: Command) -> Result<(), String> {
    match command {
        Command::Validate { scene } => run_validate(&scene),
        Command::Frame { scene, out, t, eye, at, fov, hide, cut_above, size } => {
            run_frame(&scene, &out, t, eye.as_deref(), at.as_deref(), fov, hide, cut_above, size.as_deref())
        }
        Command::Render { scene, out } => run_render(&scene, &out),
        Command::Storyboard { scene, out, frames } => run_storyboard(&scene, &out, frames),
        Command::Lint { scene, strict, cell } => run_lint(&scene, envelope::capturing(), strict, cell),
        Command::Reach { scene, from, to, cell } => run_reach(&scene, from.as_deref(), to.as_deref(), cell, envelope::capturing()),
        Command::Walk { scene, path, from, auto, to, cell, explain } => run_walk(&scene, path.as_deref(), from.as_deref(), auto, to.as_deref(), cell, explain.as_deref()),
        Command::Plan { scene, out, y, all_floors, ascii, ascii_cell, scale, bounds, labels, no_reach, no_lint } => {
            run_plan(&scene, out, y, all_floors, ascii, ascii_cell, scale, bounds.as_deref(), &labels, no_reach, no_lint)
        }
        Command::Tour { scene, out, cols, only, view } => run_tour(&scene, &out, cols, only.as_deref(), &view),
        Command::Ls { scene, filter, kind, all } => {
            let json = envelope::capturing();
            load_or_report(&scene).map(|w| print!("{}", inspect::list_objects(&w, filter.as_deref(), kind.as_deref(), all, json)))
        }
        Command::Info { scene, id } => run_info(&scene, &id),
        Command::Describe { topic, brief } => run_describe(topic.as_deref(), brief, envelope::capturing()),
        Command::Search { query, kind, limit } => run_search(&query.join(" "), kind.as_deref(), limit),
        Command::Src { cmd } => run_src(cmd),
        Command::Recipe { name, new, print } => run_recipe(name.as_deref(), new.as_deref(), print),
        Command::Verify { scene, bless, no_views, only, out_dir } => run_verify(&scene, bless, no_views, only, out_dir, envelope::capturing()),
        Command::Diff { a, b, git } => run_diff(&a, b.as_deref(), git),
        Command::Sim { scene, scenario, only, trace, checkpoint_every, dump_every } => {
            run_sim(&scene, scenario.as_deref(), only.as_deref(), trace.as_deref(), checkpoint_every, dump_every)
        }
        Command::Replay { trace, scene, against } => run_replay(&trace, scene.as_deref(), against.as_deref()),
        Command::Catalog { query, tag, category, kind, long, sheet, cols } => {
            let json = envelope::capturing();
            run_catalog(&query, tag.as_deref(), category.as_deref(), kind.as_deref(), long, json, sheet.as_deref(), cols)
        }
        Command::Props {} => {
            print!("{}", inspect::list_props(envelope::capturing()));
            Ok(())
        }
        Command::Set { scene, id, assignments, flags } => edit(&scene, &flags, |f| {
            if assignments.is_empty() {
                return Err("give at least one path=value".to_string());
            }
            for a in &assignments {
                f.set(&id, a)?;
            }
            Ok(format!("set {} field(s) on '{id}'", assignments.len()))
        }),
        Command::Move { scene, id, to, by, flags } => edit(&scene, &flags, |f| {
            match (to.as_deref(), by.as_deref()) {
                (Some(t), None) => f.place_at(&id, vec3(t)?)?,
                (None, Some(b)) => f.translate(&id, vec3(b)?)?,
                _ => return Err("give exactly one of --to x,y,z or --by dx,dy,dz".to_string()),
            }
            Ok(format!("moved '{id}'"))
        }),
        Command::Rm { scene, ids, flags } => edit(&scene, &flags, |f| {
            let mut total = 0;
            for pat in &ids {
                let n = f.remove(pat);
                if n == 0 {
                    return Err(format!("nothing matches '{pat}'"));
                }
                total += n;
            }
            Ok(format!("removed {total} object(s)"))
        }),
        Command::Add { scene, json, file, into, flags } => edit(&scene, &flags, |f| {
            let text = match (json.as_deref(), file.as_deref()) {
                (Some(j), None) => j.to_string(),
                (None, Some(p)) => std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?,
                _ => return Err("give the object as JSON text or with --file".to_string()),
            };
            let v: Value = serde_json::from_str(&text).map_err(|e| format!("not valid JSON: {e}"))?;
            let list = match v {
                Value::Array(a) => a,
                other => vec![other],
            };
            let n = list.len();
            for o in list {
                f.add(&into, o)?;
            }
            Ok(format!("added {n} entr{} to '{into}'", if n == 1 { "y" } else { "ies" }))
        }),
        Command::Clone { scene, id, new_id, by, rot_y, flags } => edit(&scene, &flags, |f| {
            let d = by.as_deref().map(vec3).transpose()?.unwrap_or([0.0; 3]);
            f.clone_object(&id, &new_id, d, rot_y)?;
            Ok(format!("cloned '{id}' as '{new_id}'"))
        }),
        Command::Array { scene, id, count, step, rot_step, flags } => edit(&scene, &flags, |f| {
            let d = vec3(&step)?;
            for k in 1..count {
                let new_id = format!("{id}_{}", k + 1);
                let kf = k as f64;
                f.clone_object(&id, &new_id, [d[0] * kf, d[1] * kf, d[2] * kf], rot_step * kf)?;
            }
            Ok(format!("made {count} copies of '{id}' ({id}, {id}_2, ...)"))
        }),
        Command::Rename { scene, old, new, flags } => edit(&scene, &flags, |f| {
            f.rename(&old, &new)?;
            Ok(format!("renamed '{old}' -> '{new}'"))
        }),
        Command::Fmt { scene, flags } => edit(&scene, &flags, |_| Ok("reformatted".to_string())),
        Command::Scatter { scene, kind, count, rect, zone, exclude, seed, id_prefix, color, scale, min_gap, clearance, y, no_yaw, lint_ignore, flags } => {
            run_scatter(
                &scene,
                &flags,
                &kind,
                count,
                &rect,
                &zone,
                &exclude,
                seed,
                &id_prefix,
                color.as_deref(),
                &scale,
                min_gap,
                clearance,
                y,
                no_yaw,
                lint_ignore.as_deref(),
            )
        }
        Command::Line { scene, kind, from, to, spacing, id_prefix, color, y, scale, lint_ignore, flags } => {
            run_line(&scene, &flags, &kind, &from, &to, spacing, &id_prefix, color, y, scale, lint_ignore.as_deref())
        }
    }
}

// ---- parsing helpers ------------------------------------------------------------------------

// ---- commands ---------------------------------------------------------------------------------
