# Known traps, rejected approaches and check codes

Institutional memory for agents, one entry per ID. `red_engine2 context <ID>` prints one; `context "<task>"` lists the ones owned by the
features it routes to; `search` finds them. Keep entries short and grounded in something that actually went wrong or was actually decided.
**Retire an entry** (delete it) once a tool or test makes the mistake impossible; point to the ADR instead of repeating it.

Kinds: `trap` (a mistake agents make), `check` (the ID a failing guard test prints), `decision` (an approach already rejected).
Fields: `features` (owners in docs/features.json), then `symptom`/`cause`/`dont`/`look`/`verify` for traps and checks, or
`rejected`/`reason`/`reconsider`/`adr` for decisions.

## HEADLESS-001 — a graphics or audio crate reached the headless build
- kind: check
- features: build_and_ci
- symptom: `tests/headless_boundary.rs` fails naming `src/<file>: wgpu::` (or winit, rodio, pollster, ffmpeg_sidecar); or `scripts/ci.sh headless-tree` lists a graphics crate
- cause: a module the server builds (`--no-default-features`) names a gfx-only crate, or a new gfx-only module is not gated
- dont: add the crate as a non-optional dependency; silence the test
- look: gate the module with `#[cfg(feature = "gfx")]` in its parent (`src/lib.rs`, or `src/app/mod.rs` for the client layer) and list its path in `GFX_ONLY` in `tests/headless_boundary.rs`; keep shared logic in a graphics-free module
- verify: `cargo test --test headless_boundary` then `scripts/ci.sh headless-tree headless-build`

## SIM-001 — the simulation tree names a renderer
- kind: check
- features: sim_core
- symptom: `sim::tests::sim_tree_has_no_renderer_imports` fails
- cause: code under `src/sim/` mentions wgpu/winit/rodio; the server, `sim`, `replay` and the tools run it without a window
- dont: move the file out of `src/sim/` to dodge the check
- look: keep the rule as a pure function in `sim/` (input state in, new state out) and do the presentation in the client
- verify: `cargo test --lib -- sim::tests`

## FEAT-001 — a file has no owner in the feature graph
- kind: check
- features: self_description
- symptom: `features --check` (and `tests/features_index.rs`) reports a source file with no owner, or a listed file/suite/doc that does not exist
- cause: a new module, test suite or doc was added without a `docs/features.json` entry; `context` and `affected` cannot see it
- dont: add a catch-all glob to an unrelated feature
- look: add the path to the feature that owns the behaviour (`context <file>` shows the neighbours); new subsystem = new feature with `depends_on`
- verify: `red_engine2 features --check`

## DOCS-001 — a CLI command is missing from the reference
- kind: check
- features: docs_and_adrs
- symptom: `tests/docs_fresh.rs` `every_cli_command_is_mentioned_in_agents_md` fails naming a command
- cause: a new subcommand was added to `src/cli/args.rs` without a row in the tool table of `docs/AGENT_REFERENCE.md`
- look: add one row next to its neighbours (command, what it does, key flags)
- verify: `cargo test --test docs_fresh`

## DOCS-002 — a new ADR or fact is not registered
- kind: check
- features: docs_and_adrs, self_description
- symptom: `docs_fresh` fails on the ADR index, the search index or the CLAUDE.md facts block
- cause: an ADR file must also be listed in `docs/adr/README.md` and embedded in `ADRS` in `src/tools/search.rs`; counts in CLAUDE.md are generated
- dont: edit the facts block by hand
- look: add the README row and the `include_str!` line; run `red_engine2 status --sync-docs CLAUDE.md`
- verify: `cargo test --test docs_fresh`

## VERIFY-001 — a check that ran nothing is not a pass
- kind: check
- features: self_description, build_and_ci
- symptom: `affected` prints `FAIL [empty-selection]` (a `--test X` that ran no tests, or a library filter that matched none), or `features --check` says a library test filter selects no test
- cause: `docs/features.json` names a test module or suite that has no tests (or lost them); before this check such steps exited 0 and counted as verification
- dont: add a dummy test to make the selection non-empty
- look: list the suites that really cover the module (`context <file>`), or write the missing unit test; cargo filters are substrings, so name the module exactly
- verify: `red_engine2 features --check` then `scripts/dev affected --quick`
## SPAWN-001 — a scene with no `spawns` starts at the camera
- kind: trap
- features: scene_format, map_analysis
- symptom: the player starts inside a wall; `lint` reports `spawn overlaps a collider` or a `leak` at an odd place; a test expecting "no spawns" to be an error passes a scene through
- cause: `parse_spawns` falls back to the scene camera's x/z and heading when there is no `spawns` array, and `lint`/`reach` use the same point
- dont: move walls to make room for the camera
- look: add a `spawns` array, or put the authored camera over the intended start
- verify: `red_engine2 lint <scene>`

## TICK-001 — tick counts from floating-point seconds
- kind: trap
- features: sim_core
- symptom: a test expecting `advance(0.1)` to run 6 ticks gets 5
- cause: `TickClock` accumulates `f32` frame times against `TICK_DT` (1/60 as f32); 0.1 s is 5.99 ticks
- dont: loosen the clock or add epsilon hacks in the simulation
- look: step whole ticks (`step`, `tick_once`) in tests, or use `sim::clock::secs_to_ticks`; give real-time tests a little slack
- verify: the test itself, run twice

## LINT-001 — static lint cannot see rule-driven collision
- kind: trap
- features: map_analysis, game_rules
- symptom: `lint` reports a zone or exit unreachable although the game is winnable (a gate opened by a rule)
- cause: `lint`/`reach` analyse the authored geometry; `rules` `collision`/`hide` actions change it only at run time
- dont: `lint_ignore` the walls or delete the gate
- look: prove the route with `checks.sim` scenarios (walk through the switches, expect the outcome)
- verify: `red_engine2 sim <scene>` (or `verify`, which runs them)

## RE2-001 — re2 does not update until the mouse is captured
- kind: trap
- features: graphical_client
- symptom: a scripted screenshot of `re2` shows no rules HUD, or a frozen world, offline
- cause: single-player `update` (and the HUD repaint) returns early until the window has captured the mouse (a click)
- dont: remove the pause-when-unfocused behaviour
- look: click the window in the script, or check the 2-D screens without a window (`ui-shot`)
- verify: look at the image

## NET-001 — prediction that keeps correcting
- kind: trap
- features: net_client, player_physics
- symptom: a client converges and then corrects again and again (`net-test` "corrections stay small" fails)
- cause: the prediction path stopped calling the same movement step the server runs (tuning, jump pads, momentum)
- dont: hide it by raising the interpolation delay or the correction limit
- look: `sim::player::step_player_tuned` / `step_player_on_tuned` and `net::predict::Predictor::reconcile_tuned` must receive the scene's tuning
- verify: `red_engine2 net-test examples/test_lab.json --profile bad`

## LOOP-001 — a test or tool binds a public address
- kind: trap
- features: build_and_ci, net_server
- symptom: an unattended run stalls on an OS firewall prompt
- cause: a socket bound to `0.0.0.0` (a wildcard) instead of loopback
- dont: add a wildcard bind to a test or tool
- look: bind `127.0.0.1:0`; clients use `net::client::local_bind_for`; hosting is the explicit `--public`/`--bind`/`--upnp` (ADR 0042)
- verify: the run completes without a prompt

## DEC-ASSETS — imported meshes, textures and sound files
- kind: decision
- features: offline_renderer, scene_format
- rejected: importing model/texture/audio files
- reason: procedural props and JSON prefabs keep assets diffable, validated and authorable by an agent
- reconsider: a game needs art that cannot be built from primitives, and a validation/lint story exists for it
- adr: 0008

## DEC-FORK — a game as a fork of the engine repository
- kind: decision
- features: blueprints_and_games
- rejected: copying or forking the engine to make a game
- reason: the fork's docs and engine fixes drift; games pin the engine and use it as a library (`new-game`, `red_engine2::app`)
- reconsider: never for games; a genuinely separate engine is a new repository
- adr: 0024

## DEC-RUSTAPI — a Rust builder API for scenes
- kind: decision
- features: scene_format
- rejected: a `SceneBuilder` / prelude for authoring maps in Rust
- reason: JSON scenes are cheaper to generate, validate, diff and patch; one source of truth
- reconsider: a class of content that JSON cannot express compactly
- adr: 0001

## DEC-CLIENT-AUTH — clients sending positions
- kind: decision
- features: net_protocol, net_server, sim_core
- rejected: a client telling the server where it is, or applying gameplay locally as authority
- reason: inputs only; the server runs the shared simulation; prediction replays the same step
- reconsider: never
- adr: 0016

## DEC-WHOLE-SUITE — the whole test suite in the edit loop
- kind: decision
- features: self_description, build_and_ci
- rejected: `cargo test` / `scripts/dev test` after every edit
- reason: `affected --quick` then `affected` verify what a change can reach; full CI stays the fallback at boundaries
- reconsider: the feature graph proves unreliable (a miss found by CI that `affected` skipped)
- adr: 0042
