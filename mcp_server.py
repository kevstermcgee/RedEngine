"""MCP server exposing forge3d to any MCP-capable AI client.

forge3d's engine is a compiled Rust binary (for stability/speed/token-efficiency — see
SPEC.md and README.md for why). This server is a thin wrapper: every tool call writes the
scene JSON to a temp file, shells out to the `forge3d` binary, and reads back its output.
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
"""
from __future__ import annotations
import json
import os
import shutil
import subprocess
import tempfile
from mcp.server.fastmcp import FastMCP, Image

ROOT = os.path.dirname(os.path.abspath(__file__))
mcp = FastMCP("forge3d")


def _binary_path() -> str:
    exe = "forge3d.exe" if os.name == "nt" else "forge3d"
    release = os.path.join(ROOT, "target", "release", exe)
    debug = os.path.join(ROOT, "target", "debug", exe)
    if os.path.isfile(release):
        return release
    if os.path.isfile(debug):
        return debug
    raise RuntimeError(
        "forge3d binary not found. Build it first: `cargo build --release` in " + ROOT
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
    """Return the forge3d scene-language reference (SPEC.md). Read this before writing a scene."""
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


if __name__ == "__main__":
    mcp.run()
