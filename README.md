# Red Engine 2 (red_engine2)

An AI-first game engine in Rust for online first-person games (it grew out of a JSON-scene 3D renderer and began as the base for a
prop hunt game). The same AI-authorable JSON-scene core, purpose-built for the real-time
first-person viewer (**Red Engine 2** — see below) rather than the offline MP4 renderer,
which is kept around only as a fast way to eyeball scenes/props while authoring them. You
describe a scenario as one compact JSON scene file — primitives, props, a posable humanoid
rig, lights, materials, camera moves, keyframed motion — and it renders with real shading and
shadows, either offline to an MP4 or live in a walk-around window.

**Start here if you're an AI being pointed at this project: read [`AGENTS.md`](AGENTS.md)** (the
workflow: how to inspect, edit, lint and visually review a map with the built-in tools), then
[`SPEC.md`](SPEC.md), the complete scene-language reference (coordinates, object types, walls,
stairs, props, keyframes, the humanoid rig). Both are written to be read once and used directly —
you shouldn't need to read the engine's source to build or change a map.

**Building a game on Red?** Do not fork this repository. Run `red_engine2 new-game ../mygame --name mygame --engine-path ../red-engine-2`
(a project that pins the engine, with a blueprint, a map, `scripts/red`, `STATUS.md` and CI; ADR 0024), read [`CLAUDE.md`](CLAUDE.md) (4 KB) and
`red_engine2 describe --brief` (1 KB) first, and host it with [`docs/HOSTING.md`](docs/HOSTING.md). Why the tooling looks the way it does:
[`docs/analysis/2026-09-24-cheddar-feedback.md`](docs/analysis/2026-09-24-cheddar-feedback.md).

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
- **Real lighting, not flat shading.** Up to 16 lights (directional/point), one shadow-casting
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

## Map-authoring tools

The `red_engine2` CLI doubles as a toolkit for building and reviewing walkable maps — all of it
runs on the *same* collision/ground code as the game, so its answers are what the player will
actually experience. Full reference and workflow in [`AGENTS.md`](AGENTS.md).

```bash
red_engine2 lint  examples/house.json          # overlaps, floating props, stairs that lead nowhere,
                                               # unreachable rooms, missing railings, perimeter leaks, ...
red_engine2 plan  examples/house.json --all-floors   # labelled top-down plan PNG per floor (or --ascii)
red_engine2 tour  examples/house.json out/tour.png   # rendered views of every room + cutaway per floor
red_engine2 walk  examples/house.json --path "0,-8; 0,1; -0.8,2; -0.8,7.6"   # replay a route with real physics
red_engine2 reach examples/house.json          # floors/rooms reached, doorways between rooms, drops, leaks
red_engine2 ls | info | props                  # inspect objects, one object, the prop library
red_engine2 set | move | add | rm | clone | array | rename | fmt    # safe edits (re-validated, atomic)
red_engine2 scatter | line                     # seeded planting: trees, bushes, flowers, hedge rows
red_engine2 frame scene.json out.png --eye x,y,z --at x,y,z --hide roof --cut-above 5.7   # free camera / cutaways
```

The scene language gained matching sugar: a **`wall`** object with `openings` (doors, windows,
arches, with trim and baseboards) and a **`fence`** object along a polyline both expand to plain
boxes at parse time, so nobody hand-computes wall pieces around a doorway again; optional `zones`
name the rooms so every tool can talk about them; and `lint_ignore` marks intentional oddities.
The MCP server (`mcp_server.py`) exposes the same tools to MCP clients.

## Rendering clarity

Lighting is tuned so objects never melt into the wall behind them: a depth-based **post pass**
adds contact ambient occlusion (soft shadows where things meet surfaces) and silhouette outlines,
ambient light is hemispherical, and `wall` gives every doorway a trim frame and every wall a
baseboard. Tunable per scene through the optional `post` block (see `SPEC.md`).

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

On launch a menu asks whether to play the **Human** (an ordinary person with a bat) or **Cheddar the
rat** (small, brownish-grey, brisk, small enough to run under tables; no bat) — click a side or press
`1` / `2`. `--as human|rat` (or `RE2_CHARACTER`) skips the menu.

Controls: **WASD** or the **arrow keys** to walk, the **mouse** to look, **Shift** to sprint
forward (with a subtle FOV kick), **Space** for a small jump, **Ctrl** to crouch, **left-click**
to swing the bat, **F** to toggle borderless
fullscreen vs. maximized, **Q** to toggle first-/third-person, click the window to capture the
mouse, **Escape** to release it and open the pause menu (Resume / Quit game). The window launches maximized, fit to whichever monitor it
opens on.

This is a viewer, not an editor. The seeker's primary action on objects is **hitting them with
the bat**: a raycast from the player's eye against the objects' *real shapes* finds what is
within bat reach, the crosshair turns gold when something is in range, and only a swing that
actually connects plays a thunk and flashes the object (a swing through air is silent).

**The revolver (mouse wheel).** The human's primary weapon is the bat; **scroll the mouse wheel** to
draw the silver revolver (scroll again to go back). **Left-click fires**: a hitscan shot along the
crosshair (80 m), with a muzzle flash, recoil kick, a gunshot, a hit flash on whatever it strikes and a
punch for loose props. Ammo is **infinite by default**; a scene turns on limited ammo with
`"weapons": {"revolver": {"ammo": {"loaded": 6, "reserve": 24}}}` (**R** reloads). Cheddar has no weapons. Online, weapons,
damage, death/respawn and pick-ups run on the server (`sim::interact`).

**Loose props (E).** Both characters can pick up a small prop with **E** (the crosshair turns
green when you are looking at one you can lift), carry it in front of them, and drop it with **E**
again; a dropped prop keeps your momentum and falls, bounces, tumbles and knocks smaller things
over. A person carries chairs, crates, barrels, plants, TVs and everything smaller; Cheddar carries
things about the size of his head (apples, mugs, books) but can shove or bat-knock anything loose.
Walking into a small prop pushes it; a bat hit sends light props flying. While carrying, the human
cannot swing the bat. Physics is [rapier](https://rapier.rs) (see ADR 0012); props sit exactly where
the map put them until something disturbs them. (Right-click is still reserved for the hider's
"choose an object to replicate" — not built yet.) Walls, furniture built from `box` primitives, and every `prop` (one
collider per prop's overall footprint, not per part) block movement (a simple
circle-vs-AABB push-out, axis-aligned); other primitive shapes and the `humanoid` rig don't
collide yet. Any keyframed objects in the scene still animate on their own clock while you walk
around, since only the camera is overridden.

Movement/collision/gravity run on a fixed 60Hz timestep decoupled from the render frame rate,
with the rendered frame interpolating between the last two completed physics states (see
`App::fixed_step_physics`/`App::update` in `src/bin/re2/frame.rs`) — frame-rate-independent and
resistant to tunneling through thin colliders under a frame-time spike. Rendering itself uses
4x MSAA and backface culling (every primitive mesh is a closed solid, verified by
`mesh::tests::all_primitives_are_ccw_front_facing`), plus per-mesh frustum culling against both
the camera's frustum and the shadow-casting light's frustum.

### Props

`props.rs` is a small library of prop-hunt props — schema-level objects
(`{"type": "prop", "prop": "<name>", ...}`) that expand into a handful of primitive parts the
same way `humanoid` expands into a posed capsule rig, so a map author places one object instead
of hand-nesting a dozen boxes. Current set (39 — `red_engine2 props` lists them with sizes and
collision): crates/barrels/cones/boxes, chairs/armchairs/sofa/beds/desks/tables/wardrobe/
nightstand/shelves/rug/tv, kitchen and bathroom fixtures, `mailbox`, `grill`, `picnic_table`,
`fence_section`, and landscaping — `tree_oak`, `tree_pine`, `bush`, `flower_patch`, `hedge`,
`boulder`, `potted_plant`. Every prop's origin is the middle of its base, and only a tree's trunk
blocks the player (flowers and rugs are walk-through). See [`examples/prop_hunt_yard.json`](examples/prop_hunt_yard.json) and
[`examples/house.json`](examples/house.json) for them placed in maps. Each takes the same one
shared `material` every other object kind does; a few parts (a barrel's rim bands, a potted
plant's foliage, ...) get a small built-in metallic/roughness/color nudge off that base
material so the prop doesn't read as one flat-colored blob, computed in Rust rather than
schema-configurable. The whole library is meant to be shared across every map — the same
`chair` or `potted_plant` reappearing in the house, school, office, and store maps is
intentional, not a gap.

### Multi-floor maps and `stairs`

The live viewer supports more than one floor: player gravity targets a dynamic ground height
(`viewer::ground_height_at`) instead of a hardcoded `y=0`, so a second floor (an ordinary `box`
used as a floor slab) is walkable once the player has actually climbed near its height — which
is exactly what a `stairs` object provides, a smooth walkable ramp under a visually stepped
mesh. See [`examples/house.json`](examples/house.json) for a full 2-story layout, and its
`### stairs` section in [`SPEC.md`](SPEC.md) for the schema. Two things worth knowing when
building a multi-floor map: a staircase only "climbs" when walked from its own local `-Z`
(bottom) end — approaching the tall end at ground level is correctly rejected as unreachable —
so route hallways as one-way approaches to a staircase rather than a through-path past it; and
any wall meant to block the far end of a stairwell needs to be built at the *upper* floor's
height, not the lower one's, since wall collision is also height-band-relative to the player's
current floor.

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
      "args": ["C:\\path\\to\\red-engine-2\\mcp_server.py"]
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
- [`examples/house.json`](examples/house.json) — the first real prop hunt map: a 2-story
  suburban home (living room, study, kitchen, dining room, powder room, three bedrooms, two
  bathrooms, a straight staircase with a railed opening) on a fully fenced lot with front/side/back
  yards, patio, garden shed and landscaping (trees, hedges, shrubs, flower beds, a tree line
  outside the fence). Lint-clean, with a real-physics walk test through every room
  (`tests/house_walk.rs`) and a good worked example of every tool. First of a planned four (house,
  school, office, convenience store), all drawing from the same shared `props.rs` library.

Render either of the first two and open the resulting `.mp4` to see the offline engine's full
current capability; open `room.json`, `prop_hunt_yard.json`, or `house.json` in `re2` to walk
around them instead.

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
  viewer.rs     # Red Engine 2: same pipeline, drawn live into a window surface; player-driven camera + multi-floor collision/ground-height + stairs
  props.rs      # prop library: primitive-composed meshes + per-prop collision policy
  macros.rs     # `wall` / `fence` sugar: expands to plain boxes at parse time
  player.rs     # player constants + the movement step shared by re2 and the analysis tools
  tools/        # map tools behind the CLI: world, reach, lint, plan, walk, edit, gen, inspect, shots, font
  audio.rs      # synthesized sound effects (no imported samples) + rodio playback
  shaders/      # WGSL: scene (lit + shadow-sampled), shadow (depth-only), background (sky), postfx (clarity)
  main.rs, cli/ # the `red_engine2` CLI (clap definition + commands, split by theme)
  bin/re2/      # the windowed game (winit): App state, frame loop, events, weapons, avatar, window
  bin/red_server.rs, bin/red_bot.rs   # the headless authoritative server and scripted client (build with --no-default-features)
  sim/          # headless simulation: match, interactions, interest, game rules, scenarios, traces/replay
  net/          # UDP protocol, server, client, prediction, interpolation
mcp_server.py   # MCP tool wrapper around the compiled binary (render + lint/plan/reach/walk/tour/edit tools)
AGENTS.md       # START HERE (AI agents): the map-editing workflow, tool reference, conventions
SPEC.md         # the scene-language reference (read this, not the source, to use the tool)
examples/       # runnable example scenes
tests/          # schema/math unit tests (in src/) + an examples-validate integration test
```

## Tests

```bash
cargo test
```

Unit tests (60+) cover easing/keyframe math, color parsing, mesh generation (index bounds, unit
normals, and — since the live viewer's pipelines cull backfaces — that every primitive's
triangles are wound consistently with their own stored normals), humanoid forward-kinematics
(symmetry, joint-bend distance checks), that every prop builds valid parts and round-trips
through its schema name, and the multi-floor ground-height mechanism itself (`viewer::
ground_tests` — climbing/descending a ramp smoothly, and both "unreachable" rejection cases) —
that last one directly drives the same per-tick clamp the live viewer uses, deliberately not
relying on simulated window input, which turned out to be too flaky in practice to trust for
anything beyond short, simple interactions. An integration test parses and validates every
bundled example scene, and `tests/house_walk.rs` walks real routes through every room of
`examples/house.json` — front door, each ground-floor room, up the stairs into every bedroom,
out to the shed — using the game's per-tick physics, so a layout edit that seals a door or breaks
the staircase fails `cargo test` instead of reaching a player. The map tools have their own unit
tests (lint checks, reachability, wall/fence expansion, edit round-trips, scatter determinism). GPU rendering itself isn't exercised by `cargo test` (no GPU in most CI
runners) — use `frame`/`storyboard`, or launch `re2`, for a manual visual check after
render-path changes.

## Known limits (intentional)

No imported meshes, textures, or audio samples (sound effects are synthesized in code — see
`audio.rs` — same reasoning as the procedural meshes), no per-vertex mesh deformation beyond
the fixed capsule-rig `humanoid`, no sloped roofs (flat ceiling/roof slabs only). The offline
renderer draws no 2-D text; the live game's menus, lobby and HUD come from the headless-audited
UI kit (`src/ui`, ADR 0026). Player physics is a simple fixed-timestep circle-vs-AABB-plus-ground-height
model, not a general physics engine — walking up multiple floors via `stairs` works, but there is no
jumping between floors, ladders, or slopes other than stairs. At most 16 lights and 1 shadow-casting
light (point lights don't cast shadows).

Multiplayer exists (ADR 0016, 0022, 0028): an authoritative headless UDP server (`red_server`), a
graphical client (`re2 --connect`), a scripted bot (`red_bot`), client prediction, interpolation, per-client
acknowledged deltas and spatial interest management. What it does **not** have: lag compensation for
hitscan, and payload confidentiality (join keys are proven by challenge-response and every datagram is
authenticated, but not encrypted). See "Known limits" in `SPEC.md` and `docs/HOSTING.md`.

## Hosting, testing and shipping a multiplayer game

`red_server` is one small headless binary (no graphics crates). `--key auto` makes joining need a key that clients prove without sending it,
and every datagram is authenticated (ADR 0028: authenticated, **not** encrypted). `--lobby` (or a scene `"match"` block) adds a lobby, ready-up,
countdown, timed rounds, results and rematch, which the `re2` client shows as a connect form, lobby, HUD and results screen (ADR 0029).
`--upnp` opens the port on a home router (ADR 0031). Before you ship: `red_engine2 net-test map.json --profile bad` (does it play on a bad
connection?), `red_engine2 perf map.json` (does it hold N players inside its `checks.perf` budget?), `red_engine2 impact --git` (which
tests does this change touch?), then `red_engine2 package out.zip` and `package --verify out.zip` for a reproducible, checkable release.
See [`docs/HOSTING.md`](docs/HOSTING.md); `red_engine2 describe multiplayer` is the one-screen version.

## For AI agents: tools that keep engine source out of context

`red_engine2` describes itself and ships the tooling to build maps accurately without reading Rust:
`describe` (self-description), `search` (docs + assets + lint codes + recipes + Rust symbols),
`catalog` (39 props + ~155 JSON prefabs with tags, sizes, params; `--sheet` renders a labelled contact
sheet), `recipe` (four known-good complete maps: house, school wing, convenience store, two rooms),
`verify` (a scene's own `checks`: lint budget, reachability, real-physics walks, object assertions,
golden-image views with diff images), `diff` (semantic scene diff), and `src map|find|show|refs|deps`
(navigate the Rust without reading files). New furniture/food/decor is added as JSON prefabs in
`assets/`, no Rust needed. See `AGENTS.md`.
