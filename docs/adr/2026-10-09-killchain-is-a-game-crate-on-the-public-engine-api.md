# 2026-10-09. Killchain is a game crate on the public engine API
Status: accepted
Summary: The Killchain game (its windowed client, 2-D screens, objective HUD text and statistics) is the crate games/killchain, a workspace member written only against red_engine2's public API; the engine keeps the reusable loadout-shooter simulation, protocol and kit and no Killchain front end.

## Context
The simplification pass (STATUS.md, stage S6) set out to make Killchain "a standalone game binary consuming the engine API". After its baseline (6a42dd1) the Killchain game modes landed inside the engine (f58e15d, +4,161 lines), and the game still lived there: `src/bin/re2/kc/**` (2,900 lines of
client) behind a hook in `re2`'s `main` that handed any map with a `shooter` block to it, `src/ui/killchain.rs` and `src/ui/objective.rs` (1,700 lines of 2-D screens and HUD sentences), `src/stats.rs` (the player's lifetime statistics, a `Killchain` data folder and a `KILLCHAIN_DATA` variable), and three examples. An engine change that touched
a screen had to build and test the whole engine; a second game on the same simulation had to read a first-person client that was really one game's front end.

## Decision
- `games/killchain` is a crate (`killchain`: a library and a `killchain` binary) in the workspace, **not a default member**, so the engine's plain `cargo build/test`, the headless server build and its dependency-tree check never compile it and never see its graphics crates through feature unification.
  It depends on `red_engine2` by path and uses only `red_engine2::...` items; the client had no `crate::` dependency on `re2`, which is why the move was mechanical.
- What moved: the client (`app`, `game`, `input`), `ui` (the screens), `objective_hud` (the HUD sentences for the objective modes), `stats`, and the examples `killchain_screens`, `weapons_table` and `arsenal_poses`.
- What stayed, on purpose: the **reusable loadout shooter** is engine (as `docs/features.json` already said): `arsenal`, `firearms`, `weapons`, `killcam`, `shooter_world`, `objective_world`, `uniforms`, `sim::{kit, ordnance, shooter, objective, objective_run}`, the protocol fields that carry modes
  (v15, ADR 2026-10-07-killchain-game-modes-free-for-all-capture-the-flag) and the bots. A `shooter` block in a scene is engine data, playable by any client. Nothing there names Killchain except in ADR ids and comments that cite them.
- `re2` no longer hands a `shooter` map to a game client; it says so and plays the map as an ordinary first-person scene. The game is run as `killchain MAP` (default `maps/main.json`; `KC_SCRIPT=file.json` for a scripted, windowless run; `RE2_NAME` for the name).
- CI: `scripts/ci.sh killchain` (clippy `-D warnings` and the crate's tests, in the engine's target directory so only the game is compiled); the planner (`scripts/dev affected`) adds the same two steps when `games/killchain/**` or the engine code it is built on (`src/net`, `src/sim`, `src/ui`, the renderer and kit files) changes.

## Consequences
The engine crate has no Killchain-specific code; the game is the first complete game written against the public API alone, and any engine API change that breaks it is now a CI failure with a name. Harder: a Windows package of Killchain must ship `killchain.exe`
where it shipped `re2.exe` with the map (`red_engine2 package` and `game play` still know only `re2`; the project in RedEngineGames needs its launch scripts changed, and that is not done here). Statistics and host identity live in the same folders as before. Undo: `git mv` the files back; nothing else depended on them.

## Parity (measured before and after, 2026-10-09, llvmpipe software rendering on one machine)
- **Screens**: `killchain_screens` (the engine's example before, the game's after) writes 20 screens at 1280x720 and again at 640x360: **20 of 20 pixel-identical at both sizes**.
- **Scripted client**: `KC_SCRIPT=scripts/kc/menu_tour.json` and `solo_bots.json` on a copy of the Killchain project, `re2` at 30bb4e5 against `killchain` at this commit: both exit 0, the same 13 pictures are written, and the logs are identical once times, ports and counts are masked. The pictures themselves cannot be compared bit for bit: **two runs of the same old binary already differ** (the match view and the animated menus depend on wall-clock time and a real-time server thread; 1 of 13 pictures identical run to run), and the new binary differs from the old by the same amounts as old from old (e.g. 464 pixels of 518,400 in `05-fire` both ways).
- **Simulation and protocol**: untouched; `objective_modes`, `net_modes`, `loadout`, `loadout_bots` pass unchanged in the full CI run.

