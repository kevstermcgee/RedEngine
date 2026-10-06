# Working on Red Engine 2 (for AI agents)

Red Engine 2 is a Rust + wgpu engine whose maps are **JSON scene files** — and whose **game rules are data too**.
`re2` walks you around one in first person; `red_server` runs the same simulation headless as an authoritative UDP
server; `red_engine2` (the CLI) validates, renders, **analyzes, edits and plays scripted matches** on maps. You almost
never need to read Rust to change a map or a game: read [`SPEC.md`](SPEC.md) for the scene language, then use the tools below.

**First command: `scripts/dev start "<your task>"`** (in a game project `scripts/red start "<task>"`; MCP: the `start` tool). It picks the workflow, names the executable it will use and why, and prints ONE next action; it never builds, installs or downloads. `next` and `resume` pick up after an interruption.

**Then `red_engine2 describe --brief` (about 1 KB)**, then ask `search "<your question>"`; open SPEC/AGENTS only for a topic
you cannot get from `describe <topic>`. Every command takes the global `--json` for one stable envelope (`describe diagnostics`).

**Making a 2D or browser game? Do not read any of the 3D material.** `red_engine2 describe web` is the front door (one page: choose 2d / hybrid / 3d, create, check, publish, read the evidence, limits);
`red_engine2 web status G` says where a game stands and the exact next command (`--json` for a program); `describe 2d` is the file format; `new-game DIR --kind 2d` starts from a verified game
(`examples/2d/gate-meadow` uses every feature at once); then `validate` -> `verify` -> `web verify` -> `publish`. `red_engine2 capabilities` says what is built (2D and hybrid run in a browser and headless;
3D runs on windows/linux, in a browser only experimentally; browser multiplayer is not supported, and it says so).

> **The one rule:** never trust a map edit you haven't run through `lint`, and never judge a
> layout you haven't *looked at* (`plan` / `tour`). The tools use the game's real collision code, so
> "lint is clean and the walk test passes" means the level is playable.

## Spend context and time like they cost money

Most of the cost of working on this engine is reading things and running things you did not need. The tools exist to make both small:

- **Ask the engine, do not read it.** `search "<question>"` returns the best few fragments (docs, ADRs, assets, lint codes, recipes, Rust symbols); `describe <topic>`,
  `catalog`, `recipe` answer the rest. Do not open SPEC.md, this file's reference (`docs/AGENT_REFERENCE.md`) or `describe all` (80 KB) whole.
- **Changing Rust? Start with `context <feature | file | words>`**: one 5-15 KB work packet with the feature's files, public API (signature + doc line), the tests that
  cover it, the verification command and pointers to the relevant ADRs. It replaces `src map` (17 KB; `src map <prefix>` is the cheap form),
  `describe commands` (11 KB; `search <name> --kind command` for one command) and opening files. Drill down with `src outline`, `src show <symbol>`, `src refs <symbol>`.
- **Verify what your change can affect, not everything.** `scripts/dev affected` maps the changed files to the features that own them and the features built on those
  (`docs/features.json`, ADR 0033) and runs only their fmt, clippy, unit and integration checks. Real-time network suites run one test at a time; the rest run in parallel.

  | command | when | what runs |
  |---|---|---|
  | `red_engine2 preflight [--fix]` | before each commit (about a second, compiles nothing) | the repository's paperwork: ADR records and index, `docs/features.json` ownership of every file, derived doc facts, the headless boundary, doc claims, `describe` budgets, rustfmt; each problem prints the exact edit (ADR 2026-09-28-generated-bookkeeping) |
  | `scripts/dev iterate` | the edit loop, after every change | only what changed since `HEAD`: fmt, type-check + clippy, the touched modules' unit tests (seconds). **Never verification**: it lists what it skipped and says full verification is still required; `--check-only` drops the tests, `--headless` type-checks without graphics when every changed file is provably graphics-free |
  | `scripts/dev affected --quick` | after a meaningful step | owners of the branch's changed files (vs `origin/main`; `--base HEAD` for just your last edit) and their integration suites; the dependents it skipped are listed |
  | `scripts/dev affected` | before you say "done" | owners plus every feature built on them |
  | `scripts/dev affected --full` (= `scripts/ci.sh`) | before pushing / any integration boundary | everything CI runs |

  Boundary changes (`Cargo.toml`/`Cargo.lock`, `src/lib.rs` (unless it only gained `mod` lines), `rustfmt.toml`, `.cargo/`, CI files, a very large diff, or an affected set that is most of the suite)
  escalate to the full run on their own. A green run is remembered by the *content* of the changed files: asking again with nothing edited is free; any edit re-runs.
  The green stamp also keys on the base commit, the cargo feature set, the toolchain and result-affecting environment, so a result is never reused across them, and an `iterate` (partial) pass is never accepted as any other tier.
  Output is a few lines per step; full logs are in `out/logs/`. `--dry-run` prints the plan and its configuration. The feature index is read from the checkout at run time: editing `docs/features.json` needs no rebuild.
- **Shader edits** (`src/shaders/**`) run `shader_validation` and the `gpu`/`fx`/`ocean_pass` layout tests: naga parses, validates and writes HLSL for every module the engine builds, and no derivative lookup may sit in a loop. That is *not* Microsoft's compiler: **Windows hosted CI stays authoritative for Direct3D**, so a green local run says "free of the known failure class", never "compiles on Windows".
- **Do not run the whole suite as a habit** (`scripts/dev test`, bare `cargo test`): that is what `--full` and CI are for. Game projects: `scripts/red check` re-verifies only maps
  whose bytes (or the project's other JSON, or the engine binary) changed.
- **Never wait for a person.** Servers, bots, `play-local`, `net-test` and `perf` listen on loopback only, which never raises an OS firewall prompt. `red_server` needs an explicit
  `--public`/`--bind IP`/`--upnp` to face other machines (hosting, `docs/HOSTING.md`); never add a wildcard (`0.0.0.0`) bind to a test or a tool.

## Current direction (ADR 0015): engine first, Test Lab as the dev map

Engine quality, the shared headless simulation (`src/sim/`), testing and extensibility come before map work.
Curated games, prototypes, test content and demos are published to RedEngineGames
through `games-publish.json`. Run `python scripts/publish_games.py check` after
changing that manifest or any published source. See docs/GAMES_PUBLISHING.md; never
publish build output, logs, credentials or unreviewed scratch files.
`examples/test_lab.json` (the **Red Test Lab**) is the primary development map: basic visuals, one room per
system under test (spawns, clearance gaps, stairs/ledges, static + dynamic props and stacks, hitscan lane,
character sizes). `tests/test_lab.rs` + its own `checks` guard it. `house`/`school`/`office`/`store` are
**legacy reference content**: keep them working when cheap, never let them block a better engine design, and
record any deliberate incompatibility in ADR 0015. Before pushing: `scripts/ci.sh` — `cargo fmt --check`, clippy `-D warnings`,
`cargo test --locked`, benches compile, and the **headless server** build/lint/test with `--no-default-features` (no wgpu/winit/rodio;
the same as `.github/workflows/ci.yml`, whose `headless-linux` job installs no graphics/audio libraries). Performance:
`cargo bench --bench sim` then `python benches/check.py` (`benches/README.md`); allocation, bandwidth and packet-size budgets are
ordinary tests (`tests/alloc_budget.rs`, `tests/net_budget.rs`), and `tests/ai_tasks.rs` budgets the *context* canonical AI tasks may use.

## Start here (you should never need to read Rust)

The engine describes itself. In this order, cheapest first:

```bash
red_engine2 describe --brief            # ~1 KB: binaries, workflow, commands, topics (read this first)
red_engine2 describe                    # overview: what exists, every command, topics (~50 lines)
red_engine2 search "<question>"         # best fragments across docs, assets, lint codes, recipes, Rust symbols
red_engine2 catalog [words|name]        # 39 props + ~155 JSON prefabs (incl. wall art, sculptures, unlit lamps); `catalog apple_red` = params + paste-ready snippet
red_engine2 catalog --category food --sheet out/food.png   # SEE the assets (labelled contact sheet)
red_engine2 recipe [name] [--new my.json]   # known-good complete maps to copy from
red_engine2 describe objects|scene|lint|physics|conventions|rules|sim|diagnostics   # exact fields, codes, numbers, rules, syntax
red_engine2 src map | find <words> | show <symbol> | refs <symbol> | deps   # only if you MUST touch Rust
red_engine2 describe glossary           # vocabulary: prop vs prefab, zone, body band, the four maps, "tire iron" naming
red_engine2 describe decisions          # ADR index: WHY it is built this way (docs/adr/); `search <topic> --kind adr`
```

Then the loop: `add/set/move` (auto-validated) -> `lint` -> `plan`/`tour`/`frame` (**look**) -> `verify`.
A scene's `checks` block (lint budget, reachability, real-physics walks, object assertions, golden
views) makes "did I break anything?" one command; `diff a.json b.json` / `diff scene.json --git`
shows what changed by object id. For the 4-map plan start from `recipe two_floor_house`
(house), `classroom_wing` (school), `convenience_store`, `rooms_and_door` (any small interior).

## Making a game: blueprints and game projects (ADR 0024)

A game is its own directory that **uses** the engine; do not fork this repo. `red_engine2 new-game ../mygame --name mygame --engine-path ../red-engine-2`
creates `game.json` (pins the engine), `blueprints/main.blueprint.json`, the built `maps/main.json`, `CLAUDE.md`, `STATUS.md`, `scripts/red` (finds/builds
the pinned engine), an assets/gameplay.json linked local asset incubator and a CI workflow; it is green from the first commit (`scripts/red check`). The loop: edit the blueprint, `scripts/red build-all`,
`scripts/red check`, `scripts/red plan maps/main.json` (look), `scripts/red serve`. A **blueprint** (`build --example`, SPEC "Blueprints") is rooms as
rectangles, doors between them, spawn groups and prop fill; it compiles to walls, floors, lamps, zones, spawns, `portals` + `interest`, fill that never
seals a door, and a `checks` block that already passes (lint, reach per room, auto-planned walks). Game logic goes in the blueprint's `scene` block as
data (`vars`/`rules`, `describe rules`) and is proven with `checks.sim`. `build --check` and `game check` fail when a committed map no longer equals what
its blueprint builds. Put custom Rust in a crate that depends on `red_engine2` as a library, never a copy of it.

**Not a first-person game?** (top-down, strategy, a spectator view) Keep the gameplay in scene rules and write only a client: `describe custom-client`
(ADR 0043). `red_engine2::app` gives a `LocalSession` over the real simulation, any `ViewCamera` with pointer picking, input, the world renderer, the
rules HUD and offscreen presentation checks; `examples/external/topdown_switch` is a complete one in its own crate.

## The editing loop

```
red_engine2 ls   map.json                 # what's there, where (world bounds per object)
red_engine2 info map.json sofa_1          # one object: JSON, bounds, neighbours, its lint findings
   ... edit (set / move / add / rm / clone / array / scatter / line, or edit the JSON directly) ...
red_engine2 lint map.json                 # exit 1 on errors; fix everything it says
red_engine2 plan map.json --all-floors    # top-down plan PNG per floor (open it with an image viewer)
red_engine2 tour map.json out/tour.png    # rendered views of every room + cutaway of every floor
red_engine2 walk map.json --path "0,-8; 0,1; -0.8,2; -0.8,7.6"   # replay a route with the real physics
```

Every editing command re-validates the whole scene and **refuses to write an invalid file**
(`--dry-run` previews, `--force` overrides), and prints the follow-up `lint` command.

## Keeping the codebase cheap for the next AI

Every task should be doable from `describe`/`search`/`src show`, not by reading whole files. When you change Rust:

- **New module = a `//!` line saying what it is** (`src map` prints it) and `///` on every `pub` item
  (`red_engine2 src coverage --file <path>` lists gaps; `src show` and `search` print docs, not bodies).
- **Simulation rules are pure functions** (like `player.rs`): input state in, new state out, no window/GPU
  types. Do not add game/physics logic to `App` in `bin/re2/` — the tools and the headless server
  (ADR 0016, 0017) can only reuse what is callable without a renderer; put it in `sim/`.
- **Prefer a new file over growing a big one.** `schema.rs`, `props.rs`, `viewer.rs` are ~1000 lines; `re2`, the CLI and `physics` were
  split by subsystem (see the map above). Navigate with `src outline`/`src show`, and put new subsystems in their own module.
- **A decision that a future reader would otherwise have to re-derive gets an ADR** (`docs/adr/`, template in
  its README; a test makes you register it). A new term gets a line in `docs/GLOSSARY.md`.
- **Behaviour worth keeping is a test or a `checks` entry**, not prose: `tests/house_walk.rs` and each
  recipe's `verify` block are the living documentation of the physics and the maps.

## Where the rest is

`docs/AGENT_REFERENCE.md` holds the long-form reference that used to live here: multiplayer, game rules and replay, walk failures, setup, the **tool reference table**
and lint codes, the map-building checklist, prefabs and recipes, lessons learned, the reference maps, **engine internals**, characters, weapons, loose props and viewer
debugging. It is indexed by `search` (each heading is a fragment); read a section only when `search` points at it.
Two end-to-end walks answer "why does this fight look wrong": `docs/LIFE_OF_A_SHOT.md` (one trigger pull, click to hit marker) and `docs/LIFE_OF_A_REMOTE_PLAYER.md` (another player on your
screen, Hello to avatar); each step names the function to open, the tuning key, how to see it without a screen and how it fails silently.
