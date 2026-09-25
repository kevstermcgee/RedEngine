# What building Cheddar taught us about Red (2026-09-24)

An AI built the game **Cheddar** on Red and had a very hard time. It wrote down what worked, what cost it time and tokens, and what it
would build next. This is the evaluation of that feedback, what changed in Red because of it, what was borrowed from two sibling
projects (BlueEngine, and Feta, a game Codex built on it), and what is still open. Scope: Red only. Cheddar itself was not touched.

## The short version

* The pain was not "the engine is bad". It was four things around it: **a game living in a fork of the engine**, **hand-typed
  coordinates**, **failures that did not say why**, and **no memory between sessions**. Most of the fixes are tools, not physics.
* Red now has a framework layer, built into the CLI rather than as a separate program: **blueprints** (`build`) that compile ~20 lines
  into a complete self-checking multiplayer-ready map, **game projects** (`new-game`, `game check|serve|play`) that pin the engine instead of
  forking it, and the diagnostics the feedback asked for (`walk` naming its blocker, `walk --auto`, `ui-shot`, `status`, `scripts/dev`).
* On Cheddar's own 885-object map the full `verify` went from **40 s to 3.7 s** (the feedback measured ~25 s in the dev profile), and its
  headline failure is now one readable block plus a picture.
* Not done, and important: **the wire protocol is unencrypted and unauthenticated**, and **none of the Linux paths have been run** (details below).

## The feedback, item by item

| # | The feedback | Status | What now happens |
|---|---|---|---|
| 1 | A failing `walk` says where the player stopped, not what stopped them | **done** (ADR 0023) | `BLOCKED BY 'floor_crate_x' [prop:crate] gap 0.00 m, occupies x -9.80..-7.00 z 5.10..7.90`, passage width vs the 0.7 m body, and an image with the blocker boxed and labelled (`out/verify/<scene>_walk<N>_explain.png`) |
| 2 | `reach` passes but `walk` fails and nothing explains the gap | **done** | the diagnosis also says whether a flood fill from the stop point can reach the target ("the straight leg is what is obstructed: try `walk --auto`") |
| 3 | Authoring walk routes by guessing coordinates | **done** | `walk --auto --from X,Z --to X,Z[,Y]` plans a route (grid A* on the reach model, string-pulled, validated with the real per-tick physics) and prints waypoints plus a paste-ready check; a `checks.walk` entry can be `{"from","to","auto":true}` and plans every run |
| 4 | `verify` printed "3 error(s)" for lint but not the errors | **already fixed in the current engine** | the fork predated it; `verify` prints the first findings inline |
| 5 | Stale project docs ("not built", "95+ tests") | **done** (ADR 0025) | `CLAUDE.md` carries a facts block derived from the repo; `tests/docs_fresh.rs` fails on a stale block, a hand-written test count, a "not built" claim, a doc path that does not exist, an undocumented blueprint key or a CLI command missing from `AGENTS.md` |
| 6 | No handoff file, so a closed terminal costs a full re-derivation | **done** | `red_engine2 status` (one screen: facts, git, recent commits, uncommitted files, `STATUS.md`), `status --note "..." --section next` |
| 7 | Toolchain setup friction (`. ~/.local/toolchain/env.sh;` before every command) | **done** | `scripts/dev` and `dev.ps1` (any OS, any directory, finds the toolchain, sets timeouts); scaffolded projects get `scripts/red` |
| 8 | UI work is blind without a screenshot path; magic-number layout | **done for the engine's screens** (ADR 0026) | `ui-shot pause out.png --size 1280x720 --message "..."` renders with no window/GPU; `ui-check` audits every screen at 9 sizes and found three real overflow bugs in the launch/pause menus (fixed). Cheddar's own `ui.rs` lives in its repo and is not ported |
| 9 | Slow feedback loops (25 s verify, 52 s map test) | **partly done** | `verify` no longer computes the grid when only walks are selected and shares it across `reach` checks; `--only walk[1]` or any name text; every check is timed; `scripts/dev test` prints a summary and keeps the log. **Not done:** the reachability flood itself (about 3.4 s on a 885-object map) and a changed-files-only test runner |
| 10 | Generated map JSON compiled in and possibly stale; loose props at hand-typed coordinates block routes | **done** | `build --check` / `game check` fail when a map is not what its blueprint builds; blueprint `fill` keeps door pads, aisles between doors and spawn pads clear (tested over 12 seeds of a crowded room); a game's own prefab libraries are merged in with `prefab_files` |
| small | `assert_send::<Server>()`, `Server::run` blocking shape, bot as practice opponent, `hint_for` / `lobby_hint` parity | not done | Cheddar-specific or minor; the blocking `Server::run(&stop)` shape was kept |

The three things the feedback said it would pick if it could only have three (blocker-naming walks, `walk --auto`, `ui-shot` + fit tests)
are all in.

## Why the fork was the root cause

Cheddar's repository is a copy of this one. Compare what happened on each side after the copy:

* Red gained `verify` printing lint findings, a `--json` envelope, headless builds, rules as data and deterministic replay, and updated
  `AGENTS.md`. None of it reached Cheddar, whose `CLAUDE.md` kept saying multiplayer was not built.
* Cheddar added its own prefab library by editing the engine's built-in asset list (`assets/market.json`), so its map does not even
  validate on stock Red (`unknown prefab 'end_cap'`). With its 20 prefab definitions supplied (now what `prefab_files` does, without
  touching the engine) the whole map validates and passes its own checks.

So a game must be able to *depend on* the engine: pin a version, bring its own assets, and upgrade by changing one line. That is what a
game project is (`game.json`, ADR 0024).

## Is a "RedFramework" between the engine and the AI the right approach?

Yes for the idea, no for the shape of a separate program. What an AI needs translated is intent ("three rooms, a door between each,
two players in the first, crates in the rest") into coordinates and checks, and failures into causes. Both are best done **inside the
tools the AI already uses**, because a separate layer is one more thing to keep in sync (exactly the drift that hurt Cheddar) and one more
thing to read. So the framework is data, not a process: a blueprint format compiled by `build`, a project layout, and diagnostics that
explain themselves. The blueprint vocabulary is intentionally small (axis-aligned rooms on shared edges, doors, spawns, fill). Non-rectangular
architecture, and anything needing hand-placed props on shelves, is still authored scene JSON, reachable through `extra` and `prefab_files`.

Measured: the example blueprint is 590 bytes and compiles to a 9 KB, 24-object map with 8 passing checks (lint, reach per room, three
auto-planned walks). Cheddar's market is not comparable yet (its generator is 31 KB of Python for 885 objects: shelves, cold cases, a car
park); porting it is the honest test of the vocabulary and would probably add `fill` for prefabs and rows/grids.

Orientation cost for a fresh AI, from cheapest: `CLAUDE.md` 4 KB, `describe --brief` 1.3 KB, `describe` 6.7 KB, then `AGENTS.md` 37 KB (too big:
see next steps), `SPEC.md` 34 KB.

## Ideas taken from BlueEngine and Feta

| Idea | Where | Taken? |
|---|---|---|
| `doctor` (read-only environment report) | BlueEngine `tools/be2.py doctor` | yes, as `red_engine2 doctor`, probing GPU (hardware then software), audio, ffmpeg, UDP, output dir, git |
| Checked patch transactions (`apply`) | `be2-tools apply` | yes, as `patch` (atomic, one validation, names the failing op) |
| `check` with persistent logs, not full output in the context | `be2.py check` | yes, in `scripts/dev` (`out/logs/`, summary only) |
| `ray` line-of-sight query | `be2-tools ray` | yes, as `ray` (uses the exact shapes the weapons use) |
| systemd unit, hosting notes, router mapping | Feta `deploy/`, `docs/HOSTING.md`, `feta_portmap.py` | unit + `docs/HOSTING.md` yes; automatic UPnP mapping **not** (a good later `red_server --upnp`) |
| Map/game content fingerprint in the handshake | BlueEngine protocol 3 | already there (`WrongMap` reject; the hash ignores line endings) |
| Bounded `GameDocument` (counters, rules, interactables) | BlueEngine `game.json` | no: Red's `vars`/`rules` (ADR 0020) is richer and already proven headless |
| `tools/FEATURES.json` feature to file to check map | BlueEngine | no: hand-kept indexes drift; Red derives (`src map`, `status`) |
| QUIC datagrams with TLS 1.3 and a bundled certificate | Feta | not yet: see "encryption" below; the decision is open |
| Follow-camera boom that retracts and eases out | Feta `FETA_CAMERA_FIX.md` | not taken: game-side third-person camera work; worth reading before building one |

## Red against your three requirements

**Multiplayer.** Strong: an authoritative server (movement, props, pick-up, bat/revolver, rules), 30 Hz delta snapshots acknowledged per
client, prediction and reconciliation, rooms-and-portals interest management, reconnect by token, deterministic replay with a first-
divergent-tick report, tested over real UDP with loss, in separate processes and against hostile packets. A blueprint-built map ran on the
real server with two bots that crossed a door and saw each other (`snapshots_missed: 0`). Gaps, in the order I would fix them:
1. **No encryption or authentication.** Anyone who can reach the port can join and read traffic. Cheddar's fork already prototyped a
   passkey-derived encrypted protocol with silent drops for strangers; Feta chose QUIC/TLS. Decide, then upstream one (the fork forked before
   protocol v2, so it is a port, not a merge). Until then `docs/HOSTING.md` recommends Tailscale/WireGuard.
2. No lag compensation for hitscan (the Cheddar fork built lag-compensated hits) and no round/lobby state machine (the fork's `Round`
   is pure and window-free, a good candidate to generalise).
3. Rule state (variables, hidden objects) is not replicated to clients; single-player `re2` still has its own weapon code; the online
   interaction wiring in `re2` is compile-verified only (proven by bots, not by a person at a window).

**Runs in any environment.** Strong on paper and by test: the server builds without a GPU, window or audio (CI-enforced boundary), `doctor`,
a software-adapter fallback for rendering, env-var configuration, SIGTERM handling, a container image, systemd unit, LF everywhere, PowerShell
and bash twins. **Not proven:** nothing was run on Linux. The Docker daemon and the WSL distro on the development machine had no Rust or were
stopped, and installing software was outside this task. The new Docker CI job and the existing `headless-linux` job have never executed.
macOS is untested. Treat "runs on Linux" as *very likely, unverified* until the first CI run is green. Cheap next step: push and read that run.

**Efficient for AI agents.** Orientation is about 1.4k tokens (`CLAUDE.md` + `describe --brief`); a first playable map is one blueprint; every
failure names its cause and writes a picture; docs are tested against the repo; a resume takes one command. Remaining cost centres: the
37 KB `AGENTS.md`, the reachability flood (3.4 s on a big map, run by lint, reach checks and diagnoses), and no changed-files test runner.

## Recommended next steps

1. Push and read the first Linux CI run (headless job, Docker job). Fix whatever it finds. Everything else is easier once that is green.
2. Authenticated encryption for the protocol, then a `--passkey`/token join flow. This is the gate for "hosted for strangers".
3. Port Cheddar onto a game project: blueprint for the rooms, `prefab_files` for `assets/market.json`, `extra`/prefab `fill` for shelving, its
   rules under `scene`. Whatever the blueprint cannot say is the list of vocabulary to add (probably prefab fill, rows, non-rect rooms).
4. Split `AGENTS.md` (workflow vs reference) and let `search` serve the reference; extend `tests/ai_tasks.rs` with "make a two-room game"
   and "fix a failing walk" so the context budget of the new tasks is enforced.
5. Speed the reachability flood (coarser grid for coarse questions, incremental updates after one `patch`), and a `scripts/dev test-changed`.
6. Port Cheddar's `ui.rs` to `ui::Layout` (its overflowing "CANNOT FIND THAT ADDRESS ..." message is exactly what `label_wrapped` fixes).
7. Optional: UPnP mapping (`red_server --upnp`), a practice-bot mode, `mcp_server.py` wrappers for the new commands.

## What is verified, and what is not

Verified here (Windows, Rust 1.98): the full test suite in the default and the headless (`--no-default-features`) configurations, clippy with
warnings as errors in both, `cargo fmt --check`, the wrapper script end to end (scaffold a project outside the repo, `scripts/red check` green),
two headless bots on a blueprint-built map, the real Cheddar map through `validate`, `lint`, `reach`, `verify` and a reproduced walk failure.
Not verified: Linux and macOS, the container image, any graphical client session driven by a human, the new `ui` screens inside a running
`re2` window (the paint functions are tested and the old API is unchanged, but I did not play with them).
