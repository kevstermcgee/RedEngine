//! The command-line definition (clap): the global flags, every subcommand and its arguments. `describe` reads this,
//! so the docs cannot drift from the real flags.

use super::*;

#[derive(Parser)]
#[command(
    name = "red_engine2",
    about = "Render a JSON scene, and analyze/edit maps (lint, reach, plan, tour, ls, set, scatter, ...). See AGENTS.md.",
    after_help = "Typical map-editing loop:  ls -> (set/move/add/rm/scatter) -> lint -> plan / tour -> fix -> repeat.\nRun `red_engine2 <command> --help` for each command's options."
)]
pub(crate) struct Cli {
    /// Wrap the command's result in one stable JSON envelope {schema, command, ok, exit, data, diagnostics, stderr} (see `describe diagnostics`).
    #[arg(long = "json", global = true, id = "json_output")]
    pub(crate) json_output: bool,
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Args, Clone)]
pub(crate) struct EditFlags {
    /// Show what would be written without touching the file.
    #[arg(long)]
    pub(crate) dry_run: bool,
    /// Write the file even if it would no longer validate.
    #[arg(long)]
    pub(crate) force: bool,
}

#[derive(Subcommand)]
pub(crate) enum SrcCmd {
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
pub(crate) enum Command {
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
    },
    /// Replay a walking route with the game's real per-tick physics: `walk scene.json --path "0,-8; 0,1; -0.8,2; -0.8,7.6"`.
    /// Prints where the player ends up at each waypoint, or where they get stuck AND which object stopped them (id, gap,
    /// passage width vs body). `--auto --to X,Z[,Y]` plans the route for you (A* on the reach grid, validated with the real
    /// physics) and prints waypoints to paste; `--explain out.png` draws the route, the stop point and the blocker.
    Walk {
        scene: PathBuf,
        /// Waypoints "x,z; x,z; ..." (semicolon separated). Not needed with --auto.
        #[arg(long, allow_hyphen_values = true)]
        path: Option<String>,
        /// Start "x,z" or "x,z,y" (default: the scene camera / spawn). `y` = a foot height on an upper floor.
        #[arg(long, allow_hyphen_values = true)]
        from: Option<String>,
        /// Plan the route from --from to --to instead of replaying --path.
        #[arg(long)]
        auto: bool,
        /// Destination "x,z" or "x,z,y" for --auto (`y` = wanted floor height).
        #[arg(long, allow_hyphen_values = true)]
        to: Option<String>,
        /// Grid resolution for --auto, metres.
        #[arg(long, default_value_t = 0.1)]
        cell: f32,
        /// Write a plan image of the walk (route, stop point, blockers). Default file: out/walk.png.
        #[arg(long, num_args = 0..=1, default_missing_value = "out/walk.png")]
        explain: Option<PathBuf>,
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
        /// Render the matches as a labelled contact-sheet PNG instead of listing them.
        #[arg(long)]
        sheet: Option<PathBuf>,
        /// Columns in the sheet.
        #[arg(long, default_value_t = 4)]
        cols: u32,
    },
    // ---- self-description, search, verification -----------------------------------------------
    /// The engine describing itself: overview, commands, objects, scene, lint, physics, conventions.
    /// Start here - no source reading needed. `describe --brief` is a ~1 KB summary; add the global `--json` for machine-readable output.
    Describe {
        /// overview (default) | commands | objects | scene | lint | physics | conventions | diagnostics | all
        topic: Option<String>,
        /// A ~1 KB summary of the engine, its commands and where to look next (the cheapest first read for an AI).
        #[arg(long)]
        brief: bool,
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
    /// Run the scene's own `checks` (lint budget, reachability, real-physics walks incl. auto-planned routes, object assertions,
    /// golden-image views, sim scenarios) and PASS/FAIL each with timings. Exit 1 on any failure. `--bless` records new goldens.
    Verify {
        scene: PathBuf,
        /// Record the current render of every view as its golden image.
        #[arg(long)]
        bless: bool,
        /// Skip rendered view checks (no GPU needed).
        #[arg(long)]
        no_views: bool,
        /// Run a subset: a group (`lint`, `reach`, `walk`, `objects`, `views`, `sim`), one entry (`walk[2]`), or any text from a
        /// check's name (`"front door"`). Failing walks print the blocking object and write out/verify/*_explain.png.
        #[arg(long)]
        only: Option<String>,
        /// Where diff images are written (default out/verify).
        #[arg(long)]
        out_dir: Option<PathBuf>,
    },
    /// Run scripted, headless play-throughs against the real authoritative simulation (no window): the scene's
    /// `checks.sim` scenarios, or `--scenario file.json`. Prints PASS/FAIL with evidence; exit 1 on failure.
    Sim {
        scene: PathBuf,
        /// A scenario file (one scenario or an array) instead of the scene's `checks.sim`.
        #[arg(long)]
        scenario: Option<PathBuf>,
        /// Only scenarios whose name contains this.
        #[arg(long)]
        only: Option<String>,
        /// Record the first scenario's run (inputs, events, checksums) to this trace file.
        #[arg(long)]
        trace: Option<PathBuf>,
        /// Checkpoint interval for `--trace`, in ticks (1 = exact first-divergent-tick).
        #[arg(long, default_value_t = 1)]
        checkpoint_every: u32,
        /// State-dump interval for `--trace`, in ticks (the readable diff a divergence prints).
        #[arg(long, default_value_t = 60)]
        dump_every: u32,
    },
    /// Replay a recorded match trace with no renderer or socket and report the first tick where it stops
    /// agreeing with the recording (or `--against` another trace). Exit 1 on divergence.
    Replay {
        trace: PathBuf,
        /// The map the trace was recorded on (default: the one the trace names).
        #[arg(long)]
        scene: Option<PathBuf>,
        /// Compare against another trace of the same match instead of replaying.
        #[arg(long)]
        against: Option<PathBuf>,
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
    Props {},

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
