# 0024. The framework layer: blueprints and game projects (games use Red, they do not fork it)
Status: accepted

## Context
The Cheddar game was built by copying this whole repository. That worked at first and then failed twice: engine
improvements (blocker-naming walks, fresh docs, headless builds) could not reach the fork, and the fork's own
`CLAUDE.md` went stale ("multiplayer is not built"). Separately, every map was ~900 objects of hand-typed coordinates
produced by a generator script, so the AI paid tokens for arithmetic (wall corners, door offsets, aisle clearance) that
a program does correctly, and loose props placed at hand-typed coordinates were the usual reason a route failed.
The maintainer asked whether a "RedFramework" between the engine and the AI would help.

## Decision
The framework is a data layer inside the existing CLI, not another process or repository to keep in sync:
- **Blueprint** (`build`, `tools/blueprint.rs`): a ~20-line JSON description (rooms as rectangles, doors between rooms, spawn
  groups, prop fill, a raw `scene` block for rules and extra checks) compiled into a complete scene: walls and doors
  from shared/free room edges, floors, lamps, sun, camera, `zones`, `spawns`, `portals` + `interest` (what the multiplayer
  server needs), seeded fill that keeps door pads, room aisles and spawn pads clear, and a `checks` block that already passes
  (lint budget, reach per room, an auto-planned walk from the first spawn to every room). Output is deterministic and
  is a normal scene; `build --check` fails when the committed map is not what the blueprint builds.
- **Game project** (`new-game`, `game check|build-all|info|serve|play`, `tools/game.rs`, `tools/newgame.rs`): a directory with
  `game.json` (name, pinned engine as a git ref or local path, blueprints, maps, server settings), blueprints, generated maps,
  `CLAUDE.md`, `STATUS.md`, `scripts/red` (+ `red.ps1`) which fetches and builds the pinned engine, and a CI workflow that runs
  `red check` on a headless build. Game rules stay data (ADR 0020), so most games need no Rust; when they do, the crate
  depends on `red_engine2` as a library.
- The escape hatches keep it honest: any blueprint key group can be replaced by editing the built scene (delete the blueprint),
  `extra` appends raw objects, `scene` merges any scene keys.

## Consequences
An AI's first game is: `new-game`, edit one blueprint, `build-all`, `check`, `serve`. Engine upgrades are a one-line ref change.
The layout vocabulary is deliberately small (axis-aligned rooms on a shared grid, doors, fill); non-rectangular architecture is
still hand-authored scene JSON. Adding a blueprint key means adding it to the key list, `SPEC.md` "Blueprints" (a test checks
every key is documented) and a test. To undo: delete `blueprint.rs`, `game.rs`, `newgame.rs`; scenes are untouched.
