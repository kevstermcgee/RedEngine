# Red Engine 2

JSON-scene 3D engine (Rust; wgpu for graphics) with authoritative UDP multiplayer, a headless server, game rules as data and a
self-describing CLI. **You never read engine source, and you never guess:** the engine describes itself, checks your work and
explains its failures. Workflow and tool reference: @AGENTS.md.

## First 60 seconds
```bash
scripts/dev doctor               # toolchain, binaries, git (Windows: powershell -File scripts\dev.ps1 doctor)
R="scripts/dev red"              # builds the CLI on first use, then runs it from any directory
$R doctor                        # what THIS machine can do: GPU or software rendering, UDP, ffmpeg, output dir
$R status                        # resume: facts + git + STATUS.md (what is done / in flight / next)
$R describe --brief              # ~1 KB manual: binaries, workflow, commands, topics; then $R search "<question>"
```

## Making a game (the fast path)
```bash
$R new-game ../mygame --name mygame --engine-path ../red-engine-2   # a project that USES the engine: blueprint + map + scripts/red + CI
cd ../mygame && scripts/red check                                   # green from the first commit
# edit blueprints/main.blueprint.json (rooms, doors, spawns, fill, rules under "scene"), then:
scripts/red build-all && scripts/red check && scripts/red plan maps/main.json   # build, verify, LOOK
scripts/red serve                                                   # headless multiplayer server; `scripts/red play HOST:PORT` joins
```
Do not fork this repository to make a game (the fork's docs and engine fixes drift; ADR 0024). `$R describe rules` covers game logic as data.

## Working on maps
```bash
$R recipe                        # known-good maps; `recipe two_floor_house --new my.json`     $R catalog apple [--sheet out.png]
$R build --example               # a working blueprint (rooms/doors/spawns/fill -> complete self-checking map)
$R lint  my.json && $R plan my.json && $R tour my.json out/tour.png   # check, then LOOK
$R verify my.json [--only walk[1]]   # the scene's own `checks`; a failing walk names the object that blocked it + writes an image
$R walk my.json --auto --from X,Z --to X,Z [--explain out.png]       # plan a route (never guess waypoints); `--path` replays one
$R ray my.json --from x,y,z --to x,y,z                              # line of sight: clear, or the first thing in the way
$R patch my.json '[{"op":"move","id":"lamp","by":[0,-0.2,0]}]'      # many edits, one atomic validated call
$R sim my.json                   # headless scripted play-throughs of the rules; `replay trace.json` re-runs a recorded match
$R ui-shot pause out/p.png --size 1280x720 ; $R ui-check           # see and audit the 2-D screens without a window
$R --json <any command>          # one stable envelope {schema, command, ok, exit, data, diagnostics, stderr}
$R src find <words>              # only if you must touch Rust: find/show/refs/deps, no file reads
scripts/dev fast | test          # unit tests | everything; prints a summary, the full log is in out/logs/
```

Map: `SPEC.md` scene + blueprint language · `AGENTS.md` workflow · `src/` engine (`$R src map`) · `assets/*.json` prefabs · `recipes/`, `examples/` ·
`docs/` glossary, ADRs (`$R describe decisions`), `HOSTING.md` · `mcp_server.py` MCP wrapper · `Dockerfile`, `deploy/` hosting.

When you change Rust: `//!` on new modules, `///` on pub items (`$R src coverage`), simulation logic as pure functions (not in `App`), decisions as
ADRs, then `scripts/dev test`. Before pushing: `scripts/ci.sh`. Record progress at every checkpoint: `$R status --note "..." --section done|now|next`.

## Facts (derived from the repo: `red_engine2 status --sync-docs CLAUDE.md` rewrites this block; a test fails if it is stale)
<!-- facts:begin -->
- Crate `red_engine2`; binaries: `re2`, `red_bot`, `red_engine2`, `red_server`.
- Cargo features: `default`, `gfx`.
- 26 integration test suites (`tests/`), 5 recipes (`recipes/`), 9 example maps (`examples/`); 35 ADRs (latest: 0035 rule state in the standard client). Test *counts* are not stated here: run `scripts/dev test`.
<!-- facts:end -->
