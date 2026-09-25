"""MCP server exposing Red Engine 2's offline scene CLI to any MCP-capable AI client.

red_engine2's engine is a compiled Rust binary (for stability/speed/token-efficiency — see
SPEC.md and README.md for why). This server is a thin wrapper: every tool call writes the
scene JSON to a temp file, shells out to the `red_engine2` binary, and reads back its output.
Build the binary once with `cargo build --release` before running this server.

Run directly (stdio transport):
    python mcp_server.py

Tools:
    get_spec()                              -> the scene-language reference (read this first)
    list_examples()                         -> names of all bundled example scenes
    get_example(name)                       -> one of the bundled example scenes, as JSON text
    validate_scene(scene_json)              -> "OK" or precise errors pointing at the bad field(s)
    render_frame(scene_json, t)             -> a single PNG frame, returned inline for a quick check
    render_storyboard(scene_json, ...)      -> multi-frame contact sheet returned inline
    render_scene(scene_json, out_path)      -> renders the full scene to an MP4 file on disk

Map-analysis tools (for walkable maps; see AGENTS.md):
    lint_map(scene_json)                    -> layout problems: overlaps, stairs that lead nowhere, unreachable rooms, ...
    reach_map(scene_json, to="")            -> where the player can walk (floors, zones, doorways); or is x,z[,y] reachable
    walk_route(scene_json, path)            -> replay a walking route with the game's real physics
    plan_map(scene_json, y)                 -> top-down floor plan image (plan_map_ascii for text)
    tour_map(scene_json, only)              -> contact sheet of views of every room / floor
    list_objects(scene_json, filter)        -> objects with world bounds
    list_props()                            -> the prop library (sizes, collision, conventions)
    run_map_tool(args)                      -> any red_engine2 subcommand on files in the project (set/move/add/rm/scatter/...)

Self-description / search / verification (an AI should never need to read the Rust):
    describe(topic)                         -> the engine describing itself (overview, commands, objects, lint, physics, ...)
    search_engine(query)                    -> best fragments across docs, assets, lint codes, recipes, commands, Rust symbols
    catalog(query)                          -> the asset catalogue (props + prefabs), or one asset's params + snippet
    catalog_sheet(query)                    -> labelled contact-sheet image of matching assets
    recipes(name)                           -> known-good example maps (list, or explain one)
    source_lookup(action, query)            -> src map|find|outline|show|refs|deps over the engine source
    verify_map(scene_json)                  -> run the scene's own `checks` block (lint/reach/walk/objects/sim; views skipped)

Game rules, headless play, replay, structured output (the MCP layer is a thin pass-through; the CLI's `--json` envelope is the API):
    describe_brief()                        -> the ~1 KB first read: binaries, workflow, commands, topics
    sim_map(scene_json, scenario_json="")   -> play scripted players through the real simulation (the scene's checks.sim, or one scenario)
    replay_trace(trace_path, scene_path="") -> re-run a recorded match; first divergent tick + state diff
    run_json(args)                          -> any subcommand with the global --json: {schema, command, ok, exit, data, diagnostics, stderr}
"""
from __future__ import annotations
import json
import os
import shutil
import subprocess
import tempfile
from mcp.server.fastmcp import FastMCP, Image

ROOT = os.path.dirname(os.path.abspath(__file__))
mcp = FastMCP("red_engine2")


def _binary_path() -> str:
    exe = "red_engine2.exe" if os.name == "nt" else "red_engine2"
    release = os.path.join(ROOT, "target", "release", exe)
    debug = os.path.join(ROOT, "target", "debug", exe)
    if os.path.isfile(release):
        return release
    if os.path.isfile(debug):
        return debug
    raise RuntimeError(
        "red_engine2 binary not found. Build it first: `cargo build --release` in " + ROOT
    )


def _run(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [_binary_path(), *args], capture_output=True, text=True, cwd=ROOT
    )


def _write_scene_tempfile(scene_json: str) -> str:
    json.loads(scene_json)  # fail fast on malformed JSON with a clear Python error
    fd, path = tempfile.mkstemp(suffix=".json", prefix="forge3d_scene_")
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        f.write(scene_json)
    return path


@mcp.tool()
def get_spec() -> str:
    """Return the red_engine2 scene-language reference (SPEC.md). Read this before writing a scene."""
    with open(os.path.join(ROOT, "SPEC.md"), "r", encoding="utf-8") as f:
        return f.read()


@mcp.tool()
def list_examples() -> list[str]:
    """Return the list of names of all bundled example scenes."""
    directory = os.path.join(ROOT, "examples")
    return sorted(f[:-5] for f in os.listdir(directory) if f.endswith(".json"))


@mcp.tool()
def get_example(name: str) -> str:
    """Return a bundled example scene's raw JSON as text."""
    directory = os.path.join(ROOT, "examples")
    names = sorted(f[:-5] for f in os.listdir(directory) if f.endswith(".json"))
    if name not in names:
        raise ValueError(f"no example named '{name}'. available: {names}")
    with open(os.path.join(directory, f"{name}.json"), "r", encoding="utf-8") as f:
        return f.read()


@mcp.tool()
def validate_scene(scene_json: str) -> str:
    """Validate a scene given as a JSON string. Returns 'OK' or precise errors naming the
    offending object id / field / keyframe so they can be fixed in one pass."""
    scene_path = _write_scene_tempfile(scene_json)
    try:
        result = _run("validate", scene_path)
        if result.returncode == 0:
            return "OK"
        return result.stderr.strip() or result.stdout.strip() or "INVALID (no details returned)"
    finally:
        os.remove(scene_path)


@mcp.tool()
def render_frame(scene_json: str, t: float = 0.0) -> Image:
    """Render a single frame at time t (seconds) and return it inline as an image, so the
    scene's look, lighting, and layout can be checked before spending time on a full render."""
    scene_path = _write_scene_tempfile(scene_json)
    out_dir = tempfile.mkdtemp(prefix="forge3d_frame_")
    out_path = os.path.join(out_dir, "frame.png")
    try:
        result = _run("frame", scene_path, out_path, "--t", str(t))
        if result.returncode != 0:
            raise RuntimeError(result.stderr.strip() or result.stdout.strip())
        with open(out_path, "rb") as f:
            return Image(data=f.read(), format="png")
    finally:
        os.remove(scene_path)
        shutil.rmtree(out_dir, ignore_errors=True)


@mcp.tool()
def render_storyboard(scene_json: str, frames: int = 6) -> Image:
    """Render a multi-frame storyboard contact sheet showing the entire scene's progression,
    returned inline as an image, so the full arc of the animation can be reviewed in one call."""
    scene_path = _write_scene_tempfile(scene_json)
    out_dir = tempfile.mkdtemp(prefix="forge3d_storyboard_")
    out_path = os.path.join(out_dir, "storyboard.png")
    try:
        result = _run("storyboard", scene_path, out_path, "--frames", str(frames))
        if result.returncode != 0:
            raise RuntimeError(result.stderr.strip() or result.stdout.strip())
        with open(out_path, "rb") as f:
            return Image(data=f.read(), format="png")
    finally:
        os.remove(scene_path)
        shutil.rmtree(out_dir, ignore_errors=True)


@mcp.tool()
def render_scene(scene_json: str, out_path: str) -> str:
    """Render the full scene to an MP4 file at out_path (absolute path, or relative to the
    engine's project directory). Returns a short summary on success."""
    scene_path = _write_scene_tempfile(scene_json)
    if not os.path.isabs(out_path):
        out_path = os.path.join(ROOT, out_path)
    os.makedirs(os.path.dirname(out_path), exist_ok=True)
    try:
        result = _run("render", scene_path, out_path)
        if result.returncode != 0:
            raise RuntimeError(result.stderr.strip() or result.stdout.strip())
        return result.stdout.strip() or f"wrote {out_path}"
    finally:
        os.remove(scene_path)


def _run_on_scene(scene_json: str, *args: str) -> subprocess.CompletedProcess:
    """Write the scene to a temp file, run `red_engine2 <args with {scene}> ...`, clean up."""
    scene_path = _write_scene_tempfile(scene_json)
    try:
        return _run(*[scene_path if a == "{scene}" else a for a in args])
    finally:
        os.remove(scene_path)


def _text(result: subprocess.CompletedProcess) -> str:
    out = (result.stdout or "").strip()
    err = (result.stderr or "").strip()
    return "\n".join(x for x in (out, err) if x) or "(no output)"


@mcp.tool()
def lint_map(scene_json: str) -> str:
    """Check a walkable map for layout problems (stairs that lead nowhere, unreachable rooms or
    floors, props overlapping walls, floating props, low ceilings, missing railings, gaps in the
    perimeter, doors blocked by furniture). Returns one line per finding plus a summary; empty of
    ERRORs means the level is playable. Silence an intentional finding with "lint_ignore" on the
    object. Read AGENTS.md for the full workflow."""
    return _text(_run_on_scene(scene_json, "lint", "{scene}"))


@mcp.tool()
def reach_map(scene_json: str, to: str = "") -> str:
    """Where can the player actually walk? Lists reachable floors, per-zone coverage, the doorways
    between zones, stairs status, drops and perimeter leaks. Pass to="x,z" or "x,z,y" to instead
    ask whether one spot is reachable from the spawn (and at what standing heights)."""
    args = ["reach", "{scene}"] + (["--to", to] if to else [])
    return _text(_run_on_scene(scene_json, *args))


@mcp.tool()
def walk_route(scene_json: str, path: str, start: str = "") -> str:
    """Replay a walking route through the map with the game's own per-tick physics. `path` is
    waypoints "x,z; x,z; ..." (meters, +Z toward the back of the map). Reports where the player
    actually ends up (position and floor height) at each waypoint, or where they get stuck."""
    args = ["walk", "{scene}", "--path", path] + ([f"--from={start}"] if start else [])
    return _text(_run_on_scene(scene_json, *args))


@mcp.tool()
def plan_map(scene_json: str, y: float = 0.0, bounds: str = "") -> Image:
    """Top-down floor plan (image) of one level at floor height `y` (0 = ground floor, e.g. 3.0
    for an upper floor): walls, props labelled with ids, stairs with an up-arrow, the walkable area
    in cyan, lights, spawn and lint findings. `bounds` zooms to "x0,z0,x1,z1". +X is right and +Z
    is down the image. Use plan_map_ascii for a compact text version."""
    scene_path = _write_scene_tempfile(scene_json)
    out_dir = tempfile.mkdtemp(prefix="re2_plan_")
    out_path = os.path.join(out_dir, "plan.png")
    try:
        args = ["plan", scene_path, out_path, "--y", str(y)] + ([f"--bounds={bounds}"] if bounds else [])
        result = _run(*args)
        if result.returncode != 0:
            raise RuntimeError(_text(result))
        with open(out_path, "rb") as f:
            return Image(data=f.read(), format="png")
    finally:
        os.remove(scene_path)
        shutil.rmtree(out_dir, ignore_errors=True)


@mcp.tool()
def plan_map_ascii(scene_json: str, y: float = 0.0, bounds: str = "", cell: float = 0.5) -> str:
    """Text floor plan (cheap on tokens) of one level at floor height `y`: `#` wall, letters =
    props (first letter of the kind, legend at the end), `^` stairs, `.` walkable, `,` floor the
    player can't reach, `!` a lint finding. `cell` is meters per character; `bounds` zooms."""
    args = ["plan", "{scene}", "--ascii", "--y", str(y), "--ascii-cell", str(cell)] + ([f"--bounds={bounds}"] if bounds else [])
    return _text(_run_on_scene(scene_json, *args))


@mcp.tool()
def tour_map(scene_json: str, only: str = "", cols: int = 3) -> Image:
    """A labelled contact sheet of rendered views: an exterior overview, a cutaway of every floor
    (roof/upper floors removed), and two views of every zone defined in the scene's `zones`.
    `only` keeps just the views whose label contains that text (e.g. "kitchen")."""
    scene_path = _write_scene_tempfile(scene_json)
    out_dir = tempfile.mkdtemp(prefix="re2_tour_")
    out_path = os.path.join(out_dir, "tour.png")
    try:
        args = ["tour", scene_path, out_path, "--cols", str(cols)] + (["--only", only] if only else [])
        result = _run(*args)
        if result.returncode != 0:
            raise RuntimeError(_text(result))
        with open(out_path, "rb") as f:
            return Image(data=f.read(), format="png")
    finally:
        os.remove(scene_path)
        shutil.rmtree(out_dir, ignore_errors=True)


@mcp.tool()
def list_objects(scene_json: str, filter: str = "", kind: str = "") -> str:
    """List the scene's objects with their world-space bounds. `filter` is a substring/glob on the
    id; `kind` is prop | box | stairs | wall | fence | plane | a prop name (e.g. sofa)."""
    args = ["ls", "{scene}"] + (["--filter", filter] if filter else []) + (["--kind", kind] if kind else [])
    return _text(_run_on_scene(scene_json, *args))


@mcp.tool()
def list_props() -> str:
    """The prop library: every prop kind with its size, how it collides, and the placement
    conventions (origin at the base, front faces +Z)."""
    return _text(_run("props"))


@mcp.tool()
def describe(topic: str = "overview") -> str:
    """The engine describing itself. Topics: brief, overview (default), commands, objects, scene, lint,
    physics, conventions, rules, sim, diagnostics, glossary, decisions, all. Start with `brief`."""
    return _text(_run("describe", topic))


@mcp.tool()
def describe_brief() -> str:
    """The cheapest first read (~1 KB): what the engine is, its binaries, the workflow, every command
    and the topics to ask for next."""
    return _text(_run("describe", "--brief"))


@mcp.tool()
def sim_map(scene_json: str, scenario_json: str = "") -> str:
    """Play scripted players through the real authoritative simulation, headless, and report PASS/FAIL
    with evidence. With no scenario_json it runs the scene's own `checks.sim`; otherwise the given
    scenario (one object or an array). See `describe sim` for the scenario format."""
    scene_path = _write_scene_tempfile(scene_json)
    scenario_path = None
    try:
        args = ["sim", scene_path]
        if scenario_json:
            json.loads(scenario_json)
            fd, scenario_path = tempfile.mkstemp(suffix=".json", prefix="re2_scenario_")
            with os.fdopen(fd, "w", encoding="utf-8") as f:
                f.write(scenario_json)
            args += ["--scenario", scenario_path]
        return _text(_run(*args))
    finally:
        os.remove(scene_path)
        if scenario_path:
            os.remove(scenario_path)


@mcp.tool()
def replay_trace(trace_path: str, scene_path: str = "") -> str:
    """Re-run a recorded match trace (from `sim --trace` or `red_server --record`) with no renderer or
    socket; reports the first divergent tick, which of players/props/rules differ, and a state diff."""
    args = ["replay", trace_path] + (["--scene", scene_path] if scene_path else [])
    return _text(_run(*args))


@mcp.tool()
def run_json(args: list[str]) -> str:
    """Run any red_engine2 subcommand with the global --json flag and return the stable envelope:
    {schema, command, ok, exit, data, diagnostics:[{code, path, message, fix?}], stderr}. Use this
    when you want to parse a result instead of reading text."""
    return _run("--json", *args).stdout


@mcp.tool()
def search_engine(query: str, kind: str = "", limit: int = 8) -> str:
    """Search docs, assets, lint codes, conventions, recipes, CLI commands and Rust symbols in one
    query (e.g. "why does my door block the player"). Returns only the best fragments, each with
    where to read more. kind may be doc|asset|lint|rule|type|recipe|command|src."""
    args = ["search", query, "--limit", str(limit)] + (["--kind", kind] if kind else [])
    return _text(_run(*args))


@mcp.tool()
def catalog(query: str = "", long: bool = False) -> str:
    """List/search the asset catalogue (39 props + ~100 JSON prefabs: food, kitchen, furniture,
    office, school, store, decor, outdoor). A single exact name (e.g. "apple_red") returns that
    asset's size, params and a paste-ready JSON snippet."""
    args = ["catalog"] + query.split() + (["--long"] if long else [])
    return _text(_run(*args))


@mcp.tool()
def catalog_sheet(query: str = "", category: str = "", cols: int = 5) -> Image:
    """Render a labelled contact sheet (PNG) of catalogue assets matching `query`/`category`
    (categories: prop, food, kitchen, furniture, office, school, store, decor, outdoor)."""
    fd, out = tempfile.mkstemp(suffix=".png", prefix="re2_catalog_")
    os.close(fd)
    args = ["catalog"] + query.split() + ["--sheet", out, "--cols", str(cols)] + (["--category", category] if category else [])
    result = _run(*args)
    if result.returncode != 0:
        os.remove(out)
        raise RuntimeError(_text(result))
    try:
        with open(out, "rb") as f:
            return Image(data=f.read(), format="png")
    finally:
        os.remove(out)


@mcp.tool()
def recipes(name: str = "") -> str:
    """Known-good example maps to imitate. No name lists them; a name explains one (what it teaches,
    tips). To start a map from one: run_map_tool(["recipe", name, "--new", "path/mymap.json"])."""
    return _text(_run("recipe", *([name] if name else [])))


@mcp.tool()
def source_lookup(action: str, query: str = "") -> str:
    """Explore the engine's Rust without reading files. action: map (module overview), find <words>,
    outline <file>, show <symbol> (just that item's source), refs <symbol>, deps [module]."""
    if action not in ("map", "find", "outline", "show", "refs", "deps"):
        return "action must be one of: map, find, outline, show, refs, deps"
    return _text(_run("src", action, *query.split()))


@mcp.tool()
def verify_map(scene_json: str) -> str:
    """Run the scene's own top-level "checks" block: lint budget, reachability, real-physics walk
    routes and object assertions (rendered `views` are skipped here; use run_map_tool(["verify",
    path]) on a file for golden-image checks). PASS/FAIL per check with evidence."""
    return _text(_run_on_scene(scene_json, "verify", "{scene}", "--no-views"))


@mcp.tool()
def run_map_tool(args: list[str]) -> str:
    """Run any red_engine2 subcommand on scene files inside the project directory, e.g.
    ["set", "examples/house.json", "sofa_living", "material.color=#aa3322"],
    ["move", "examples/house.json", "sofa_living", "--by", "1,0,0"],
    ["scatter", "examples/house.json", "--zone", "back_yard", "--kind", "bush", "--count", "6"],
    ["lint", "examples/house.json"]. Edit commands validate the whole scene and refuse to write an
    invalid one. Paths are relative to the project root. See AGENTS.md for every command."""
    return _text(_run(*args))


if __name__ == "__main__":
    mcp.run()
