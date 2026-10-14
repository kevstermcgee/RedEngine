# Working on Red Engine 2's own code

The engine maintainer's page: how to change the Rust cheaply, verify only what a change can affect, and leave the codebase cheap for the next
agent. The entry point is [`AGENTS.md`](../AGENTS.md); making a game or a map is [`MAKING_GAMES.md`](MAKING_GAMES.md).

## Spend context and time like they cost money

Most of the cost of working on this engine is reading things and running things you did not need. The tools exist to make both small:

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
- **Worktrees.** `scripts/dev worktree NAME` for a second checkout (a branch, another agent) with its dependencies already compiled. Only one heavy job
  (build, test, affected, iterate, ci) runs per target directory; `RED_WAIT=1` queues. (Records, for everyone, are in `AGENTS.md`.)
- **Profiles and target directories.** `dev` (the default; tests), `fast` (`RED_PROFILE=fast scripts/dev build`: optimized without LTO, for iterating on a game) and `release` (LTO, minutes: ship with it); never iterate with `--release`.
  `CARGO_TARGET_DIR` (honoured by every script) puts build output on another drive, but then the binaries are `$CARGO_TARGET_DIR/<profile>/red_engine2`, not `target/<profile>/`: an old copy left in `target/` keeps running with no error and hides your change (a `describe` that lacks the new thing is the tell).
- **Shader edits** (`src/shaders/**`) run `shader_validation` and the `gpu`/`fx`/`ocean_pass` layout tests: naga parses, validates and writes HLSL for every module the engine builds, and no derivative lookup may sit in a loop. That is *not* Microsoft's compiler: **Windows hosted CI stays authoritative for Direct3D** (on a pull request its `direct3d` stage builds the renderer's pipelines through Microsoft's compiler; the full Windows suite runs on main), so a green local run says "free of the known failure class", never "compiles on Windows".
- **Do not run the whole suite as a habit** (`scripts/dev test`, bare `cargo test`): that is what `--full` and CI are for. Game projects: `scripts/red check` re-verifies only maps
  whose bytes (or the project's other JSON, or the engine binary) changed.
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


## Keeping the codebase cheap for the next AI

Every task should be doable from `describe`/`search`/`src show`, not by reading whole files. When you change Rust:

- **New module = a `//!` line saying what it is** (`src map` prints it) and `///` on every `pub` item
  (`red_engine2 src coverage --file <path>` lists gaps; `src show` and `search` print docs, not bodies).
- **Simulation rules are pure functions** (like `player.rs`): input state in, new state out, no window/GPU
  types. Do not add game/physics logic to `App` in `bin/re2/` — the tools and the headless server
  (ADR 0016, 0017) can only reuse what is callable without a renderer; put it in `sim/`.
- **Prefer a new file over growing a big one.** `schema.rs`, `props.rs`, `viewer.rs` are ~1000 lines; `re2`, the CLI and `physics` were
  split by subsystem (`src map` shows them). Navigate with `src outline`/`src show`, and put new subsystems in their own module.
- **A decision that a future reader would otherwise have to re-derive gets an ADR** (`docs/adr/`, template in
  its README; a test makes you register it). A new term gets a line in `docs/GLOSSARY.md`.
- **Behaviour worth keeping is a test or a `checks` entry**, not prose: `tests/house_walk.rs` and each
  recipe's `verify` block are the living documentation of the physics and the maps.
