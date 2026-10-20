# Working on Red Engine 2 (for AI agents)

**This file is the single entry point.** (The README is a pointer page; `CLAUDE.md` adds only the facts derived from the code.) Red Engine 2 is a Rust + wgpu engine whose maps are **JSON scene files** — and whose **game rules are data too**.
`re2` walks you around one in first person; `red_server` runs the same simulation headless as an authoritative UDP
server; `red_engine2` (the CLI) validates, renders, **analyzes, edits and plays scripted matches** on maps. You almost
never need to read Rust to change a map or a game: the scene language is `describe scene` and `describe <topic>`, and `search` finds the rest
([`SPEC.md`](SPEC.md) is its full reference, searched, never read whole).

**First command: `scripts/dev start "<your task>"`** (in a game project `scripts/red start "<task>"`). It is a shell command, not an MCP tool. It picks the workflow, names the executable it will use and why, and prints ONE next action; it never builds or verifies. With no usable executable it tries to fetch the prebuilt release that was built from exactly this checkout's sources (checksum verified, no compile; `--no-fetch` or `RED_NO_FETCH=1` stays offline) and says why when there is none: then the next action is the build. `next` and `resume` pick up after an interruption.

**Then `red_engine2 describe --brief` (under <!--fact:brief-kb-->2.5<!--/fact--> KB)**, then ask `search "<your question>"`; open a document only for a topic
you cannot get from `describe <topic>`. In a checkout `red_engine2` below means `scripts/dev red` (builds the CLI on first use; `scripts/dev doctor` says what this machine has).
Every command takes the global `--json` for one stable envelope (`describe diagnostics`).

**Then the page for your task, and only that one:**

- **Making a game or a map** (no Rust): [`docs/MAKING_GAMES.md`](docs/MAKING_GAMES.md): the cheapest-first commands, game projects and blueprints, the
  editing loop. **A 2D game reads none of the 3D material:** `red_engine2 describe 2d` is the format and the loop; `new-game DIR --kind 2d` starts from a
  verified game (`examples/2d/gate-meadow` uses every feature); then `validate` -> `verify` -> `play2d` -> `publish`. All games ship as native executables.
- **Publishing a game, an update or a fork** (the game, its page on the download site, the publish entry, in one pull request): [`CONTRIBUTING.md`](CONTRIBUTING.md);
  `scripts/games.py fork` makes your version of any game. Two equal maintainers, the feedback inbox and the daily improver: [`docs/COLLABORATING.md`](docs/COLLABORATING.md).
  With Claude Code: `/make-game`, `/update-game`, `/fork-game`, `/engine-feedback` (leave one at the end of every game session), `/inbox`.
- **Changing the engine's Rust:** [`docs/ENGINE_WORK.md`](docs/ENGINE_WORK.md): `context` before you open a file, the `iterate` -> `affected` ladder, records,
  profiles, the current direction, and keeping the codebase cheap for the next agent.

> **The one rule:** never trust a map edit you haven't run through `lint`, and never judge a
> layout you haven't *looked at* (`plan` / `tour`). The tools use the game's real collision code, so
> "lint is clean and the walk test passes" means the level is playable.

## For everyone

- **Ask the engine, do not read it.** `search "<question>"` returns the best few fragments (docs, ADRs, assets, lint codes, recipes, Rust symbols);
  `describe <topic>`, `catalog`, `recipe` answer the rest. Do not open SPEC.md, `docs/AGENT_REFERENCE.md` or `describe all` (80 KB) whole.
- **Records.** `red_engine2 preflight [--fix]` before each commit (about a second, compiles nothing); `red_engine2 status --note "..." --section
  done|now|next` at every checkpoint; `red_engine2 adr new "Title" --summary "one sentence"` for a decision a future reader would otherwise re-derive
  (a fix, a rename, a measurement or progress is a status note, not an ADR).
- **Never wait for a person.** Servers, bots, `play-local`, `net-test` and `perf` listen on loopback only, which never raises an OS firewall prompt;
  never add a wildcard (`0.0.0.0`) bind to a test or a tool (hosting is `docs/HOSTING.md`).

## Where the rest is

`docs/ENGINE_OVERVIEW.md` is the design, the map tools by example, the source layout, the tests and the known limits. `docs/AGENT_REFERENCE.md` is the
long-form reference (multiplayer, game rules and replay, the **tool reference table** and lint codes, the map-building checklist, prefabs and recipes,
**engine internals**, characters, weapons, viewer debugging), indexed by `search` heading by heading. `red_engine2 mcp` serves the same commands to MCP
clients (<!--fact:mcp-tools-->11<!--/fact--> tools). `docs/LIFE_OF_A_SHOT.md` and `docs/LIFE_OF_A_REMOTE_PLAYER.md` walk one trigger pull and one remote
player end to end, naming the function to open at each step.
