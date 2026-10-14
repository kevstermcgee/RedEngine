# The files over 1,000 lines, and how the three longest split (2026-10-14)

ADR 2026-10-13-the-next-phase, alongside Phase 1: "a `preflight` file-length check (an allowlist of today's 28, a plan to split red2d `game.rs`, red2d
`sim.rs` and `net/protocol.rs`)". The review (docs/analysis/2026-10-13-next-phase-review-verified.md) counted 28 Rust files over 1,000 lines in `src/`
and `crates/`; on main at the time of this check there are still 28 (`find src crates -name '*.rs' | xargs wc -l | awk '$1 > 1000'`).

## The check

`preflight`'s `length` check (`src/tools/preflight.rs`): a Rust file under `src/` or `crates/*/src/` holds at most `MAX_FILE_LINES` (1,000) lines. The
28 that were longer are `LONG_FILES`, each with a ceiling it may not pass: its length on this day, rounded up to the next 250 lines. A listed file that
falls under the limit, or no longer exists, has to leave the list. A new long file, or a listed one past its ceiling, fails preflight with the edit
that fixes it: split it, a module per concern.

## How the three longest split

Each split is a structural pull request with its tests unchanged in number (the moves of ADR 2026-10-13-red-core-is-one-crate-the-engine-and-its-games-share
are the pattern); none is in this pull request.

| file | lines | what is in it | the split |
|---|---|---|---|
| `crates/red2d/src/game.rs` | 3,493 | the 2D game format's data model (lines 1-1,030: views, sprites, bodies, rules, widgets, levels, `GameDef`, the key tables) and its checker-parser (`Ctx`, `parse_sprite`, `parse_shape`, `parse_body`, `parse_move`, `parse_actions`, `parse_when`, and `parse`, lines 2,710-3,390), with no tests of its own | `game.rs` keeps the data model (~1,030); `game_parse.rs` the entry point, levels and the shared `Ctx` helpers; `game_parse_world.rs` sprites, shapes, bodies, movers and emitters (lines 1,115-1,712); `game_parse_rules.rs` actions, conditions and rules (1,714-2,700): each under 1,000 |
| `crates/red2d/src/sim.rs` | 2,566 | the 2D simulation: the public API and the tick (lines 281-900), rule execution (`fire`, `run_actions`, `drain_events`, buttons, clicks: 1,044-1,330), movement and collision (`build_grid`, `move_all`, `move_one`, `carry`, `solid_hit*`, `integrate`: 1,332-1,808), particles (1,808-2,027), and 540 lines of tests | `sim.rs` the API and the tick; `sim_rules.rs` rule execution; `sim_motion.rs` the grid, movement and collision; particles with the tick; the tests in `sim/tests.rs` |
| `crates/red_core/src/net/protocol.rs` | 2,222 | the wire protocol: constants and message types (lines 1-760), the codec (`W`, `R`, `put_*`/`get_*`: 897-1,213), `ClientMsg`/`ServerMsg` encode and decode (1,214-1,680), and the budget tests and tests (1,680-2,222) | `net/protocol/mod.rs` the constants and types; `net/protocol/wire.rs` the codec; `net/protocol/messages.rs` the message encoders; the tests in `net/protocol/tests.rs`. The path `net::protocol` stays; `PROTOCOL_VERSION` stays where `status` and `upgrade` read it (`status::PROTOCOL_FILES` names the file: it gains the new path) |
