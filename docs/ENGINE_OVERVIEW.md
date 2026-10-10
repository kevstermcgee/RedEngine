# Red Engine 2: overview

The long form that the README used to carry: why the engine is built the way it is, the map tools by example, props and floors, how to build and test it, and
its known limits. **It is not required reading.** [`AGENTS.md`](../AGENTS.md) is the single entry point and the engine answers questions itself
(`red_engine2 search "<question>"`, `describe <topic>`). The first-person viewer's prop hunt history is in [`VIEWER_HISTORY.md`](VIEWER_HISTORY.md).

## Why this design

- **JSON in, live game or offline render out.** Scenes are data files: cheap to generate,
  validate and patch. The same scene drives the graphical client, headless simulation,
  analysis tools and optional offline MP4 renderer.
- **World coordinates, not pixels.** Right-handed, Y-up, roughly `-15..15` on X/Z. The
  camera and lights are just objects with position tracks, same as everything else.
- **Sparse keyframes.** Set only what changes, when it changes; the engine interpolates the
  rest with the same easing vocabulary as the 2D engine (`linear`, `in`, `out`, `inout`,
  `hold`, `back`, `bounce`, `elastic`).
- **A posable rig, not just primitives.** `humanoid` is a fixed capsule-and-sphere skeleton
  posed by joint rotations (forward kinematics) — the 3D analog of the 2D engine's
  `stickfigure`. `group` covers everything else you want to build once and move as a unit.
- **Real lighting, not flat shading.** Up to 256 authored lights with the nearest 16 active per view, one shadow-casting
  sun with a shadow map, Blinn-Phong-ish shading (softened shininess curve + a cheap Fresnel
  rim term) with metallic/roughness controls, a sky gradient background, and Reinhard
  tone-mapping so bright/overlapping lights roll off gracefully instead of blowing out to flat
  white.
- **A lower-level, compiled core.** Rust + [`wgpu`](https://wgpu.rs) instead of a scripting
  language: real GPU rasterization (shadows, per-pixel lighting) at native speed, a type
  system that catches whole classes of scene-schema bugs at compile time, and no interpreter
  overhead when rendering a long clip frame-by-frame. MP4 export (the `video` cargo feature, not in the default build) pipes frames
  straight into an ffmpeg process via `ffmpeg-sidecar`, which downloads a static ffmpeg the first time it is needed.
- **Tight, cheap feedback loop.** `validate` catches mistakes with a precise
  `object_id.field` pointer before any render time. `frame` renders one PNG at a given
  timestamp; `storyboard` renders a multi-frame contact sheet — both far cheaper than a full
  render while iterating on layout, pose, or lighting.

## Map-authoring tools

The `red_engine2` CLI doubles as a toolkit for building and reviewing walkable maps — all of it
runs on the *same* collision/ground code as the game, so its answers are what the player will
actually experience. Full reference and workflow in [`AGENTS.md`](../AGENTS.md).

```bash
red_engine2 lint  examples/house.json          # overlaps, floating props, stairs that lead nowhere,
                                               # unreachable rooms, missing railings, perimeter leaks, ...
red_engine2 plan  examples/house.json --all-floors   # labelled top-down plan PNG per floor (or --ascii)
red_engine2 tour  examples/house.json out/tour.png   # rendered views of every room + cutaway per floor
red_engine2 walk  examples/house.json --path "0,-8; 0,1; -0.8,2; -0.8,7.6"   # replay a route with real physics
red_engine2 reach examples/house.json          # floors/rooms reached, doorways between rooms, drops, leaks
red_engine2 ls examples/house.json             # inspect objects and world bounds
red_engine2 info examples/house.json sofa_1    # inspect one object and nearby findings
red_engine2 props                              # list the Rust prop library
red_engine2 set examples/house.json sofa_1 material.color=#aa3322  # safe, re-validated edit
red_engine2 scatter examples/house.json --zone back_yard --kind bush --count 8 --seed 3
red_engine2 frame scene.json out.png --eye x,y,z --at x,y,z --hide roof --cut-above 5.7   # free camera / cutaways
```

The scene language gained matching sugar: a **`wall`** object with `openings` (doors, windows,
arches, with trim and baseboards) and a **`fence`** object along a polyline both expand to plain
boxes at parse time, so nobody hand-computes wall pieces around a doorway again; optional `zones`
name the rooms so every tool can talk about them; and `lint_ignore` marks intentional oddities.
`red_engine2 mcp` serves the same commands to MCP clients (eleven tools, in process).

## Rendering clarity

Lighting is tuned so objects never melt into the wall behind them: a depth-based **post pass**
adds contact ambient occlusion (soft shadows where things meet surfaces) and silhouette outlines,
ambient light is hemispherical, and `wall` gives every doorway a trim frame and every wall a
baseboard. Tunable per scene through the optional `post` block (see `SPEC.md`).

### Props

`props.rs` is a small library of prop-hunt props — schema-level objects
(`{"type": "prop", "prop": "<name>", ...}`) that expand into a handful of primitive parts the
same way `humanoid` expands into a posed capsule rig, so a map author places one object instead
of hand-nesting a dozen boxes. The set (`red_engine2 props` lists it with sizes and
collision): crates/barrels/cones/boxes, chairs/armchairs/sofa/beds/desks/tables/wardrobe/
nightstand/shelves/rug/tv, kitchen and bathroom fixtures, `mailbox`, `grill`, `picnic_table`,
`fence_section`, and landscaping — `tree_oak`, `tree_pine`, `bush`, `flower_patch`, `hedge`,
`boulder`, `potted_plant`. Every prop's origin is the middle of its base, and only a tree's trunk
blocks the player (flowers and rugs are walk-through). See [`examples/prop_hunt_yard.json`](../examples/prop_hunt_yard.json) and
[`examples/house.json`](../examples/house.json) for them placed in maps. Each takes the same one
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
mesh. See [`examples/house.json`](../examples/house.json) for a full 2-story layout, and its
`### stairs` section in [`SPEC.md`](../SPEC.md) for the schema. Two things worth knowing when
building a multi-floor map: a staircase only "climbs" when walked from its own local `-Z`
(bottom) end — approaching the tall end at ground level is correctly rejected as unreachable —
so route hallways as one-way approaches to a staircase rather than a through-path past it; and
any wall meant to block the far end of a stairwell needs to be built at the *upper* floor's
height, not the lower one's, since wall collision is also height-band-relative to the player's
current floor.

### Looking at a game without a screen

The real client can be run, played and photographed by a program. `red_engine2 playtest MAP` hosts the map, joins it with the real client loop (no window), lets a
scripted player spin, walk, aim and fire, and writes pictures from the player's eyes, third person, above the map and behind another player (rendered offscreen: no focus,
no visible desktop) with a labelled contact sheet and a JSON report of what was drawn. `re2 MAP --host --headless --script play.json --dump state.json` is the same loop with your
own script, whose `expect` steps assert on the dumped state (`"8 fighters means 7 drawn"` is one line), and `re2 --debug-help` lists every `RE2_*` switch. Every way a remote
player can silently fail to be drawn is counted (F3 overlay, `RE2_STATS`), `game check` verifies that every body a map's bots wear has an avatar, and `lint` reports zones without portals and
slabs a jump shoves the player under. See `red_engine2 describe playtest` and `docs/adr/2026-09-28-seeing-what-the-player-sees.md`.

## Setup

```bash
cargo build --release                    # the CLI, the client, the server and the bots
cargo build --release --features video   # also MP4 export (`red_engine2 render`); not in the default build
```

That's the whole install for the engine itself. The binaries land at `target/release/red_engine2` (the CLI) and
`target/release/re2` (the live viewer), `.exe` on Windows. For daily work use `scripts/dev` (debug builds, a seeded `target/`)
rather than `--release`; `AGENTS.md` has the ladder.

## Usage

### CLI

```bash
red_engine2 validate examples/hello_world.json
red_engine2 frame examples/hello_world.json out/check.png --t 1.5
red_engine2 storyboard examples/hello_world.json out/storyboard.png --frames 6
# MP4 export needs a build with the `video` feature: cargo build --release --features video
red_engine2 render examples/hello_world.json out/hello_world.mp4
```

### MCP server

```bash
red_engine2 mcp          # stdio; the client starts it
```

A native MCP server: <!--fact:mcp-tools-->11<!--/fact--> tools (`describe`, `search`, `context`, `validate`, `lint`, `analyze`, `patch`, `verify`, `sim`, `view`, `run`) that run the CLI's own commands
in the server's process, with typed argument schemas. Files are passed by path in the server's working directory, so a scene is never sent as text; `view` returns a
picture; `run` reaches every other command (servers and interactive play are refused). `tools/list` is about 5 KB, a fifth of the Python adapter it replaced.
No Python packages are needed. The workflow's first command, `scripts/dev start`, is a shell command (a Python script), not an MCP tool: run it from the shell.

To register it with Claude Code, add to your MCP config:

```json
{
  "mcpServers": {
    "red_engine2": { "command": "red_engine2", "args": ["mcp"] }
  }
}
```

`python mcp_server.py` (a 45-line launcher that finds the executable and runs `red_engine2 mcp`) still works for existing configurations.

## Examples

- [`examples/test_lab.json`](../examples/test_lab.json) — the primary engine-development
  map, with one small area per system under test and self-contained checks.
- [`examples/hello_world.json`](../examples/hello_world.json) — a bouncing ball, a spinning
  cube, a signpost `group`, a waving `humanoid`, shadows, and a camera dolly.
- [`examples/orbit_walk.json`](../examples/orbit_walk.json) — a full camera orbit around a
  `humanoid` walk cycle through a tiny forest of `group`-built trees.
- [`examples/room.json`](../examples/room.json) — a small enclosed room (walls, ceiling, a table,
  a shelf, a rug, a `humanoid` greeter) built for `re2` to walk around in; also renders
  fine through the offline pipeline.
- [`examples/prop_hunt_yard.json`](../examples/prop_hunt_yard.json) — a larger warehouse/yard
  map populated with the `prop` library below, for prop hunt map iteration.
- [`examples/house.json`](../examples/house.json) — the first real prop hunt map: a 2-story
  suburban home (living room, study, kitchen, dining room, powder room, three bedrooms, two
  bathrooms, a straight staircase with a railed opening) on a fully fenced lot with front/side/back
  yards, patio, garden shed and landscaping (trees, hedges, shrubs, flower beds, a tree line
  outside the fence). Lint-clean, with a real-physics walk test through every room
  (`tests/house_walk.rs`) and a good worked example of every tool.
- [`examples/school.json`](../examples/school.json),
  [`examples/office.json`](../examples/office.json), and
  [`examples/store.json`](../examples/store.json) — the other three legacy reference maps.
  All four draw from the same prop/prefab library and carry verification checks.
  `tests/maps_verify.rs` covers these three maps, while `tests/house_walk.rs` covers
  house routes and `tests/prop_physics.rs` covers prop behavior across all four.

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
  viewer.rs     # the live renderer: `render_view` draws the world from any camera; re2's weapon/crosshair layers are optional
  app/          # the client layer for custom games: ViewCamera + picking, LocalSession (MatchSim), input, window GPU, HUD, offscreen checks
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
crates/red2d/   # the 2D game crate: a declarative 2D game, its simulation and CPU renderer
mcp_server.py   # launcher for the native MCP server (`red_engine2 mcp`)
AGENTS.md       # START HERE (AI agents): the single entry point: workflow, tool reference, conventions
SPEC.md         # the scene-language reference (read this, not the source, to use the tool)
examples/       # runnable example scenes
tests/          # schema/math unit tests (in src/) + an examples-validate integration test
```

## Tests

```bash
cargo test
```

The test suite covers easing/keyframe math, color parsing, mesh generation (index bounds, unit
normals, and — since the live viewer's pipelines cull backfaces — that every primitive's
triangles are wound consistently with their own stored normals), humanoid forward-kinematics
(symmetry, joint-bend distance checks), that every prop builds valid parts and round-trips
through its schema name, and the multi-floor ground-height mechanism itself
(`collide::ground_tests` — climbing/descending a ramp smoothly, and both "unreachable"
rejection cases) —
that last one directly drives the same per-tick clamp the live viewer uses, deliberately not
relying on simulated window input, which turned out to be too flaky in practice to trust for
anything beyond short, simple interactions. An integration test parses and validates every
bundled example scene, and `tests/house_walk.rs` walks real routes through every room of
`examples/house.json` — front door, each ground-floor room, up the stairs into every bedroom,
out to the shed — using the game's per-tick physics, so a layout edit that seals a door or breaks
the staircase fails `cargo test` instead of reaching a player. The map tools have their own unit
tests (lint checks, reachability, wall/fence expansion, edit round-trips, scatter determinism),
and integration suites cover authoritative networking, authentication, rules, replay,
budgets and repository/doc consistency. GPU rendering itself isn't exercised by `cargo test` (no GPU in most CI
runners) — use `frame`/`storyboard`, or launch `re2`, for a manual visual check after
render-path changes.

## Known limits (intentional)

No imported meshes, textures, or audio samples (sound effects are synthesized in code — see
`audio.rs` — same reasoning as the procedural meshes), no per-vertex mesh deformation beyond
the fixed capsule-rig `humanoid`, no sloped roofs (flat ceiling/roof slabs only). The offline
renderer draws no 2-D text; the live game's menus, lobby and HUD come from the headless-audited
UI kit (`src/ui`, ADR 0026). Player physics is a simple fixed-timestep circle-vs-AABB-plus-ground-height
model, not a general physics engine — walking up multiple floors via `stairs` works, but there is no
jumping between floors, ladders, or slopes other than stairs. At most 256 authored lights, 16 active per view, and 1 shadow-casting
light (point lights don't cast shadows).

Multiplayer exists (ADR 0016, 0022, 0028): an authoritative headless UDP server (`red_server`), a
graphical client (`re2 --connect`), a scripted bot (`red_bot`), client prediction, interpolation, per-client
acknowledged deltas and spatial interest management. Lag compensation for hitscan is built (ADR 0053):
the server rewinds the players a shot can hit by the shooter's view lag. Hosted servers speak QUIC + TLS 1.3 with a pinned server identity (ADR 0044); loopback development uses plain authenticated UDP.
See "Known limits" in `SPEC.md` and `docs/HOSTING.md`.

## Hosting, testing and shipping a multiplayer game

`red_server` is one small headless binary (no graphics crates). `--key auto` makes joining need a key that clients prove without sending it,
and, on the production QUIC transport, every datagram is encrypted and the server verified by its fingerprint (`red_engine2 net-identity`,
`--tls-cert/--tls-key`, clients `--server-fingerprint`; ADR 0044). `--lobby` (or a scene `"match"` block) adds a lobby, ready-up,
countdown, timed rounds, results and rematch, which the `re2` client shows as a connect form, lobby, HUD and results screen (ADR 0029).
`--upnp` opens the port on a home router (ADR 0031). Before you ship: `red_engine2 net-test map.json --profile bad` (does it play on a bad
connection?), `red_engine2 perf map.json` (does it hold N players inside its `checks.perf` budget?), `red_engine2 impact --git` (which
tests does this change touch?), then `red_engine2 package out.zip` and `package --verify out.zip` for a reproducible, checkable release.
See [`docs/HOSTING.md`](HOSTING.md); `red_engine2 describe multiplayer` is the one-screen version.
