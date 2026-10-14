# 2026-10-14. A game runs the engine's commands with its own blocks: the command line is a library
Status: accepted
Summary: The `red_engine2` command line becomes library code (`red_engine2::cli`), and a game's own server program runs it with the game's scene blocks (`PROGRAM cli ...`), named by `game.json`'s `server.binary`, so a game whose blocks leave the engine keeps the engine's tools for its maps

## Context
ADR 2026-10-14-a-system-stays-in-the-engine-when-two-games-use-it-or moves the loadout shooter to Killchain, and ADR
2026-10-14-a-game-plugs-into-the-server-s-match-simulation-the gives Killchain a server program of its own. Before the move, a check of what else
reads a `shooter` map (2026-10-14):
- The Killchain project (RedEngineGames `projects/killchain`) is where its maps are made: generators in `tools/`, three maps, docs, and a
  `CLAUDE.md` that sends agents to `red_engine2 lint`, `nav` and `plan` on each map, and to `scripts/red check` and `serve`, which run
  `red_engine2 game check` and `red_engine2 game serve` (`red_server`).
- The engine's scene parser is strict: a root key no block defines is an error (`strict`), on purpose (a skipped key is an invisible bug).
  Once the `shooter` block is Killchain's, every one of those commands refuses Killchain's maps, and `red_server` cannot host them.
Frameless did not meet this: its content moved into its crate (`games/frameless/content`) and is published from there; no project edits it.

Considered and not done:
- Parse other games' blocks leniently (drop a known foreign key): the strict parser exists to stop silent drops, and `check` and `serve` would
  run the map without its game.
- Move Killchain's maps into its crate, as Frameless's: the project's generators, docs and agents work in RedEngineGames, and the engine's
  commands still could not read the maps.
- Let the engine's binary know the games: the engine does not depend on games.

## Decision
- **The command line is a library**, `red_engine2::cli` (`cli::main`; the `red_engine2` binary's `main` is one line), as the client
  (`client::run`) and the server (`server_app::run`) already are.
- **A game's server program is also the engine's command line with the game's blocks**: `PROGRAM cli <command>` (`server_app::run` defines the
  program's blocks, then runs the command line), so `killchain_server cli lint maps/main.json` reads a `shooter` map. The server program is
  the one place a game's blocks are defined without a window, so it suits checks, CI and containers.
- **`game.json` names the server program**: `server.binary` (default `red_server`; a package of the engine's workspace whose binary has its
  name, as `binary` is for the client). `game serve` starts it; a project's `scripts/red` runs the engine's commands through it.
- **In steps, each its own pull request:** (1) the command line moves into the library, nothing else; (2) `cli` in server programs,
  `server.binary`, `game serve`, the project script template, and `validate` reading a block the program defines; (3) the shooter's move,
  with `killchain_server` (and the release package builds a game's server without graphics); (4) the Killchain project names
  `killchain_server` (RedEngineGames, the owner merges).

## Consequences
Every game that brings scene blocks keeps every engine tool for its maps, through one program it already needs for hosting. The library grows by
the command line (about 5,000 lines): building it now compiles them for a crate that only links the library (a game crate), where before only
the `red_engine2` binary did; a full build compiles them once either way. Undo: put `src/cli/` back behind `src/main.rs` (a move), and drop `cli`
from server programs.
