# Browser reliability milestone: what was wrong when features were combined, what was changed, what is still not proven

Scope: the 2D / WebAssembly / browser workflow and its interaction with the simulation, the procedural world, verification and publishing. Start: `origin/main` at `316c8d7` (then merged `88a8165`, 3D phase 1).
Every finding below was reproduced before it was fixed, and each fix has a test that fails without it.

## What was found, per phase

| # | Finding | Fix | Test that fails without it |
|---|---|---|---|
| 1 | `publish` staged `webgames/` and ran a plain `git commit`: anything somebody else had already staged was committed with the game; another game's uncommitted edit reached the catalog; a failed push left a commit behind. | `tools::gitscope`: the commit is built from `HEAD` plus the game on a temporary index (`commit-tree`), the checkout follows by `read-tree -m -u`, the branch moves by compare-and-swap, a failed push is undone, unsafe states are refused before anything is written with the exact fix. ADR `2026-10-06-publishing-builds-its-commit-from-head-on-a-private`. | `tests/publish_git_isolation.rs` (15 cases) and one whole-command case in `tests/web2d_publish.rs` |
| 2 | In the browser runtime a key, focus or Start event before WebAssembly had loaded reached `wasm.exports` while `wasm` was null; the throw hit the global error handler, which turned a page that was merely still loading into a fatal error. A failed load left the start card, spinner and touch pad in place. | One state model: `live()` gates every reaction (keys, pointer, touch pad, gamepad, focus, visibility, `begin()`); earlier input is counted in `status.early_input` and dropped; a failure hides the start card, spinner and pad, releases everything, stops audio and keeps the first error; focus return and visibility return resync the clock. | `web verify` phase F: 11 kinds of input while `game.wasm` is held pending, then the page must finish exactly like an untouched one; four failing loads (not wasm, 404, 500, refused game). The old runtime fails them with `Cannot read properties of null (reading 'exports')`. |
| 3 | Four places assembled "what is solid and how high is the floor" by hand: the client, `MatchSim`, `ClientWorld` (online prediction and bots: **no generated world at all**) and `MapWorld` (every analysis tool: dropped the scene's terrain as soon as a rule or phase opened any gate). | `collide::PhysicalWorld`, the one definition; all four use it; a guard test fails if another module assembles ground from groups. ADR `...one-physicalworld...`. | `tests/ground_consistency.rs`: a pen on generated hills and trees with a gate, a lever rule and a phase; five consumers agree, closed and open; opening the gate removes exactly the gate's collider. Dropping the scene ground fails all of them. |
| 4 | One success flag could not say whether a game kept its save, played sound or survived going offline; `native passed` was hard-coded `true`. | `tools::evidence`: 20 pieces, each `passed`/`failed`/`not_run`/`not_applicable`, five levels that never merge, `publication.json`, `web status`. ADR `...publication-evidence...`. | unit tests in `tools::evidence`, `tests/web2d_publish.rs`, and `scripts/web_check.sh` fails if an applicable piece did not pass |
| 5 | (Found by building the combination game.) A world that depends on saved progress could not be rebuilt on load (`start` rules run before the save is read); the browser reset and backup phases only performed clicks, so a keyboard-driven game could not be checked for them; 2D games had no map analysis. | the built-in `loaded` event; `Sim::can_reach` + `{reach}` expectation + `checks.reach`; `do_input` in the verifier. ADR `...2d-map-analysis...`. | `tests/gate_meadow.rs`, red2d unit tests |
| 6 | Imperfect checkouts: see 1. Added: interrupted run, held index lock, crashed scratch, vetoed ref update, leftovers byte-identical (cleared) or different (refused with the recovery command). | `gitscope` | the same file |
| 7 | A fresh model had to find `describe 2d`, then guess the loop, the publish options and how to read the result. | `describe web`, `web status G` (the exact next command), pointers in the brief, AGENTS.md, CLAUDE.md, the new-game template, PUBLISHING_2D.md. | `tests/web2d_games.rs` (describe web answers every question and stays one page; status walks a new game) |
| 8 | "3D in a browser" meant four different things, and `capabilities` still said nothing builds for wasm. | `describe web3d`, honest `capabilities`, a measurement and parity harness; see `2026-10-06-3d-browser-measurements.md`. | `tests/web2d_games.rs`; the harness itself |
| 9 | Automated Chromium was described as the only verification. | `docs/DEVICE_QUALIFICATION.md`, device records read by `web status`, `web verify --engine firefox`. | unit tests in `tools::webstatus` |
| 10 | The starter broke when a gem was added (hard-coded counts in its expectations) and did not prove a save and reload in the browser. | the starter says what must hold, not how many there were, and proves save and reload; `bench/fresh-agent`, `RED_TRACE`, `scripts/agent_bench.py`. | `tests/fresh_agent_bench.rs` |

## The combination game

`examples/2d/gate-meadow.game2d.json`: hybrid presentation; ground = the engine's generated world (seed 7, a 30 x 12 window pressed into a tile map: 19 trees block, raised ground is drawn in 3D); a fence and a gate that a
key opens by a rule; saved progress that rebuilds the open gate on load; music and three sound effects; keyboard, mouse and touch; installable and offline (every 2D game is). Automated: initial state, the fence holds,
crossing generated terrain, the gate opening, every tree still there, saving, reload, continuing from the save, winning; the analysis (`checks.reach`) and the play-through agree about what the gate keeps out; in a real
browser: key opens gate, progress written, reload, gate open, win, music button. `tests/gate_meadow.rs` also keeps the committed map equal to what the generator makes.

## Evidence model (what `publication.json` says)

Levels: `built`, `locally_verified`, `uploaded`, `remotely_playable`, `human_playtested` (the tool never sets the last). Pieces: `native_scenarios`, `wasm_compiled`, `browser_package_valid`, `wasm_instantiated`,
`loading_robustness`, `playable_state`, `input_keyboard`, `input_pointer`, `input_touch`, `input_gamepad`, `persistence_write`, `persistence_reload`, `audio_api`, `audio_playback`, `offline_cache`, `offline_reload`,
`installable`, `browser_scenarios`, `other_browsers`, `remote_deployment`. A local run of gate-meadow: 17 passed, 1 not applicable (gamepad), 1 not run (remote), plus `other_browsers` when Firefox ran.

## The fresh-agent benchmark

`bench/fresh-agent/TASK.md` is the whole prompt; `scripts/agent_bench.py score` judges the end state from the engine's own answers; `RED_TRACE` + `summary` report the friction (documentation topics read, failed commands, retries,
repair cycles, CLI source exploration; with a transcript, direct reads of engine source). The reference agent (what `describe web` says, nothing else) passes with **8 commands, 0 failures, 0 retries, 0 source exploration**.
Running the reference agent found two friction points that were removed in the **template**, not documented: the starter's expectations counted gems (`gems_total eq 5`, `count gem eq 5`), so the first honest edit (a sixth star)
failed `verify`; and the starter did not prove a save and reload in the browser, so an agent had to invent that check. **No live model run has been made** (it spends tokens; the memory of earlier sessions says to ask first):
the harness, the command and the scoring are ready (`bench/fresh-agent/README.md`), and the table of live results is empty.

## Not done, and why

* **No real device was used.** `human_playtested` is false everywhere; `docs/DEVICE_QUALIFICATION.md` is the procedure.
* **WebKit** is not installed here (its system libraries need root): `web verify --engine webkit` exists and is untested. Firefox was run.
* **3D**: one bounded step (see the measurements file). No packaging, no WebGL2, no audio, no touch; the numbers come from a loaded machine and a software adapter.
* **No hosted CI** has run on this branch. The branch is local; nothing was pushed and no pull request was opened.
