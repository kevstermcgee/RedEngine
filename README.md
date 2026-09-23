# Red Engine 2 (red_engine2)

A fork of [`forge3d`](../forge3d), made to become the base for an online prop hunt game:
same AI-authorable JSON-scene core, purpose-built from here on for the real-time
first-person viewer (**Red Engine 2** — see below) rather than the offline MP4 renderer,
which is kept around only as a fast way to eyeball scenes/props while authoring them. You
describe a scenario as one compact JSON scene file — primitives, props, a posable humanoid
rig, lights, materials, camera moves, keyframed motion — and it renders with real shading and
shadows, either offline to an MP4 or live in a walk-around window.

**Start here if you're an AI being pointed at this tool: read [`SPEC.md`](SPEC.md).** It's
the complete scene-language reference (coordinates, object types, keyframes, the humanoid
rig) and is written to be read once and then used directly — you shouldn't need to read the
engine's source to use it.

## Why this design

- **JSON in, MP4 out.** The scene format is the entire interface — a data file, cheap to
  generate, cheap to validate, cheap to patch when something's off.
- **World coordinates, not pixels.** Right-handed, Y-up, roughly `-15..15` on X/Z. The
  camera and lights are just objects with position tracks, same as everything else.
- **Sparse keyframes.** Set only what changes, when it changes; the engine interpolates the
  rest with the same easing vocabulary as the 2D engine (`linear`, `in`, `out`, `inout`,
  `hold`, `back`, `bounce`, `elastic`).
- **A posable rig, not just primitives.** `humanoid` is a fixed capsule-and-sphere skeleton
  posed by joint rotations (forward kinematics) — the 3D analog of the 2D engine's
  `stickfigure`. `group` covers everything else you want to build once and move as a unit.
- **Real lighting, not flat shading.** Up to 8 lights (directional/point), one shadow-casting
  sun with a shadow map, Blinn-Phong-ish shading (softened shininess curve + a cheap Fresnel
  rim term) with metallic/roughness controls, a sky gradient background, and Reinhard
  tone-mapping so bright/overlapping lights roll off gracefully instead of blowing out to flat
  white.
- **A lower-level, compiled core.** Rust + [`wgpu`](https://wgpu.rs) instead of a scripting
  language: real GPU rasterization (shadows, per-pixel lighting) at native speed, a type
  system that catches whole classes of scene-schema bugs at compile time, and no interpreter
  overhead when rendering a long clip frame-by-frame. Encoding reuses the 2D engine's
  "boring, mature stack" philosophy — frames are piped straight into a bundled ffmpeg binary
  via `ffmpeg-sidecar` (auto-downloaded, no system ffmpeg install required).
- **Tight, cheap feedback loop.** `validate` catches mistakes with a precise
  `object_id.field` pointer before any render time. `frame` renders one PNG at a given
  timestamp; `storyboard` renders a multi-frame contact sheet — both far cheaper than a full
  render while iterating on layout, pose, or lighting.

## Red Engine 2 — first-person viewer

`re2` is a real-time, walk-around viewer for a scene: it opens a window, drops you in
at the scene camera's position, and lets you look around and walk through the room. It's
built on the same scene schema, mesh generation, and shading pipeline as the offline
renderer (see [`src/viewer.rs`](src/viewer.rs)) — the difference is the camera is driven by
player input every frame instead of a keyframe track, and frames go straight to a window
instead of an MP4.

```bash
cargo run --release --bin re2 -- examples/prop_hunt_yard.json
```

Controls: **WASD** or the **arrow keys** to walk, the **mouse** to look, **Shift** to sprint
forward (with a subtle FOV kick), **Space** for a small jump, **Ctrl** to crouch, **E** or
**left-click** to interact with whatever the crosshair is aimed at, **F** to toggle borderless
fullscreen vs. maximized, **Q** to toggle first-/third-person, click the window to capture the
mouse, **Escape** to release it. The window launches maximized, fit to whichever monitor it
opens on.

This is a viewer, not an editor — there's no real interaction with objects yet (picking things
up, opening doors, etc. is future scope). What's here now: a raycast from the camera finds the
nearest scene object within reach, the crosshair turns gold when one's in range, and E/click
logs it to the console and gives it a brief highlight-glow pulse — a placeholder to build real
interactions on top of. Walls, furniture built from `box` primitives, and every `prop` (one
collider per prop's overall footprint, not per part) block movement (a simple
circle-vs-AABB push-out, axis-aligned); other primitive shapes and the `humanoid` rig don't
collide yet. Any keyframed objects in the scene still animate on their own clock while you walk
around, since only the camera is overridden.

Movement/collision/gravity run on a fixed 60Hz timestep decoupled from the render frame rate,
with the rendered frame interpolating between the last two completed physics states (see
`App::fixed_step_physics`/`App::update` in `src/bin/re2.rs`) — frame-rate-independent and
resistant to tunneling through thin colliders under a frame-time spike. Rendering itself uses
4x MSAA and backface culling (every primitive mesh is a closed solid, verified by
`mesh::tests::all_primitives_are_ccw_front_facing`), plus per-mesh frustum culling against both
the camera's frustum and the shadow-casting light's frustum.

### Props

`props.rs` is a small library of prop-hunt props — schema-level objects
(`{"type": "prop", "prop": "<name>", ...}`) that expand into a handful of primitive parts the
same way `humanoid` expands into a posed capsule rig, so a map author places one object instead
of hand-nesting a dozen boxes. Current set: `crate`, `barrel`, `traffic_cone`, `box_stack`,
`chair`, `trash_can`, `vending_machine`, `bench`, `fire_extinguisher`, `filing_cabinet`,
`potted_plant`, `bookshelf` — see [`examples/prop_hunt_yard.json`](examples/prop_hunt_yard.json)
for all of them placed in one map. Each takes the same one shared `material` every other object
kind does; a few parts (a barrel's rim bands, a potted plant's foliage, ...) get a small
built-in metallic/roughness/color nudge off that base material so the prop doesn't read as one
flat-colored blob, computed in Rust rather than schema-configurable.

## Setup

```bash
cargo build --release
```

That's the whole install for the engine itself — `ffmpeg-sidecar` downloads a static ffmpeg
binary the first time it's needed, so nothing else has to be on `PATH`. The binaries land at
`target/release/red_engine2` (the offline CLI) and `target/release/re2` (the live viewer),
`.exe` on Windows.

For the MCP server: `python -m pip install -r requirements.txt` (Python 3.10+).

## Usage

### CLI

```bash
red_engine2 validate examples/hello_world.json
red_engine2 frame examples/hello_world.json out/check.png --t 1.5
red_engine2 storyboard examples/hello_world.json out/storyboard.png --frames 6
red_engine2 render examples/hello_world.json out/hello_world.mp4
```

### MCP server

```bash
python mcp_server.py
```

Exposes `get_spec`, `list_examples`, `get_example(name)`, `validate_scene(scene_json)`,
`render_frame(scene_json, t)`, `render_storyboard(scene_json, frames)`, and
`render_scene(scene_json, out_path)`. It's a thin wrapper around the compiled binary — build
that first with `cargo build --release`.

To register it with Claude Code, add to your MCP config:

```json
{
  "mcpServers": {
    "red_engine2": {
      "command": "python",
      "args": ["C:\\Users\\TheNa\\ClaudePlayground\\red-engine-2\\mcp_server.py"]
    }
  }
}
```

## Examples

- [`examples/hello_world.json`](examples/hello_world.json) — a bouncing ball, a spinning
  cube, a signpost `group`, a waving `humanoid`, shadows, and a camera dolly.
- [`examples/orbit_walk.json`](examples/orbit_walk.json) — a full camera orbit around a
  `humanoid` walk cycle through a tiny forest of `group`-built trees.
- [`examples/room.json`](examples/room.json) — a small enclosed room (walls, ceiling, a table,
  a shelf, a rug, a `humanoid` greeter) built for `re2` to walk around in; also renders
  fine through the offline pipeline.
- [`examples/prop_hunt_yard.json`](examples/prop_hunt_yard.json) — a larger warehouse/yard
  map populated with the `prop` library below, for prop hunt map iteration.

Render either of the first two and open the resulting `.mp4` to see the offline engine's full
current capability; open `room.json` or `prop_hunt_yard.json` in `re2` to walk around them
instead.

## Project layout

```
src/
  schema.rs     # JSON -> Scene: manual walk with precise "id.field: message" validation errors
  track.rs      # constant-or-keyframed Track<T> + sampling
  easing.rs     # linear/in/out/inout/hold/back/bounce/elastic
  color.rs      # hex -> linear-space RGB
  mesh.rs       # procedural geometry for every primitive (box/sphere/cylinder/cone/capsule/plane)
  skeleton.rs   # humanoid forward-kinematics: joint angles -> world-space capsule segments
  gpu.rs        # wgpu device/pipelines/bind-group-layouts (shadow pass, background, main pass)
  render.rs     # scene -> per-frame GPU draws -> RGB pixels (2x supersampled, then downsampled)
  video.rs      # RGB frames -> ffmpeg -> mp4
  viewer.rs     # Red Engine 2: same pipeline, drawn live into a window surface; player-driven camera + wall collision
  props.rs      # prop-hunt prop library: primitive-composed meshes (crate, barrel, chair, ...)
  shaders/      # WGSL: scene (lit + shadow-sampled), shadow (depth-only), background (sky gradient)
  main.rs       # validate / frame / render / storyboard CLI
  bin/re2.rs    # windowing/input (winit) for the first-person viewer
mcp_server.py   # MCP tool wrapper around the compiled binary
SPEC.md         # the scene-language reference (read this, not the source, to use the tool)
examples/       # runnable example scenes
tests/          # schema/math unit tests (in src/) + an examples-validate integration test
```

## Tests

```bash
cargo test
```

Unit tests cover easing/keyframe math, color parsing, mesh generation (index bounds, unit
normals, and — since the live viewer's pipelines cull backfaces — that every primitive's
triangles are wound consistently with their own stored normals), humanoid forward-kinematics
(symmetry, joint-bend distance checks), and that every prop builds valid parts and round-trips
through its schema name. An integration test parses and validates every bundled example scene.
GPU rendering itself isn't exercised by `cargo test` (no GPU in most CI runners) — use
`frame`/`storyboard`, or launch `re2`, for a manual visual check after render-path changes.

## Known limits (intentional)

No imported meshes or textures, no per-vertex mesh deformation beyond the fixed capsule-rig
`humanoid`, no on-screen 2D text/UI overlay (composite with the 2D engine for captions). Player
physics (the live viewer only) is a simple fixed-timestep circle-vs-AABB model, not a general
physics engine. At most 8 lights and 1 shadow-casting light. No online multiplayer yet — single
local player only; that's the next thing planned on top of this fork. See "Known limits" in
`SPEC.md`.
