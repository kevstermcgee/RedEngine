# Red Engine 2

JSON-scene 3D engine (Rust; wgpu for graphics) with authoritative UDP multiplayer, a headless server, game rules as data and a
self-describing CLI. **You never read engine source, and you never guess:** the engine describes itself, checks your work and
explains its failures. Workflow: `AGENTS.md` (short; read it, do not import it). Everything longer is in `docs/AGENT_REFERENCE.md` and
`SPEC.md`, reached through `$R search "<question>"`, not by opening them whole (an `@`-import of either costs 10k+ tokens every session).

## Cheap by default (context and time)
```bash
$R context <feature|file|words>  # 5-15 KB work packet for a Rust change: files, public API, tests, ADR pointers (instead of `src map`, file reads)
scripts/dev preflight [--fix]    # ~1 s: ADR index, feature ownership, doc facts, headless boundary, fmt (+ command table and describe budgets with --full, which builds first); prints the exact edit for each problem
$R adr new "Title" --summary "one sentence"   # a decision record: dated id, index refreshed (no list to edit); an unfinished Summary fails preflight
# One ladder, cheapest first; each rung is what you run at that stage and none of them is optional at its own stage:
scripts/dev iterate              # 1. THE EDIT LOOP (seconds): only what changed since HEAD: fmt + type-check + clippy + the touched modules' unit tests. NEVER verification: it prints what it skipped
scripts/dev affected --quick     # 2. the features that own your branch's changes, with their integration suites (the change set is everything since origin/main; on a long branch use --base HEAD)
scripts/dev affected             # 3. + every feature built on them: before you say "done" (a green run is remembered by file content, base commit, feature set and toolchain)
scripts/dev affected --full      # 4. = scripts/ci.sh: before pushing (Cargo.*, a src/lib.rs change beyond new `mod` lines, CI files escalate to it by themselves)
```
Do not run the whole suite (`scripts/dev test`, bare `cargo test`) as an edit-loop habit, and never iterate with `--release` (measured on the 4-core dev box: type-check 2-4 s, lib-test rebuild 4 s, release binary 3 min 18 s, release test suite about 15 min). Servers, bots and tests listen on loopback only
(no OS firewall prompt to wait on); `red_server --public`/`--bind IP`/`--upnp` is the explicit way to face other machines.
Build profiles: `dev` (the default; tests), `fast` (`RED_PROFILE=fast scripts/dev build` or `cargo build --profile fast --bins`: optimized without LTO, a fraction of the release build; iterate on a
game with it) and `release` (LTO, 4+ minutes: ship with it). Put build output on another drive with `CARGO_TARGET_DIR` (every script honours it); C: filling up is the usual failure. With it set, the binaries are `$CARGO_TARGET_DIR/<profile>/red_engine2`, not `target/<profile>/`: an old copy left in `target/` keeps running with no error and hides your change (a `describe` that lacks the new thing is the tell), so run the one the build printed.

## First 60 seconds
```bash
scripts/dev doctor               # toolchain, binaries, git (Windows: powershell -File scripts\dev.ps1 doctor)
# No Rust on this Linux machine? curl -fsSL https://raw.githubusercontent.com/kevstermcgee/RedEngine/main/scripts/bootstrap.sh | sh   # prebuilt binaries + doctor
R="scripts/dev red"              # builds the CLI on first use, then runs it from any directory
$R doctor                        # what THIS machine can do: GPU or software rendering, UDP, ffmpeg, output dir
$R status                        # resume: facts + git + STATUS.md (what is done / in flight / next)
$R describe --brief              # ~1 KB manual: binaries, workflow, commands, topics; then $R search "<question>"
```

## Making a game (the fast path)
```bash
$R new-game ../mygame --name mygame --engine-path ../red-engine-2   # engine pin + blueprint/map + local asset incubator + scripts/red + CI
cd ../mygame && scripts/red check                                   # green from the first commit
# edit blueprints/main.blueprint.json (rooms, doors, spawns, fill, rules under "scene"), then:
scripts/red build-all && scripts/red check && scripts/red plan maps/main.json   # build, verify, LOOK
scripts/red serve                                                   # headless multiplayer server; `scripts/red play HOST:PORT` joins
```
A kart racer: `$R new-game ../mykarts --kind race --engine-path ../red-engine-2` (a generated circuit, the eight animals, bots, a lobby); then `race-test`, `frame`, the scaffolded hosting script (host it), `game publish ../RedEngineGames` (ship it).
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
scripts/dev fast | test          # all unit tests | the WHOLE suite (prefer `affected`); prints a summary, the full log is in out/logs/
```

Map: `SPEC.md` scene + blueprint language · `AGENTS.md` workflow (+ `docs/AGENT_REFERENCE.md`) · `src/` engine (`$R src map`) · `assets/*.json` prefabs · `recipes/`, `examples/` ·
`docs/` glossary, ADRs (`$R describe decisions`), `HOSTING.md`, `docs/LIFE_OF_A_SHOT.md` + `docs/LIFE_OF_A_REMOTE_PLAYER.md` (one shot / one remote player, end to end: read when a fight looks wrong) · `mcp_server.py` MCP wrapper · `Dockerfile`, `deploy/` hosting.

When you change Rust: `//!` on new modules, `///` on pub items (`$R src coverage`), simulation logic as pure functions (not in `App`), decisions as
ADRs, then `scripts/dev iterate` while you edit and `scripts/dev affected` before you say done. Before pushing: `scripts/dev affected --full` (= `scripts/ci.sh`). Only one heavy job (build, test, affected, iterate, ci) runs per target directory; `RED_WAIT=1` queues. Record progress at every checkpoint: `$R status --note "..." --section done|now|next`.

## Facts (derived from the repo: `red_engine2 status --sync-docs CLAUDE.md` rewrites this block; a test fails if it is stale)
<!-- facts:begin -->
- Crate `red_engine2`; binaries: `re2`, `red_bot`, `red_engine2`, `red_relay`, `red_server`.
- Cargo features: `default`, `gfx`.
- Wire protocol v14 (`src/net/protocol.rs`): a client and a server must be built from the same version. Counts (suites, maps, ADRs) and test totals: `red_engine2 status`, `scripts/dev test`.
<!-- facts:end -->
