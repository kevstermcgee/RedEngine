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
    /// Every module: purpose, size, public items. `src map net` lists only the modules under that prefix (much cheaper).
    Map {
        /// Only modules whose name starts with this (`net`, `tools::`, `sim`).
        prefix: Option<String>,
    },
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
pub(crate) enum AdrCmd {
    /// Create `docs/adr/<today>-<slug>.md` from the template and refresh the index.
    New {
        /// The decision, as a short title.
        title: String,
        /// The one sentence that becomes the index row.
        #[arg(long)]
        summary: Option<String>,
        /// accepted (default) or proposed.
        #[arg(long, default_value = "accepted")]
        status: String,
        /// File-name slug (default: made from the title).
        #[arg(long)]
        slug: Option<String>,
    },
    /// One line per ADR: id, status, summary.
    List,
    /// Check that the generated table in `docs/adr/README.md` is current (`--check`, default) or rewrite it (`--write`).
    Index {
        /// Rewrite the table.
        #[arg(long)]
        write: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum AnalysisCmd {
    /// Create `docs/analysis/<today>-<slug>.md`; with `--from FILE` the file's text becomes the body.
    New {
        /// The note's title.
        title: String,
        /// Take the body from this file (a pasted report) instead of the template.
        #[arg(long)]
        from: Option<PathBuf>,
    },
    /// One line per note: date, title, first sentence.
    List,
}

#[derive(Subcommand)]
pub(crate) enum GameCmd {
    /// Project health: every blueprint builds and equals its committed map, every map passes its own `checks`. Exit 1 on any problem.
    Check {
        /// Also run the golden-image view checks (needs a GPU; the default is headless).
        #[arg(long)]
        views: bool,
    },
    /// Compile every blueprint listed in game.json into its map.
    BuildAll,
    /// Print the resolved project: engine pin, blueprints, maps, server settings, commands.
    Info,
    /// Start the headless authoritative server on the project's map (extra arguments go to `red_server`).
    Serve {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        extra: Vec<String>,
    },
    /// Start the graphical client on the project's map, connected to HOST:PORT (default: the local server).
    Play { addr: Option<String> },
    /// Start the project's map directly in local single-player, with no server or network connection.
    PlayLocal,
    /// Ship a version: pin the engine in game.json to the exact commit of an engine checkout (default: the project's local engine path, else the checkout
    /// this binary was built from), so the release's clients and server are built from the same engine. Refuses uncommitted or unpushed engine commits.
    Pin {
        /// The engine checkout to read the commit from.
        #[arg(long)]
        engine: Option<PathBuf>,
        /// Pin HEAD even though tracked files are modified.
        #[arg(long)]
        allow_dirty: bool,
    },
    /// Develop again: point game.json at a local engine checkout (a path relative to the project) instead of a pinned commit.
    Unpin { path: String },
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
        /// Only this pack/category (prop, food, kitchen, furniture, office, school, store, decor, art, lamps, outdoor).
        #[arg(long)]
        category: Option<String>,
        /// prop | prefab
        #[arg(long)]
        kind: Option<String>,
        /// Include a game/project prefab JSON file in discovery (repeatable). Definitions use the same format as assets/*.json.
        #[arg(long = "library")]
        libraries: Vec<PathBuf>,
        /// Print the versioned asset API, pack registry, and growth policy instead of asset rows.
        #[arg(long)]
        manifest: bool,
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
    /// The framework layer: compile a short blueprint (rooms, doors, spawns, prop fill) into a complete, lint-clean scene with walls,
    /// floors, lamps, zones, spawns, portals/interest for multiplayer, and a `checks` block that already passes (incl. auto-planned
    /// walks). `build --example` prints a starter; `build bp.json --check` fails if the committed map is stale.
    Build {
        /// The blueprint JSON (not needed with --example).
        blueprint: Option<PathBuf>,
        /// Where to write the scene (default: next to the blueprint; `x.blueprint.json` -> `x.json`, else `x.map.json`).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Do not write: exit 1 if the existing scene differs from what the blueprint builds (catches stale or hand-edited maps).
        #[arg(long)]
        check: bool,
        /// Print a small working blueprint and exit.
        #[arg(long)]
        example: bool,
    },
    /// What can this machine do? Probes for real: GPU adapter (hardware or software), audio, ffmpeg, UDP loopback and the default server
    /// port, a writable output dir, git. Ends with a plain "what works here" list. Exit 1 only if UDP or the output dir is broken.
    Doctor {
        /// Output directory to probe (default `out`).
        #[arg(long, default_value = "out")]
        out_dir: PathBuf,
    },
    /// Line of sight between two points against the map's real shapes: `ray scene.json --from 0,1.6,-5 --to 4,1.0,6`.
    /// Answers "clear" or the first object in the way (id, kind, distance, point). For hide-and-seek design: can the seeker see the spot?
    Ray {
        scene: PathBuf,
        /// Start "x,y,z" (an eye position: y about 1.6).
        #[arg(long, allow_hyphen_values = true)]
        from: String,
        /// End "x,y,z".
        #[arg(long, allow_hyphen_values = true)]
        to: String,
        /// Object ids to ignore (repeatable), e.g. the floor plane or the prop the target is hiding in.
        #[arg(long)]
        skip: Vec<String>,
    },
    /// Prove a scene's `nav` graph (what bots route along) by travelling every edge with the real movement. Edges use the real tuned physics and
    /// jump pads (`walk` and `drop` both ways, `jump` jumping, `pad` from a real pad through the air), and every node must stand on a
    /// floor outside solid geometry and connect to the first node. `nav scene.json` exits 1 on any problem; `--route FROM TO` prints the route
    /// bots would take; `--all` lists the edges that work too.
    Nav {
        scene: PathBuf,
        /// Print the bots' route between two node ids instead of checking: `--route hall deck`.
        #[arg(long, num_args = 2, value_names = ["FROM", "TO"])]
        route: Vec<String>,
        /// List every edge that passed, with its travel time, not just the failures.
        #[arg(long)]
        all: bool,
    },
    /// Apply many edits atomically (add/set/move/rm/clone/rename) from a JSON list, validated once: it lands completely or not at all.
    /// `patch scene.json --file ops.json` or `patch scene.json '[{"op":"move","id":"lamp_a","by":[0,-0.2,0]}]'`.
    Patch {
        scene: PathBuf,
        /// The patch as JSON text.
        json: Option<String>,
        /// Read the patch from a file instead.
        #[arg(long)]
        file: Option<PathBuf>,
        #[command(flatten)]
        flags: EditFlags,
    },
    /// Render a 2-D screen (menu, pause, connect, lobby, countdown, hud, results) to a PNG with no window or GPU. UI work is never blind:
    /// `ui-shot pause out/pause.png --size 1280x720 --hover resume --message "long text wraps"`.
    UiShot {
        /// Screen name (see `ui-check`'s output or `describe ui`): menu | pause | connect | lobby | countdown | hud | results.
        screen: String,
        /// Output PNG.
        out: PathBuf,
        /// Window size "WxH".
        #[arg(long, default_value = "1280x720")]
        size: String,
        /// Highlight a button: `resume` / `quit` on the pause menu, or an online-screen button id (`ready`, `character`, `leave`, `connect`, `back`, `field_key`).
        #[arg(long)]
        hover: Option<String>,
        /// Pause menu status line (long text wraps).
        #[arg(long)]
        message: Option<String>,
        /// Launch menu selection: human, rat, wizard, cowboy, alien or robot.
        #[arg(long)]
        selected: Option<String>,
        /// Map name shown on the screens.
        #[arg(long, default_value = "test_lab")]
        map: String,
    },
    /// Audit every 2-D screen at 9 window sizes (small, common, portrait, 1440p): everything on screen, inside its container, text not
    /// wider than its panel, no overlaps. Exit 1 on any violation. The same audit runs in `cargo test`.
    UiCheck {
        /// Only this screen.
        #[arg(long)]
        screen: Option<String>,
        /// Only this window size "WxH" (default: all standard sizes).
        #[arg(long)]
        size: Option<String>,
    },
    /// Scaffold a game project that USES the engine (pinned in game.json) instead of forking it: a starter blueprint and the map it builds,
    /// CLAUDE.md, STATUS.md, `scripts/red` (finds/builds the pinned engine) and a CI workflow. The result already passes `game check`.
    NewGame {
        /// Directory to create the project in.
        dir: PathBuf,
        /// Project name (default: the directory name).
        #[arg(long)]
        name: Option<String>,
        /// Use a local engine checkout (relative to the project) instead of cloning one.
        #[arg(long)]
        engine_path: Option<String>,
        /// Engine git URL to pin (default: the official repo).
        #[arg(long)]
        engine_git: Option<String>,
        /// Branch, tag or commit of the engine to pin.
        #[arg(long)]
        engine_ref: Option<String>,
    },
    /// Operate on a game project (the directory holding game.json): check, build-all, info, play-local, serve, play.
    Game {
        #[command(subcommand)]
        cmd: GameCmd,
        /// The project directory.
        #[arg(long, default_value = ".", global = true)]
        dir: PathBuf,
    },
    /// Resume-in-one-screen and keep the docs honest: facts derived from the repo (binaries, features, ADRs, suites), git position,
    /// uncommitted files and the project's STATUS.md handoff. `--init` creates STATUS.md, `--note "..." --section next` appends a
    /// dated bullet, `--sync-docs CLAUDE.md` rewrites the derived-facts region of a doc (tests fail if it is stale).
    Status {
        /// Project root (default: the current directory).
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Create STATUS.md from the template if it does not exist.
        #[arg(long)]
        init: bool,
        /// Append a dated bullet to STATUS.md.
        #[arg(long)]
        note: Option<String>,
        /// Section for --note: now | done | next | blocked | notes.
        #[arg(long, default_value = "done")]
        section: String,
        /// Print only the derived facts block.
        #[arg(long)]
        facts: bool,
        /// Rewrite the `<!-- facts:begin -->` region of these docs with the derived facts (repeatable).
        #[arg(long)]
        sync_docs: Vec<PathBuf>,
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
    /// Repository bookkeeping in about a second, nothing compiled: ADR index, feature ownership, generated docs facts, headless boundary, doc claims, `describe`
    /// budgets, rustfmt. Every problem prints the exact edit; `--fix` makes the mechanical ones. Run it before every commit (the checks CI bounced on, found early).
    Preflight {
        /// Apply the mechanical edits (index, ownership, facts, tool-table rows, `cargo fmt`).
        #[arg(long)]
        fix: bool,
        /// Skip `cargo fmt --check`.
        #[arg(long)]
        no_fmt: bool,
        /// Repository root (default: found from the current directory).
        #[arg(long)]
        root: Option<PathBuf>,
    },
    /// Play a map in the real client with no window and look at it: pictures, a sheet, a report. Runs `re2 --playtest` (host + a scripted player: a spin, a walk, a fight;
    /// pictures from the player's eyes, third person, above the map and behind another player, rendered offscreen so no focus or visible desktop is needed), then prints the
    /// verdict: how many other players were drawn, undrawn, stood in for or hidden by interest management, what was heard, where the contact sheet is. Exit 1 when a player
    /// was left undrawn or an expectation failed. `--script play.json` plays your own script instead (`describe playtest`).
    Playtest {
        /// The map.
        scene: PathBuf,
        /// Seconds of game time to play.
        #[arg(long)]
        secs: Option<f32>,
        /// Pictures to take.
        #[arg(long)]
        shots: Option<usize>,
        /// Where the pictures, `contact-sheet.png` and `playtest.json` go.
        #[arg(long, default_value = "out/playtest")]
        out: PathBuf,
        /// Play this script instead of the generated one.
        #[arg(long)]
        script: Option<PathBuf>,
        /// Join a running server instead of hosting the match here.
        #[arg(long, value_name = "HOST:PORT")]
        connect: Option<String>,
        /// Players the match aims for; empty places are filled with bots.
        #[arg(long, value_name = "N")]
        fill: Option<usize>,
        /// The bots' level: rookie, easy, normal, hard, nightmare, or 0 to 1.
        #[arg(long, value_name = "LEVEL")]
        bot_skill: Option<String>,
        /// Picture size, WxH.
        #[arg(long, value_name = "WxH")]
        size: Option<String>,
    },
    /// Architecture decision records: `adr new "Title"` (a dated file, the index refreshed), `adr list`, `adr index --check|--write`.
    Adr {
        #[command(subcommand)]
        cmd: AdrCmd,
        /// Repository root (default: found from the current directory).
        #[arg(long, global = true)]
        root: Option<PathBuf>,
    },
    /// Analysis notes (`docs/analysis/`): what a builder reported and what was done about it. `analysis new "Title" [--from file]`, `analysis list`; `search --kind analysis`.
    Analysis {
        #[command(subcommand)]
        cmd: AnalysisCmd,
        /// Repository root (default: found from the current directory).
        #[arg(long, global = true)]
        root: Option<PathBuf>,
    },
    /// The feature index: what exists and who owns which file. `docs/features.json` is compiled in: `features` lists them, `features NAME` shows
    /// one, `features WORDS` searches, `features --check` fails when the index names something that does not exist or a source file belongs
    /// to no feature (also a test).
    Features {
        /// A feature name, or words to search for.
        query: Vec<String>,
        /// Validate the index against the repository (files, test suites, docs, ownership of every source file). Exit 1 on any problem.
        #[arg(long)]
        check: bool,
    },
    /// What must pass when files change. Names the features they belong to, the features built on those, the exact `cargo test` and
    /// verification commands and the docs to keep true. `impact src/net/server.rs`, or `impact --git` for the working tree (add a ref to compare with).
    Impact {
        /// Changed files (paths from the repository root).
        files: Vec<String>,
        /// Use `git diff --name-only REF` (default HEAD) plus untracked files instead of listing files.
        #[arg(long, num_args = 0..=1, default_missing_value = "HEAD")]
        git: Option<String>,
    },
    /// A 5-15 KB work packet for one Rust change. It holds the feature's summary and neighbours, its source files with purpose and public API, the tests that cover it,
    /// the verification command and pointers to the relevant ADRs. Name a feature (`features` lists them), a file (`context src/net/server.rs`) or words
    /// (`context lobby countdown`). Use it instead of `src map` + several `src outline`s + reading docs.
    Context {
        /// A feature name, a file path, or words.
        query: Vec<String>,
        /// Also use the features that own the files changed in the working tree (git diff + untracked).
        #[arg(long)]
        git: bool,
        /// Maximum size of the packet in bytes.
        #[arg(long, default_value_t = 12000)]
        budget: usize,
    },
    /// Verify only what a change can affect. Maps the changed files to the features that own them (and are built on them), then runs only
    /// their formatting/lint/unit/integration checks, real-time network suites one at a time, everything else in parallel. Boundary changes
    /// (Cargo.*, src/lib.rs, CI files, huge diffs) escalate to `scripts/ci.sh`. A passing run is remembered by content hash: asking again with nothing edited
    /// costs nothing. `--quick` = only the owning features (edit loop); default = plus dependents (before "done"); `--full` = CI (before pushing). Logs: out/logs/.
    Affected {
        /// Changed files (default: everything changed since the merge base with origin/main, plus the working tree).
        files: Vec<String>,
        /// Compare with this git ref instead of the merge base with origin/main.
        #[arg(long)]
        base: Option<String>,
        /// Only the features that own the changed files (seconds); the dependents are listed as not run.
        #[arg(long)]
        quick: bool,
        /// Run everything CI runs (`scripts/ci.sh`).
        #[arg(long)]
        full: bool,
        /// Print the plan and stop.
        #[arg(long)]
        dry_run: bool,
        /// Do not stop at the first failing step.
        #[arg(long)]
        keep_going: bool,
        /// Ignore and do not write the green stamp (`out/.affected-green.json`).
        #[arg(long)]
        no_cache: bool,
    },
    /// A release you can prove. Builds the client + CLI (default features) and the dedicated server + bot (`--no-default-features`, its own
    /// target directory so no graphics stack can leak in), and writes one reproducible zip of the tracked source and the binaries with a
    /// `PACKAGE-MANIFEST.json` (SHA-256 of every file, commit, dirty files, Cargo.lock hash, toolchain). Refuses a dirty tree unless
    /// `--allow-dirty`. `package --verify X.zip` re-checks every hash and that the headless binaries hold no graphics code.
    Package {
        /// The zip to write (or, with --verify, to check).
        zip: PathBuf,
        /// Check an existing package instead of making one.
        #[arg(long)]
        verify: bool,
        /// Package a tree with uncommitted changes (they are recorded in the manifest).
        #[arg(long)]
        allow_dirty: bool,
        /// Do not build: use the binaries a previous `package` left in target/package-gui and target/package-headless.
        #[arg(long)]
        no_build: bool,
    },
    /// Host from a home PC: open the game's UDP port on your router with UPnP. No router password, no manual port forwarding. `portmap status`
    /// finds the router by SSDP and says what is possible from here (and warns about carrier-grade NAT, which no mapping can fix); `enable`
    /// maps the port with a lease and prints the address a friend joins; `remove` deletes only the mapping this tool made; `keep` renews it
    /// until Ctrl-C and then removes it. It never overwrites or deletes a mapping that is not its own and refuses permanent leases unless allowed.
    Portmap {
        /// status | enable | remove | keep
        action: String,
        /// The UDP port (default: the server's default).
        #[arg(long, default_value_t = red_engine2::net::DEFAULT_PORT)]
        port: u16,
        /// Lease in seconds; the router forgets an unrenewed mapping after this.
        #[arg(long, default_value_t = red_engine2::net::upnp::DEFAULT_LEASE_SECS)]
        lease: u32,
        /// Ask this gateway address instead of searching the network.
        #[arg(long)]
        router: Option<std::net::IpAddr>,
        /// Accept a router that only creates permanent mappings (then remove it yourself when done).
        #[arg(long)]
        allow_permanent: bool,
    },
    /// Performance as a contract. `N` real players walk the scene in an in-process server and the result is judged against a budget.
    /// Reports microseconds per sim tick and per whole server tick (mean, p50, p95, p99, worst; best of several windows, because noise only
    /// ever adds time), bytes per client per second, the largest datagram and how many props physics promoted. Budgets come from
    /// the scene's `checks.perf` (also run by `verify`) or `--budget file.json` (a `{"sim_tick_p95_us": ...}` block). Exit 1 if one is blown.
    Perf {
        scene: PathBuf,
        /// Players walking (1..8). Default: the scene's `checks.perf.players`, else 4.
        #[arg(long)]
        players: Option<usize>,
        /// Seconds per measurement window. Default: the scene's `checks.perf.secs`, else 3.
        #[arg(long)]
        secs: Option<f64>,
        /// Windows measured; each figure is the best of them. Default: 3.
        #[arg(long)]
        windows: Option<usize>,
        /// A JSON file holding a perf budget block (overrides the scene's).
        #[arg(long)]
        budget: Option<PathBuf>,
    },
    /// Make a QUIC server identity. A self-signed certificate and private key (ADR 0044) go to `DIR/cert.pem` and `DIR/key.pem`, and the
    /// SHA-256 fingerprint clients pin (`--server-fingerprint`) is printed. Never commit the key; an existing key is never overwritten.
    /// `red_server --tls-cert DIR/cert.pem --tls-key DIR/key.pem` uses it.
    NetIdentity {
        /// Directory to write into (created; must not already hold a key.pem).
        #[arg(long)]
        out: PathBuf,
        /// DNS names or IP addresses clients will connect to (repeatable; default localhost). Only matters for CA-style verification.
        #[arg(long = "name")]
        names: Vec<String>,
    },
    /// Test the game on a bad connection. A real server and real clients (prediction, interpolation, reconnect) run in-process
    /// behind a seeded, bursty-lossy, laggy UDP proxy and are judged on what a player would notice: disconnects, prediction ending on the
    /// server's position, other players gliding instead of teleporting, corrections, bandwidth. `net-test scene.json --profile bad` or
    /// `--profile all`. Exit 1 on any failed check.
    NetTest {
        scene: PathBuf,
        /// Link profile(s): lan | wifi | 4g | bad | awful | all (repeatable, or comma separated). Default: bad (5% bursty loss, 100 ms round trip).
        #[arg(long, value_delimiter = ',', default_value = "bad")]
        profile: Vec<String>,
        /// Players, each behind its own proxy (1..8).
        #[arg(long, default_value_t = 2)]
        players: usize,
        /// Seconds of play per profile.
        #[arg(long, default_value_t = 6.0)]
        secs: f64,
        /// Seed for the simulated losses.
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// Transport: `udp` (development, the default) or `quic` (production: TLS 1.3 with a throwaway identity for the run).
        #[arg(long, default_value = "udp")]
        transport: String,
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
