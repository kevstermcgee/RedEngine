# 2026-10-06. Per-player rule variables: player_vars and me.name
Status: accepted
Summary: Rules can keep a variable per player (laps, lives, a personal score) and read and write the triggering player's copy as me.name; sim-only first slice

## Context
Rule variables (`vars`) are match-wide, so a game could not say "this player has lapped twice" or "this player has two lives left" as data. That is why
`sim::race` keeps laps in native Rust, and the Great Outdoors build note (docs/analysis/2026-09-30-agent-feedback-building-great-outdoors.md) ranks per-player
variables as the largest missing capability: every game with a per-player score, lives or progress needed Rust in the engine.

## Decision
A scene may declare `"player_vars": {"laps": 0, "lives": 3}` (at most `MAX_PLAYER_VARS` = 8). Each of the `MAX_PLAYERS` slots has its own copy, held in
`RulesEngine` as one flat row per slot and reset by `RulesEngine::reset_player` when `MatchSim::add_player_at` fills the slot. A rule reads and writes the copy of the
player that triggered it as `me.name` (`Expr::Me`, `Action::SetMe`; `World::me` answers the read). The triggering player is the slot of an `enter`/`exit` or of the
event's cause; `emit` passes the slot on. `me.` in a `start`, `every`, `after` or prop trigger is a validate error, because no player triggered it. The values are folded
into `RulesEngine::checksum` only when a scene declares any, so every existing scene and recorded trace keeps its checksum. A scenario proves one player's copy with
`{"player_var": "laps", "of": "<player id>", "gte": 2}`.

First slice only: nothing is sent to clients (the wire `RuleState` still carries match-wide variables, protocol v14) and nothing is saved by `persist`. Not done yet:
reading another player's copy or aggregates (`max_of`, `sum_of`), writing every player's copy at once, per-player values in the generic HUD and scoreboard, and moving
`sim::race` onto these variables (it stays native: Great Outdoors is live).

## Consequences
A game can count laps, lives and personal scores in data, and prove them per player in `sim`. A later slice adds a bounded per-player section to `RuleState`, which
bumps the wire protocol and needs client and server built from the same version. Undo: remove the `player_vars` block from a scene; scenes without it behave and
checksum exactly as before.
