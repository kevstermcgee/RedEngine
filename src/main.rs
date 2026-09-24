//! The `red_engine2` CLI: render commands plus every map-analysis/edit/search command (clap definitions and dispatch).

use clap::{Args, Parser, Subcommand};
use glam::{Vec2, Vec3};
use red_engine2::props::PropKind;
use red_engine2::tools::{catalog, describe, recipes, search, symbols, verify};
use red_engine2::tools::edit::SceneFile;
use red_engine2::tools::gen::{self, LineParams, ScatterParams};
use red_engine2::tools::inspect;
use red_engine2::tools::lint::{self, Severity};
use red_engine2::tools::plan::{self, Labels, PlanOptions};
use red_engine2::tools::reach::{self, ReachParams};
use red_engine2::tools::shots::{self, FrameOpts, View};
use red_engine2::tools::world::{load_or_report, MapWorld};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Parser)]
#[command(
    name = "red_engine2",
    about = "Render a JSON scene, and analyze/edit maps (lint, reach, plan, tour, ls, set, scatter, ...). See AGENTS.md.",
    after_help = "Typical map-editing loop:  ls -> (set/move/add/rm/scatter) -> lint -> plan / tour -> fix -> repeat.\nRun `red_engine2 <command> --help` for each command's options."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Args, Clone)]
struct EditFlags {
    /// Show what would be written without touching the file.
    #[arg(long)]
    dry_run: bool,
    /// Write the file even if it would no longer validate.
    #[arg(long)]
    force: bool,
}

#[derive(Subcommand)]
enum SrcCmd {
    /// Every module: purpose, size, public items.
    Map,
    /// Where is X defined? Ranked matches with signature and doc.
    Find {
        query: Vec<String>,
        /// fn | struct | enum | trait | const | type | static
        #[arg(long)]
        kind: Option<String>,
        /// Only files whose path contains this.
        #[arg(long)]
        file: Option<String>,
        /// Include #[cfg(test)] code.
        #[arg(long)]
        tests: bool,
        #[arg(long, default_value_t = 12)]
        limit: usize,
    },
    /// The items of one file (pub only unless --all): `src outline viewer`.
    Outline {
        file: String,
        #[arg(long)]
        all: bool,
    },
    /// Print one item's source (bounded): `src show ground_height_at`, `src show PropKind::name`.
    Show {
        symbol: Vec<String>,
        #[arg(long, default_value_t = 70)]
        lines: usize,
    },
    /// Every use of a symbol, grouped by file with the enclosing function.
    Refs {
        symbol: String,
        #[arg(long, default_value_t = 40)]
        limit: usize,
    },
    /// Module dependency edges (uses / used by).
    Deps { module: Option<String> },
    /// Public items missing a `///` doc comment (what `src show`/`search` display instead of source).
    Coverage {
        /// Only files whose path contains this (`--file tools/lint`).
        #[arg(long)]
        file: Option<String>,
        #[arg(long, default_value_t = 40)]
        limit: usize,
    },
}

#[derive(Subcommand)]
enum Command {
    /// Check a scene file for errors before spending any render time.
    Validate { scene: PathBuf },
    /// Render a single PNG frame at time `--t` seconds (fast layout/lighting check). Supports a free
    /// camera, hiding objects, and cutaways so any part of a map can be inspected without editing it.
    Frame {
        scene: PathBuf,
        out: PathBuf,
        #[arg(long, default_value_t = 0.0)]
        t: f32,
        /// Camera position "x,y,z" (overrides the scene camera).
        #[arg(long, allow_hyphen_values = true)]
        eye: Option<String>,
        /// Point to look at "x,y,z".
        #[arg(long, allow_hyphen_values = true)]
        at: Option<String>,
        /// Vertical field of view in degrees.
        #[arg(long)]
        fov: Option<f32>,
        /// Hide top-level objects by id (glob, repeatable): --hide roof --hide 'wall2_*'
        #[arg(long)]
        hide: Vec<String>,
        /// Hide every object whose lowest point is at/above this height (peels off roofs/upper floors).
        #[arg(long)]
        cut_above: Option<f32>,
        /// Output size "WxH" (default: the scene's own).
        #[arg(long)]
        size: Option<String>,
    },
    /// Render the full scene to an MP4.
    Render { scene: PathBuf, out: PathBuf },
    /// Render a multi-frame contact sheet PNG for fast whole-clip review.
    Storyboard {
        scene: PathBuf,
        out: PathBuf,
        #[arg(long, default_value_t = 6)]
        frames: u32,
    },

    // ---- map analysis -------------------------------------------------------------------------
    /// Static map checker: overlaps, floating/sunk props, stairs that lead nowhere, low ceilings,
    /// unprotected drops, perimeter leaks, unreachable rooms/floors/props. Exit 1 on errors.
    Lint {
        scene: PathBuf,
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
        /// Also exit 1 on warnings.
        #[arg(long)]
        strict: bool,
        /// Reachability grid size in meters (smaller = more faithful to tight gaps, slower).
        #[arg(long, default_value_t = 0.1)]
        cell: f32,
    },
    /// Where can the player walk? Floors reached, zone coverage, doorways between zones, drops, leaks.
    Reach {
        scene: PathBuf,
        /// Start "x,z" (default: the scene camera / spawn).
        #[arg(long, allow_hyphen_values = true)]
        from: Option<String>,
        /// Ask whether "x,z" or "x,z,y" can be reached from the start.
        #[arg(long, allow_hyphen_values = true)]
        to: Option<String>,
        #[arg(long, default_value_t = 0.1)]
        cell: f32,
        #[arg(long)]
        json: bool,
    },
    /// Replay a walking route with the game's real per-tick physics: `walk scene.json --path "0,-8; 0,1; -0.8,2; -0.8,7.6"`.
    /// Prints where the player actually ends up (position + floor height) at each waypoint, or where they get stuck.
    Walk {
        scene: PathBuf,
        /// Waypoints "x,z; x,z; ..." (semicolon separated).
        #[arg(long, allow_hyphen_values = true)]
        path: String,
        /// Start "x,z" (default: the scene camera / spawn).
        #[arg(long, allow_hyphen_values = true)]
        from: Option<String>,
    },
    /// Top-down floor plan (PNG, or --ascii to stdout): walls, props with ids, stairs, walkable area, findings.
    Plan {
        scene: PathBuf,
        /// Output PNG (default out/plan_y<height>.png).
        out: Option<PathBuf>,
        /// Floor height to draw (default 0).
        #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
        y: f32,
        /// Draw one plan per reachable floor (out name gets a _y<h> suffix).
        #[arg(long)]
        all_floors: bool,
        /// Print a text plan instead of writing a PNG.
        #[arg(long)]
        ascii: bool,
        /// Meters per character for --ascii.
        #[arg(long, default_value_t = 0.5)]
        ascii_cell: f32,
        /// Pixels per meter.
        #[arg(long, default_value_t = 40.0)]
        scale: f32,
        /// Window "x0,z0,x1,z1" (default: the whole map).
        #[arg(long, allow_hyphen_values = true)]
        bounds: Option<String>,
        /// auto | all | none
        #[arg(long, default_value = "auto")]
        labels: String,
        #[arg(long)]
        no_reach: bool,
        #[arg(long)]
        no_lint: bool,
    },
    /// Labelled contact sheet of auto-generated views: exterior, each floor cut away, each zone from two corners.
    Tour {
        scene: PathBuf,
        out: PathBuf,
        #[arg(long, default_value_t = 3)]
        cols: u32,
        /// Only views whose label contains this text.
        #[arg(long)]
        only: Option<String>,
        /// Custom view instead of the automatic set: "label:ex,ey,ez:tx,ty,tz[:fov]" (repeatable).
        #[arg(long, allow_hyphen_values = true)]
        view: Vec<String>,
    },

    // ---- inspection ---------------------------------------------------------------------------
    /// List objects with their world bounds.
    Ls {
        scene: PathBuf,
        /// Substring or glob on the id.
        #[arg(long)]
        filter: Option<String>,
        /// prop | box | stairs | wall | fence | plane | a prop name (e.g. sofa) ...
        #[arg(long)]
        kind: Option<String>,
        /// One row per leaf piece instead of per top-level object.
        #[arg(long)]
        all: bool,
        #[arg(long)]
        json: bool,
    },
    /// Everything about one object: JSON, world bounds, neighbours, lint findings.
    Info { scene: PathBuf, id: String },
    /// The asset catalogue: every placeable prop and prefab, searchable. `catalog apple` filters,
    /// `catalog apple_red` shows params + a paste-ready snippet, `--sheet out.png` renders a labelled contact sheet.
    Catalog {
        /// Search words (all must match a name, tag, category or description) — or one exact asset name.
        query: Vec<String>,
        /// Only assets carrying this exact tag.
        #[arg(long)]
        tag: Option<String>,
        /// Only this category (prop, food, kitchen, furniture, office, school, store, decor, outdoor).
        #[arg(long)]
        category: Option<String>,
        /// prop | prefab
        #[arg(long)]
        kind: Option<String>,
        /// Print the full table (sizes, tags) even for long lists.
        #[arg(long)]
        long: bool,
        #[arg(long)]
        json: bool,
        /// Render the matches as a labelled contact-sheet PNG instead of listing them.
        #[arg(long)]
        sheet: Option<PathBuf>,
        /// Columns in the sheet.
        #[arg(long, default_value_t = 4)]
        cols: u32,
    },
    // ---- self-description, search, verification -----------------------------------------------
    /// The engine describing itself: overview, commands, objects, scene, lint, physics, conventions.
    /// Start here - no source reading needed. `--json` for machine-readable output.
    Describe {
        /// overview (default) | commands | objects | scene | lint | physics | conventions | all
        topic: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Search docs, assets, lint codes, recipes, commands and Rust symbols at once; prints only the best
    /// fragments with where to read more, e.g. `search <a question in plain words>`.
    Search {
        query: Vec<String>,
        /// doc | asset | lint | rule | type | recipe | command | src
        #[arg(long)]
        kind: Option<String>,
        #[arg(long, default_value_t = 8)]
        limit: usize,
    },
    /// Explore the engine's Rust without reading it: map, find, outline, show, refs, deps.
    Src {
        #[command(subcommand)]
        cmd: SrcCmd,
    },
    /// Known-good example maps: `recipe` lists, `recipe <name>` explains, `--new out.json` copies one to start from.
    Recipe {
        name: Option<String>,
        /// Write the recipe's scene to this file (refuses to overwrite).
        #[arg(long)]
        new: Option<PathBuf>,
        /// Print the recipe's JSON.
        #[arg(long)]
        print: bool,
    },
    /// Run the scene's own `checks` (lint budget, reachability, real-physics walks, object assertions,
    /// golden-image views) and PASS/FAIL each. Exit 1 on any failure. `--bless` records new goldens.
    Verify {
        scene: PathBuf,
        /// Record the current render of every view as its golden image.
        #[arg(long)]
        bless: bool,
        /// Skip rendered view checks (no GPU needed).
        #[arg(long)]
        no_views: bool,
        /// Only checks whose name contains this (lint, reach, walk, objects, view).
        #[arg(long)]
        only: Option<String>,
        /// Where diff images are written (default out/verify).
        #[arg(long)]
        out_dir: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Semantic diff of two scenes by object id (or one scene against git HEAD with --git).
    Diff {
        a: PathBuf,
        b: Option<PathBuf>,
        /// Compare `a` against its committed version in git HEAD.
        #[arg(long)]
        git: bool,
    },
    /// The prop library: sizes, collision behaviour, conventions.
    Props {
        #[arg(long)]
        json: bool,
    },

    // ---- editing ------------------------------------------------------------------------------
    /// Set fields: `set scene.json sofa_1 position.0=2.5 material.color=#aa3322`
    Set {
        scene: PathBuf,
        id: String,
        /// path=value pairs (value is JSON or a bare string)
        assignments: Vec<String>,
        #[command(flatten)]
        flags: EditFlags,
    },
    /// Move an object (walls/fences shift their endpoints): --to x,y,z or --by dx,dy,dz.
    Move {
        scene: PathBuf,
        id: String,
        #[arg(long, allow_hyphen_values = true)]
        to: Option<String>,
        #[arg(long, allow_hyphen_values = true)]
        by: Option<String>,
        #[command(flatten)]
        flags: EditFlags,
    },
    /// Remove objects by id or glob (`rm scene.json 'fence_*' chair_2`).
    Rm {
        scene: PathBuf,
        ids: Vec<String>,
        #[command(flatten)]
        flags: EditFlags,
    },
    /// Add an object from JSON (`--into lights|zones` for those arrays).
    Add {
        scene: PathBuf,
        /// A JSON object (or array of objects).
        json: Option<String>,
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long, default_value = "objects")]
        into: String,
        #[command(flatten)]
        flags: EditFlags,
    },
    /// Duplicate an object under a new id, optionally offset/rotated.
    Clone {
        scene: PathBuf,
        id: String,
        new_id: String,
        #[arg(long, allow_hyphen_values = true)]
        by: Option<String>,
        #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
        rot_y: f64,
        #[command(flatten)]
        flags: EditFlags,
    },
    /// Make N total copies of an object, each offset by a step (ids: <id>_2, <id>_3, ...).
    Array {
        scene: PathBuf,
        id: String,
        #[arg(long)]
        count: u32,
        #[arg(long, allow_hyphen_values = true)]
        step: String,
        #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
        rot_step: f64,
        #[command(flatten)]
        flags: EditFlags,
    },
    /// Rename an object id.
    Rename {
        scene: PathBuf,
        old: String,
        new: String,
        #[command(flatten)]
        flags: EditFlags,
    },
    /// Re-format a scene file into the canonical readable layout.
    Fmt {
        scene: PathBuf,
        #[command(flatten)]
        flags: EditFlags,
    },

    // ---- generators ---------------------------------------------------------------------------
    /// Seeded random placement of props in a region, avoiding walls/furniture/each other.
    Scatter {
        scene: PathBuf,
        /// Prop kind(s), comma separated: tree_oak,bush,flower_patch
        #[arg(long)]
        kind: String,
        #[arg(long)]
        count: usize,
        /// Region "x0,z0,x1,z1" (repeatable).
        #[arg(long, allow_hyphen_values = true)]
        rect: Vec<String>,
        /// Use a named zone's rectangle from the scene's `zones`.
        #[arg(long)]
        zone: Vec<String>,
        /// Keep-out rectangle "x0,z0,x1,z1" (repeatable).
        #[arg(long, allow_hyphen_values = true)]
        exclude: Vec<String>,
        #[arg(long, default_value_t = 1)]
        seed: u64,
        #[arg(long, default_value = "scatter")]
        id_prefix: String,
        /// Colors, comma separated (default: a natural green/etc. with per-item variation).
        #[arg(long)]
        color: Option<String>,
        /// Scale range "lo:hi"
        #[arg(long, default_value = "0.85:1.25")]
        scale: String,
        #[arg(long, default_value_t = 0.4)]
        min_gap: f32,
        #[arg(long, default_value_t = 0.7)]
        clearance: f32,
        /// Height of the surface being planted on.
        #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
        y: f32,
        /// Don't randomize rotation.
        #[arg(long)]
        no_yaw: bool,
        /// Silence lint checks on the generated objects, comma separated (e.g. `unreachable` for a decorative tree line).
        #[arg(long)]
        lint_ignore: Option<String>,
        #[command(flatten)]
        flags: EditFlags,
    },
    /// Evenly spaced props along a line (a hedge row, a line of trees).
    Line {
        scene: PathBuf,
        #[arg(long)]
        kind: String,
        /// "x,z"
        #[arg(long, allow_hyphen_values = true)]
        from: String,
        /// "x,z"
        #[arg(long, allow_hyphen_values = true)]
        to: String,
        /// Center-to-center distance (default 1.8).
        #[arg(long, default_value_t = 1.8)]
        spacing: f32,
        #[arg(long, default_value = "line")]
        id_prefix: String,
        #[arg(long)]
        color: Option<String>,
        #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
        y: f32,
        #[arg(long, default_value_t = 1.0)]
        scale: f32,
        /// Silence lint checks on the generated objects, comma separated.
        #[arg(long)]
        lint_ignore: Option<String>,
        #[command(flatten)]
        flags: EditFlags,
    },
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Validate { scene } => run_validate(&scene),
        Command::Frame { scene, out, t, eye, at, fov, hide, cut_above, size } => {
            run_frame(&scene, &out, t, eye.as_deref(), at.as_deref(), fov, hide, cut_above, size.as_deref())
        }
        Command::Render { scene, out } => run_render(&scene, &out),
        Command::Storyboard { scene, out, frames } => run_storyboard(&scene, &out, frames),
        Command::Lint { scene, json, strict, cell } => run_lint(&scene, json, strict, cell),
        Command::Reach { scene, from, to, cell, json } => run_reach(&scene, from.as_deref(), to.as_deref(), cell, json),
        Command::Walk { scene, path, from } => run_walk(&scene, &path, from.as_deref()),
        Command::Plan { scene, out, y, all_floors, ascii, ascii_cell, scale, bounds, labels, no_reach, no_lint } => {
            run_plan(&scene, out, y, all_floors, ascii, ascii_cell, scale, bounds.as_deref(), &labels, no_reach, no_lint)
        }
        Command::Tour { scene, out, cols, only, view } => run_tour(&scene, &out, cols, only.as_deref(), &view),
        Command::Ls { scene, filter, kind, all, json } => load_or_report(&scene).map(|w| print!("{}", inspect::list_objects(&w, filter.as_deref(), kind.as_deref(), all, json))),
        Command::Info { scene, id } => run_info(&scene, &id),
        Command::Describe { topic, json } => run_describe(topic.as_deref(), json),
        Command::Search { query, kind, limit } => run_search(&query.join(" "), kind.as_deref(), limit),
        Command::Src { cmd } => run_src(cmd),
        Command::Recipe { name, new, print } => run_recipe(name.as_deref(), new.as_deref(), print),
        Command::Verify { scene, bless, no_views, only, out_dir, json } => run_verify(&scene, bless, no_views, only, out_dir, json),
        Command::Diff { a, b, git } => run_diff(&a, b.as_deref(), git),
        Command::Catalog { query, tag, category, kind, long, json, sheet, cols } => {
            run_catalog(&query, tag.as_deref(), category.as_deref(), kind.as_deref(), long, json, sheet.as_deref(), cols)
        }
        Command::Props { json } => {
            print!("{}", inspect::list_props(json));
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
            run_scatter(&scene, &flags, &kind, count, &rect, &zone, &exclude, seed, &id_prefix, color.as_deref(), &scale, min_gap, clearance, y, no_yaw, lint_ignore.as_deref())
        }
        Command::Line { scene, kind, from, to, spacing, id_prefix, color, y, scale, lint_ignore, flags } => {
            run_line(&scene, &flags, &kind, &from, &to, spacing, &id_prefix, color, y, scale, lint_ignore.as_deref())
        }
    };
    if let Err(msg) = result {
        if !msg.is_empty() {
            eprintln!("{msg}");
        }
        std::process::exit(1);
    }
}

// ---- parsing helpers ------------------------------------------------------------------------

fn floats(s: &str) -> Result<Vec<f64>, String> {
    s.split(',').map(|p| p.trim().parse::<f64>().map_err(|_| format!("'{p}' in '{s}' is not a number"))).collect()
}

fn vec3(s: &str) -> Result<[f64; 3], String> {
    let v = floats(s)?;
    if v.len() != 3 {
        return Err(format!("expected x,y,z but got '{s}'"));
    }
    Ok([v[0], v[1], v[2]])
}

fn v3(s: &str) -> Result<Vec3, String> {
    let v = vec3(s)?;
    Ok(Vec3::new(v[0] as f32, v[1] as f32, v[2] as f32))
}

fn v2(s: &str) -> Result<Vec2, String> {
    let v = floats(s)?;
    if v.len() != 2 {
        return Err(format!("expected x,z but got '{s}'"));
    }
    Ok(Vec2::new(v[0] as f32, v[1] as f32))
}

fn rect(s: &str) -> Result<(Vec2, Vec2), String> {
    let v = floats(s)?;
    if v.len() != 4 {
        return Err(format!("expected x0,z0,x1,z1 but got '{s}'"));
    }
    Ok((Vec2::new(v[0].min(v[2]) as f32, v[1].min(v[3]) as f32), Vec2::new(v[0].max(v[2]) as f32, v[1].max(v[3]) as f32)))
}

fn split_list(s: Option<&str>) -> Vec<String> {
    s.map(|v| v.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect()).unwrap_or_default()
}

fn prop_kinds(s: &str) -> Result<Vec<PropKind>, String> {
    s.split(',')
        .map(|n| {
            PropKind::from_name(n.trim()).ok_or_else(|| {
                let names: Vec<&str> = PropKind::ALL.iter().map(|k| k.name()).collect();
                format!("unknown prop '{n}' (run `red_engine2 props`; kinds: {})", names.join(", "))
            })
        })
        .collect()
}

// ---- commands ---------------------------------------------------------------------------------

/// The CLI's own definition as JSON (name, about, args): the source of truth `describe`/`search` read,
/// so command docs can never drift from the real flags.
fn commands_json() -> Value {
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

fn run_describe(topic: Option<&str>, json: bool) -> Result<(), String> {
    print!("{}", describe::render(topic.unwrap_or("overview"), &commands_json(), json)?);
    Ok(())
}

fn run_search(query: &str, kind: Option<&str>, limit: usize) -> Result<(), String> {
    if query.trim().is_empty() {
        return Err("give some words to search for: `red_engine2 search how do stairs work`".to_string());
    }
    let docs = search::corpus(&commands_json());
    let hits = search::search(&docs, query, kind, limit);
    print!("{}", search::render(&hits, query));
    Ok(())
}

fn run_src(cmd: SrcCmd) -> Result<(), String> {
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

fn run_recipe(name: Option<&str>, new: Option<&Path>, print: bool) -> Result<(), String> {
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

fn run_verify(scene: &Path, bless: bool, no_views: bool, only: Option<String>, out_dir: Option<PathBuf>, json: bool) -> Result<(), String> {
    let report = verify::run(scene, &verify::Options { bless, skip_views: no_views, only, out_dir })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report.to_json()).unwrap());
    } else {
        print!("{}", report.render());
    }
    if report.failed() > 0 {
        return Err(String::new());
    }
    Ok(())
}

fn run_diff(a: &Path, b: Option<&Path>, git: bool) -> Result<(), String> {
    let read = |p: &Path| -> Result<Value, String> {
        serde_json::from_str(&std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?).map_err(|e| format!("{}: {e}", p.display()))
    };
    let (old, new) = match (b, git) {
        (Some(b), false) => (read(a)?, read(b)?),
        (None, true) => {
            let abs = a.canonicalize().map_err(|e| format!("{}: {e}", a.display()))?;
            let dir = abs.parent().ok_or("no parent dir")?;
            let file = abs.file_name().ok_or("no file name")?.to_string_lossy().to_string();
            let out = std::process::Command::new("git").arg("-C").arg(dir).arg("show").arg(format!("HEAD:./{file}")).output().map_err(|e| format!("git: {e}"))?;
            if !out.status.success() {
                return Err(format!("git could not show HEAD:{file}: {}", String::from_utf8_lossy(&out.stderr).trim()));
            }
            (serde_json::from_slice(&out.stdout).map_err(|e| format!("HEAD version is not valid JSON: {e}"))?, read(a)?)
        }
        _ => return Err("give two scene files, or one file with --git".to_string()),
    };
    print!("{}", red_engine2::tools::diff::diff(&old, &new).text);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_catalog(query: &[String], tag: Option<&str>, category: Option<&str>, kind: Option<&str>, long: bool, json: bool, sheet: Option<&Path>, cols: u32) -> Result<(), String> {
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

fn run_validate(scene: &Path) -> Result<(), String> {
    match red_engine2::validate_scene_file(scene) {
        Ok(()) => {
            println!("OK: scene is valid");
            Ok(())
        }
        Err(errs) => {
            for e in &errs {
                eprintln!("error: {e}");
            }
            Err(format!("{} error(s)", errs.len()))
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_frame(
    scene: &Path,
    out: &Path,
    t: f32,
    eye: Option<&str>,
    at: Option<&str>,
    fov: Option<f32>,
    hide: Vec<String>,
    cut_above: Option<f32>,
    size: Option<&str>,
) -> Result<(), String> {
    let started = Instant::now();
    let size = match size {
        Some(s) => {
            let (w, h) = s.split_once('x').ok_or("--size must look like 1280x720")?;
            Some((w.parse::<u32>().map_err(|_| "bad width")?, h.parse::<u32>().map_err(|_| "bad height")?))
        }
        None => None,
    };
    let opts = FrameOpts { eye: eye.map(v3).transpose()?, at: at.map(v3).transpose()?, fov, hide, cut_above, size, t };
    shots::render_frame(scene, out, &opts)?;
    println!("wrote {} ({:.2}s)", out.display(), started.elapsed().as_secs_f32());
    Ok(())
}

fn run_render(scene: &Path, out: &Path) -> Result<(), String> {
    let started = Instant::now();
    red_engine2::render_video(scene, out, |done, total| {
        print!("\rrendering frame {done}/{total}");
        use std::io::Write;
        std::io::stdout().flush().ok();
    })
    .map_err(|e| e.to_string())?;
    println!("\nwrote {} ({:.2}s)", out.display(), started.elapsed().as_secs_f32());
    Ok(())
}

fn run_storyboard(scene: &Path, out: &Path, frames: u32) -> Result<(), String> {
    let started = Instant::now();
    red_engine2::render_storyboard_png(scene, out, frames).map_err(|e| e.to_string())?;
    println!("wrote {} ({:.2}s)", out.display(), started.elapsed().as_secs_f32());
    Ok(())
}

fn analyze(scene: &Path, cell: f32) -> Result<(MapWorld, reach::Reach, Vec<lint::Finding>), String> {
    let world = load_or_report(scene)?;
    let r = reach::compute(&world, &ReachParams { cell, ..Default::default() });
    let findings = lint::lint(&world, &r);
    Ok((world, r, findings))
}

fn run_lint(scene: &Path, json: bool, strict: bool, cell: f32) -> Result<(), String> {
    let (world, r, findings) = analyze(scene, cell)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&lint::to_json(&findings)).unwrap());
    } else {
        let solid = world.items.iter().filter(|i| i.is_solid()).count();
        println!("{}: {} objects, {} solid pieces, {} zone(s); walkable {:.0} m^2 across {} floor(s)", scene.display(), world.scene.objects.len(), solid, world.zones.len(), r.total_area(), r.floors().len());
        print!("{}", lint::format_report(&findings));
    }
    let errors = findings.iter().filter(|f| f.sev == Severity::Error).count();
    let warns = findings.iter().filter(|f| f.sev == Severity::Warn).count();
    if errors > 0 || (strict && warns > 0) {
        return Err(String::new());
    }
    Ok(())
}

fn run_reach(scene: &Path, from: Option<&str>, to: Option<&str>, cell: f32, json: bool) -> Result<(), String> {
    let world = load_or_report(scene)?;
    let start = from.map(v2).transpose()?;
    let r = reach::compute(&world, &ReachParams { cell, start, ..Default::default() });
    if !r.start_ok {
        return Err(format!("the start ({:.1}, {:.1}) is inside solid geometry", r.start.x, r.start.y));
    }
    if let Some(t) = to {
        let v = floats(t)?;
        if v.len() < 2 || v.len() > 3 {
            return Err("--to must be x,z or x,z,y".to_string());
        }
        let p = Vec2::new(v[0] as f32, v[1] as f32);
        let heights: Vec<f32> = r.levels_at(p);
        let ok = if v.len() == 3 { r.reachable(p, v[2] as f32, 0.3) } else { !heights.is_empty() };
        println!("({:.1}, {:.1}{}) is {}{}", p.x, p.y, if v.len() == 3 { format!(", y={:.1}", v[2]) } else { String::new() }, if ok { "REACHABLE" } else { "NOT reachable" }, if heights.is_empty() { String::new() } else { format!("  (standing heights there: {})", heights.iter().map(|h| format!("{h:.2}")).collect::<Vec<_>>().join(", ")) });
        return if ok { Ok(()) } else { Err(String::new()) };
    }
    let floors = r.floors();
    let mut zones_json = Vec::new();
    let mut text = String::new();
    text.push_str(&format!("reach from ({:.1}, {:.1}) y={:.2}   grid {:.2} m\n", r.start.x, r.start.y, r.start_y, r.cell));
    text.push_str("floors the player can stand on:\n");
    for (y, area) in &floors {
        text.push_str(&format!("  y={y:.1}   {area:.0} m^2\n"));
    }
    if !world.zones.is_empty() {
        text.push_str("zones:\n");
        for z in &world.zones {
            let (got, free) = r.area_in(&world, z.min, z.max, z.y, 0.35);
            let pct = if free > 0.0 { got / free * 100.0 } else { 0.0 };
            let status = if free < 0.5 { "no floor?" } else if got < 0.5 { "UNREACHABLE" } else if pct < 60.0 { "PARTLY SEALED" } else { "ok" };
            text.push_str(&format!("  {:<18} y={:<4.1} {:>5.1} / {:<5.1} m^2  {:>3.0}%  {}\n", z.id, z.y, got, free, pct, status));
            zones_json.push(serde_json::json!({"zone": z.id, "y": z.y, "reachable_m2": got, "free_m2": free, "status": status}));
        }
        let passages = r.passages(&world.zones);
        if !passages.is_empty() {
            text.push_str("connections (doorways/openings, same floor):\n");
            for (a, b, mid, w) in &passages {
                text.push_str(&format!("  {a} <-> {b}   ~{w:.2} m wide at ({:.1}, {:.1})\n", mid.x, mid.y));
            }
        }
    }
    for it in world.items.iter().filter(|i| i.stairs.is_some()) {
        let s = it.stairs.unwrap();
        let (bot, top) = (s.point(-0.5, 0.0), s.point(s.run + 0.5, 0.0));
        let (bok, tok) = (r.reachable(bot, s.base_y(), 0.2), r.reachable(top, s.base_y() + s.rise, 0.2));
        text.push_str(&format!("stairs '{}': bottom ({:.1},{:.1}) y={:.1} {}   top ({:.1},{:.1}) y={:.1} {}\n", it.top_id, bot.x, bot.y, s.base_y(), if bok { "reachable" } else { "NOT reachable" }, top.x, top.y, s.base_y() + s.rise, if tok { "reachable" } else { "NOT reachable" }));
    }
    let drops = r.drop_clusters();
    if drops.is_empty() {
        text.push_str("drops: none\n");
    } else {
        for (p, from, to, n) in &drops {
            text.push_str(&format!("drop: {:.1} m at ({:.1}, {:.1}) from y={:.1} to y={:.1} (~{n} cells)\n", from - to, p.x, p.y, from, to));
        }
    }
    text.push_str(if r.leaks.is_empty() { "perimeter: sealed (the player cannot leave the map)\n" } else { "perimeter: LEAKS (the player can walk off the map)\n" });
    if json {
        println!("{}", serde_json::json!({"floors": floors.iter().map(|(y, a)| serde_json::json!({"y": y, "area_m2": a})).collect::<Vec<_>>(), "zones": zones_json, "leaks": r.leaks.len(), "drops": drops.len()}));
    } else {
        print!("{text}");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_plan(
    scene: &Path,
    out: Option<PathBuf>,
    y: f32,
    all_floors: bool,
    ascii: bool,
    ascii_cell: f32,
    scale: f32,
    bounds: Option<&str>,
    labels: &str,
    no_reach: bool,
    no_lint: bool,
) -> Result<(), String> {
    let (world, r, findings) = analyze(scene, 0.1)?;
    let findings = if no_lint { vec![] } else { findings };
    let labels = match labels {
        "auto" => Labels::Auto,
        "all" => Labels::All,
        "none" => Labels::None,
        other => return Err(format!("--labels must be auto, all, or none (got '{other}')")),
    };
    let bounds = bounds.map(rect).transpose()?;
    let mut heights = vec![y];
    if all_floors {
        heights = r.floors().iter().map(|f| f.0).collect();
        if heights.is_empty() {
            heights.push(0.0);
        }
    }
    for h in heights {
        let opts = PlanOptions { y: h, scale, bounds, labels, show_reach: !no_reach, show_findings: !no_lint };
        if ascii {
            print!("{}", plan::render_ascii(&world, Some(&r), &findings, &opts, ascii_cell));
            continue;
        }
        let path = match (&out, all_floors) {
            (Some(p), false) => p.clone(),
            (Some(p), true) => {
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("plan");
                p.with_file_name(format!("{stem}_y{h:.1}.png"))
            }
            (None, _) => PathBuf::from(format!("out/plan_y{h:.1}.png")),
        };
        let img = plan::render_png(&world, Some(&r), &findings, &opts);
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
        }
        img.save(&path).map_err(|e| e.to_string())?;
        println!("wrote {} ({}x{})", path.display(), img.width(), img.height());
    }
    if !ascii && !findings.is_empty() {
        println!("findings marked on the plan (numbered in the order of `lint`'s warnings/errors):");
        for (n, f) in findings.iter().filter(|f| f.sev >= Severity::Warn).enumerate() {
            println!("  {}: [{}] {}", n + 1, f.code, f.message);
        }
    }
    Ok(())
}

fn run_walk(scene: &Path, path: &str, from: Option<&str>) -> Result<(), String> {
    let world = load_or_report(scene)?;
    let wps: Vec<Vec2> = path.split(';').filter(|p| !p.trim().is_empty()).map(v2).collect::<Result<_, _>>()?;
    if wps.is_empty() {
        return Err("--path needs at least one waypoint".to_string());
    }
    let start = from.map(v2).transpose()?.unwrap_or(world.spawn);
    let steps = red_engine2::tools::walk::walk(&world, start, &wps);
    print!("{}", red_engine2::tools::walk::format_walk(&steps, wps.len()));
    if steps.len() < wps.len() || steps.last().is_some_and(|l| !l.reached) {
        return Err(String::new());
    }
    Ok(())
}

fn run_tour(scene: &Path, out: &Path, cols: u32, only: Option<&str>, views: &[String]) -> Result<(), String> {
    let started = Instant::now();
    let custom: Option<Vec<View>> = if views.is_empty() {
        None
    } else {
        let mut v = Vec::new();
        for s in views {
            let parts: Vec<&str> = s.split(':').collect();
            if parts.len() < 3 {
                return Err(format!("--view '{s}' must look like label:ex,ey,ez:tx,ty,tz[:fov]"));
            }
            v.push(View {
                label: parts[0].to_string(),
                eye: v3(parts[1])?,
                at: v3(parts[2])?,
                fov: parts.get(3).and_then(|f| f.parse().ok()).unwrap_or(90.0),
                cut_above: None,
            });
        }
        Some(v)
    };
    let labels = shots::tour(scene, out, custom, cols, only)?;
    println!("wrote {} with {} view(s): {} ({:.1}s)", out.display(), labels.len(), labels.join(", "), started.elapsed().as_secs_f32());
    Ok(())
}

fn run_info(scene: &Path, id: &str) -> Result<(), String> {
    let (world, _r, findings) = analyze(scene, 0.1)?;
    print!("{}", inspect::object_info(&world, id, &findings)?);
    Ok(())
}

/// Load a scene file, run `f` on it, save (validating), and report what changed.
fn edit(path: &Path, flags: &EditFlags, f: impl FnOnce(&mut SceneFile) -> Result<String, String>) -> Result<(), String> {
    let mut file = SceneFile::load(path)?;
    let msg = f(&mut file)?;
    file.save(path, flags.dry_run, flags.force)?;
    println!("{msg}  ({})", path.display());
    println!("next: red_engine2 lint {}", path.display());
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_scatter(
    scene: &Path,
    flags: &EditFlags,
    kind: &str,
    count: usize,
    rects: &[String],
    zones: &[String],
    excludes: &[String],
    seed: u64,
    id_prefix: &str,
    color: Option<&str>,
    scale: &str,
    min_gap: f32,
    clearance: f32,
    y: f32,
    no_yaw: bool,
    lint_ignore: Option<&str>,
) -> Result<(), String> {
    let world = load_or_report(scene)?;
    let mut rs: Vec<(Vec2, Vec2)> = rects.iter().map(|r| rect(r)).collect::<Result<_, _>>()?;
    for z in zones {
        let zone = world.zones.iter().find(|zn| &zn.id == z).ok_or_else(|| format!("no zone '{z}' in the scene (zones: {})", world.zones.iter().map(|z| z.id.as_str()).collect::<Vec<_>>().join(", ")))?;
        rs.push((zone.min, zone.max));
    }
    let (lo, hi) = scale.split_once(':').ok_or("--scale must look like 0.9:1.2")?;
    let params = ScatterParams {
        kinds: prop_kinds(kind)?,
        count,
        rects: rs,
        excludes: excludes.iter().map(|r| rect(r)).collect::<Result<_, _>>()?,
        seed,
        id_prefix: id_prefix.to_string(),
        colors: color.map(|c| c.split(',').map(|s| s.trim().to_string()).collect()).unwrap_or_default(),
        scale: (lo.parse().map_err(|_| "bad --scale")?, hi.parse().map_err(|_| "bad --scale")?),
        min_gap,
        clearance,
        y,
        random_yaw: !no_yaw,
        lint_ignore: split_list(lint_ignore),
    };
    let objs = gen::scatter(&world, &params)?;
    edit(scene, flags, |f| {
        let n = objs.len();
        for o in objs {
            f.add("objects", o)?;
        }
        Ok(format!("scattered {n} object(s) (ids {id_prefix}_1 ...; remove them again with: rm '{id_prefix}_*')"))
    })
}

#[allow(clippy::too_many_arguments)]
fn run_line(scene: &Path, flags: &EditFlags, kind: &str, from: &str, to: &str, spacing: f32, id_prefix: &str, color: Option<String>, y: f32, scale: f32, lint_ignore: Option<&str>) -> Result<(), String> {
    let kinds = prop_kinds(kind)?;
    let objs = gen::line(&LineParams { kind: kinds[0], from: v2(from)?, to: v2(to)?, spacing, id_prefix: id_prefix.to_string(), color, y, scale, lint_ignore: split_list(lint_ignore) })?;
    edit(scene, flags, |f| {
        let n = objs.len();
        for o in objs {
            f.add("objects", o)?;
        }
        Ok(format!("placed {n} object(s) (ids {id_prefix}_1 ...; remove them again with: rm '{id_prefix}_*')"))
    })
}

