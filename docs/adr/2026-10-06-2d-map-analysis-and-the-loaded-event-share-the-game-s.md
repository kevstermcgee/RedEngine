# 2026-10-06. 2D map analysis and the loaded event share the game's own collision
Status: accepted
Summary: Sim::can_reach answers can this walker touch that, with the game's solid_hit; it is a live scenario expectation and a static check with an open list, and loading a save fires the built-in loaded event so a world can be rebuilt from saved progress.

## Context
2D games had scripted scenarios but no map analysis: nothing said whether the key can be reached, or what a gate keeps out, without playing it. And a world that depends on saved progress (a gate already opened) could not be rebuilt on load: `start` rules run before the save is read.

## Decision
`Sim::can_reach(from, to)` floods a 1 px grid (coarser only for huge worlds) from where a top-down walker stands, asking the game's own `solid_hit`, and reports whether the walker can touch the target (a scene id or `tag:NAME`). It is a scenario expectation (`{reach, from, reachable}`, asked of the live world) and a static check (`checks.reach` with an `open` list of things assumed gone, via `remove_for_analysis`); the two must agree, and `tests/gate_meadow.rs` proves they do. Loading a save that restored something fires the built-in `loaded` event, so a rule can rebuild the world from saved variables.

## Consequences
An author can prove a level is solvable and that a gate matters in a second, with the engine's collision rather than a second model of it. Only top-down walkers have a reach (a platformer's depends on jumps). Harder: a game that relied on `start` alone to rebuild from saves must add a `loaded` rule. To undo: remove the check, the expectation and the event.
