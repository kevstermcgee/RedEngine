# Red Engine 2

An AI-first game engine in Rust (wgpu). A game is data: JSON scenes (geometry, props, lights), game rules (`vars`/`rules`) and blueprints that compile
rooms and doors into a complete, self-checking map. The engine describes itself, checks your work with the same collision and simulation code the game
runs (`lint`, `walk`, `verify`, `sim`) and shows what a player would see (`frame`, `plan`, `ui-shot`), so an author spends its time on the game.

## First command

```bash
scripts/dev start "<what you want to do>"   # picks the workflow and the executable, prints ONE next action
scripts/dev red describe --brief            # the manual, under <!--fact:brief-kb-->2.5<!--/fact--> KB; then: scripts/dev red search "<question>"
```

No Rust on this Linux machine? `curl -fsSL https://raw.githubusercontent.com/kevstermcgee/RedEngine/main/scripts/bootstrap.sh | sh` installs the prebuilt binaries.

**AI agents: [`AGENTS.md`](AGENTS.md) is the single entry point.** Ask the engine instead of reading it: `search` returns the few fragments that answer a question; `SPEC.md` (the scene-language reference)
and the long references are for lookup, never for reading whole.

## What you can build

- **First-person online games** (`re2`, `red_server`): authoritative multiplayer with lobby and rounds, prediction, lag compensation, bots, kart racing and a reusable firearm arsenal.
- **2D and hybrid native games** (`crates/red2d`): one JSON file, a deterministic simulation and a native window player (`play2d`). All games ship as native executables.
- **Other views** (top-down, strategy, spectator): keep the gameplay in scene rules and write a small client on `red_engine2::app` (`describe custom-client`).

A game is its own directory that uses the engine; never fork this repository (`red_engine2 new-game ../mygame`, ADR 0024).

## Where to go next

| you want | read |
|---|---|
| to work on the engine or build a game (the workflow, the tools, the rules) | [`AGENTS.md`](AGENTS.md) |
| the design, the tools by example, the source layout, the tests, the known limits | [`docs/ENGINE_OVERVIEW.md`](docs/ENGINE_OVERVIEW.md) |
| the scene language, only through `search` or a section | [`SPEC.md`](SPEC.md) |
| to host a multiplayer game | [`docs/HOSTING.md`](docs/HOSTING.md) |
| to publish a game | [`docs/GAMES_PUBLISHING.md`](docs/GAMES_PUBLISHING.md) (published games: [RedEngineGames](https://github.com/kevstermcgee/RedEngineGames), Windows packages on its [latest release](https://github.com/kevstermcgee/RedEngineGames/releases/latest)) |
| the game-idea generator and its nightly job | [`docs/IDEA_FORGE.md`](docs/IDEA_FORGE.md) |
| the sandbox, controllers and in-window map browser | [`docs/CONTROLLERS_AND_SANDBOX.md`](docs/CONTROLLERS_AND_SANDBOX.md) |
| where the project came from (the prop hunt viewer, Cheddar the rat) | [`docs/VIEWER_HISTORY.md`](docs/VIEWER_HISTORY.md) |
| what is in flight right now | [`STATUS.md`](STATUS.md) |

`examples/test_lab.json` is the primary development map; `house`, `school`, `office` and `store` are legacy reference maps kept as regression fixtures (ADR 0015).
