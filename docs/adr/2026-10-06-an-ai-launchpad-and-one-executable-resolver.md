# 2026-10-06. An AI launchpad and one executable resolver
Status: accepted
Summary: scripts/dev start|next|resume picks the workflow and the executable and prints one next action without building; one resolver (scripts/red_resolve.py) is shared by scripts/dev, scripts/red and MCP

## Context
A fresh agent had four front doors that disagreed. README said to read AGENTS.md, AGENTS.md said to run `describe --brief` (which needs a compiled CLI), CLAUDE.md started with
`scripts/dev doctor`, and MCP had its own idea of the engine: `mcp_server.py` looked only in `<repo>/target/{release,debug}`, ignored `CARGO_TARGET_DIR` and always preferred `release`
however old, while `scripts/dev` and the generated `scripts/red` honoured the target directory, a profile and a project pin. Three real defects followed from that: `scripts/dev` and
`scripts/red` tested freshness on `src/` and `assets/` only, so an edit under `crates/` (red2d, web3d) left a stale binary in use; `scripts/dev build --headless` rebuilt the CLI without
updating its mode stamp, so a later `scripts/dev red` ran a headless CLI believing it was the full one; and a binary built from another checkout (a worktree) looked as good as any. Nothing
told an agent which of the 15 command families fitted its task, so the first minutes went on reading and on compiling a CLI just to ask what to do.

## Decision
`scripts/red_resolve.py` (standard library only, read-only) is the one rule for "which executable": `RED_ENGINE` > the project's `game.json` pin > this checkout; `CARGO_TARGET_DIR`; `RED_PROFILE`
(default `debug`, other profiles listed and never preferred); `RED_ENGINE_EXE` as an explicit override. A candidate is `stale` when a build input (Cargo.toml, Cargo.lock, build.rs, src/, assets/,
crates/, not docs or helper scripts) is newer, and a stale binary is reported, never selected. Features and revision come from the build records `scripts/dev` now writes next to a binary
(`.red-dev-*.mode`, `.red-build-*.json`); a binary without them says `unknown`. A prebuilt install records no revision, so it is `uncertain` and never serves a pinned project. `mcp_server.py`
calls `red_resolve.require`; `scripts/dev doctor` and `scripts/red doctor` print its verdict; `scripts/dev`, `scripts/red` and `scripts/red.ps1` test `crates/` for freshness and `scripts/dev build`
records the mode it built.

`scripts/launchpad.py` is the front door: `scripts/dev start "<task>" [--project DIR] [--target ...] [--workflow W]`, `next`, `resume` (`scripts/red ...` in a project, an MCP `start` tool).
It routes by named cues (every cue that fired is printed; `--workflow` overrides; no match is `unrouted`, an incomplete route and not an unsupported capability), composes the existing tools
(`propose` and `capabilities` for what can be built, the owners in `docs/features.json` and `context` for an engine change, `new-game`, `game upgrade plan`, `game check`, the `iterate` ->
`affected` ladder) and prints one next action with working directory, argv and a success condition, plus iteration checks and the separate claims (validation, behavior, visual/input, target
execution, networking) that a result may support. It never compiles, installs, downloads or verifies: the only processes it starts are `git` and, when a fresh executable exists, the read-only
`propose`, `capabilities` and `context`. `start` records `out/launchpad/<task>.json` (ignored; `--no-save` skips it): the objective, route, and the identity of the inputs, the executable and the
verification configuration. `resume` diffs that identity with the present one, names what changed, and marks every recorded result `stale` after a change; a recorded result is otherwise
`unverified` because only its own tool (`affected`, `game check`, `web status`) knows what it keyed on. Agent notes (`--note`) are kept apart from observed results; no state is ever `passed` here.

## Consequences
One rule instead of three, and a first command that works on a fresh checkout. The two defects above are fixed and tested (`scripts/test_launchpad.py`, run by `tests/launchpad.rs`, including a
mutation check against the old `scripts/dev`). Not done: capability answers need a fresh CLI (without one they are reported unchecked); no published release exists, so `bootstrap.sh` still
fails until one does (diagnosed, not fixed: publishing is out of scope); `scripts/red` on Windows could not be run here; no claim is made about tokens, completion time or smaller-model success,
which need fresh-agent trials. Undo: delete the `start|next|resume` cases and `scripts/launchpad.py`; the resolver is independent of it.
