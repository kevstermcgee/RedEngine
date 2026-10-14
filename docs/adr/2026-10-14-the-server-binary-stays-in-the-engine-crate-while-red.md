# 2026-10-14. The server binary stays in the engine crate while red_core holds its whole tree
Status: accepted
Summary: Item 3 is measured done but one clause: red_core holds everything red_server uses except the engine's scene-block registration, a sim edit goes through iterate in 9-14 s, and the binary stays in the engine crate until a game needs a server of its own

## Context
ADR 2026-10-13-the-next-phase, item 3, "done: the headless build, clippy and tests compile only `red_core` and its dependencies; no `#[path]`
across crates; a one-line edit in `sim/` goes through `iterate` in under 20 s warm on the 4-core box, measured against today's ~50 s." Measured after
the moves (#121, #122, #124-#129, #131, #132; docs/analysis/2026-10-13-red-core-cut-plan.md and 2026-10-13-red-core-inner-loop-measured.md):

- red_core holds 104 modules, about 52,000 lines: the scene format, the simulation, the physics, the wire protocol, the server, the client and the
  QUIC transport (behind its `quic` feature). `src deps --reach bin::red_server` finds one engine module left: `scene_blocks`, through which the
  server registers the engine's scene blocks (the museum).
- No `#[path]` crosses crates (since #121).
- A one-line edit in `crates/red_core/src/sim/rules.rs` goes through `iterate` in 9.4, 13.9 and 8.7 s warm, against ~50 s.
- `red_server` is still a binary of the engine crate, so `cargo build --no-default-features --bin red_server` compiles the engine library (without
  any graphics or audio crate: CI's headless-tree stage holds that).

Building it from red_core alone needs a crate of its own, and Cargo only gives a test the path of a binary of its own package
(`CARGO_BIN_EXE_<name>`): `net_processes` starts `red_server` and `red_bot` (whose tree is the engine's), and `ai_tasks` starts the server beside the
engine's tools. Those suites would find or build another package's binary at run time, which a parallel run and a single-suite run make fragile. The
generic server would also stop accepting the museum block (ADR 2026-10-13-a-scene-block-can-be-defined-above-the-core: a game registers its own).

## Decision
- `red_server` stays a binary of the engine crate. Item 3 is done in what it was for (one compiled core, its tests run once, the simulation's inner
  loop under 20 s), and the clause "the headless build compiles only red_core" waits for a reason to pay its cost: a game shipping a server of its
  own (item 5's game crates), which brings the per-game server binary and its block registration with it.
- Until then the rule of ADR 2026-10-13-red-core-is-one-crate-the-engine-and-its-games-share holds for anything the server needs: it moves into the
  core, never the other way, and `src deps --reach bin::red_server` is how to check that only `scene_blocks` is outside it.

## Consequences
The server's deployment build is what it was (the engine library without graphics). The suites that start the server keep `CARGO_BIN_EXE_red_server`.
Undo: a `crates/red_server` crate with the binary, the server suites moved or taught to find it, and the generic server registering no game blocks.
