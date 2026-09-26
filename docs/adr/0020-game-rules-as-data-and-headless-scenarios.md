# 0020. Game rules as data, proven by headless scenarios
Status: accepted

## Context
The engine described a *world* in JSON but every *game rule* (pick-ups, goals, traps, scoring) was Rust in `re2.rs`, so an AI
making a game had to edit a 1500-line file it could neither validate nor run without a window. The goal is: an AI defines a game
idea as data, and proves it works with one command.

## Decision
- `vars` + `rules` in the scene (`src/sim/rules.rs`, syntax in SPEC "Game rules as data"): `when` (enter/exit a volume, event, timer,
  start) / `who` / `if` (a small expression language, `rules_expr.rs`) / `once` / `cooldown` / `do` (set, add, emit, hide, show,
  teleport, end, impulse). Volumes come from zones, objects (with padding) or boxes and are resolved at parse time. Everything a
  rule refers to is validated when the scene loads, with a did-you-mean; an event nothing emits is an error.
- `RulesEngine` (`rules_run.rs`) is pure: it is given player positions each tick and returns effects (teleport, impulse); its state
  is plain data folded into the match checksum. `MatchSim` runs it after physics every tick, so the server, the scenario runner
  and (later) single-player share one implementation.
- **Scenarios** (`sim/scenario.rs`, `red_engine2 sim`, `checks.sim`) drive simulated players through `MatchSim` at 60 Hz with
  steering, waits and held inputs, then assert on events, variables, hidden objects, the outcome and where players ended. No
  window, GPU or socket; `verify` runs them, so "did I break the game?" is one command. `recipe coin_run` is a complete game
  and its own proof.

## Consequences
Rules are state, not presentation: `hide` records a hidden object; a renderer or client must act on it. ADR 0035 wires that state
to single-player `re2`; ADR 0036 replicates its bounded presentation state online. Variables are global (no per-player or per-team
scope yet). New actions or triggers go in `rules.rs` (parse) and `rules_run.rs` (run) with a test each; `describe rules` is
generated from `ACTIONS` and its example is parsed and *played* by a test, so the docs cannot drift.
