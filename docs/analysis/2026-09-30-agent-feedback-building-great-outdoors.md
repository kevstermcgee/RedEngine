# Building a whole game on Red as an AI agent: what worked, what cost time, what changed (2026-09-30)

Written by the agent that built Great Outdoors (an 8-animal hosted kart racer: engine karts, races, pickups, bots, protocol, prediction, HUD, camera, gamepad, lobby,
sounds, a generated forest map, hosting and publishing) from an empty idea to a published playable in about two sessions. The per-phase numbers are in
`benches/history/great-outdoors.json`; the engine frictions found phase by phase are in `2026-09-29-great-outdoors-engine-findings.md`. This note is the whole-build
view: what an agent should expect, ranked by how much time it cost, and what was changed in the engine in answer.

## What made it fast (keep these)

1. **The engine describes and checks itself.** `describe`, `search`, `context`, `src show` answered "where does this live" in a few KB; `preflight` named the exact edit for
   every doc, feature-index, budget and derived-fact omission. I never had to hold the repo's conventions in my head; the repo told me when I broke one.
2. **One pure shared step function.** Because the server, the client's prediction and the bots all call `step_kart_ex`, a new movement mode was identical everywhere by
   construction, and prediction was bit-exact on the first real network run.
3. **Headless everything.** `race-test` (bots race the real tick), `frame` (render a picture with no display), `lint` (found an actual gap in the barrier ring and 235
   z-fighting planes), `ui-shot`/`ui-check` (screens audited at nine window sizes), `playtest` (a real client, pictures). Every visual bug was found by *looking*, and looking
   cost seconds.
4. **Deterministic test clocks** (`Server::pump(now)`, `tick(now)`, `RawClient`) made wire tests quick and stable.
5. **Small verified commits** with one gate (`scripts/ci.sh`) meant I always knew the last good state.

## What cost the most time (ranked), and what changed

| # | Cost | What happened | Answer in the engine |
|---|---|---|---|
| 1 | Hours | A raceable map (loop, barriers with no gaps, gates, boxes, grid, bot line, animals, match block, checks) was ~400 lines of hand-written Python, iterated with 8 render/lint/race-test rounds. Every kart game needs the same thing. | **`red_engine2 race-track OUT.json`** generates it from a few numbers, lint-clean, bots finish it (tested at two sizes). **`new-game --kind race`** scaffolds a whole race project around it. The eight animals are now the core **`karts`** pack (`catalog karts`), so no game re-draws them. |
| 2 | ~1 hour | A shortcut trapped bots: a missed line point behind a wall pinned them for the rest of the race, and the bot doc *promised* recovery that the code did not do. The tool reported only "did not finish". | Bots back up to the previous line point when wedged twice at one; **`race-test` says where a bot that did not finish stopped**. |
| 3 | ~1 hour | Hosting and publishing were manual, undocumented-in-one-place steps: build the headless server, make an identity, invent a join key, write a systemd unit, copy the project into RedEngineGames without scratch files or keys, hand-edit `.release-games.json` in its own style. | **`deploy/install.sh`** in every scaffold (reads `game.json`; `--info`, `--uninstall`); **`game publish ../RedEngineGames`** (never commits; refuses keys and env files); both documented in HOSTING.md / GAMES_PUBLISHING.md. |
| 4 | ~1 hour | Hosted play had no lobby and no second race: without a `match` block a map is open play, and nothing ended a race round. | A finished race ends the round, the winner is first place, the lobby character byte is the animal (picker with keys, d-pad, bumpers). The race scaffold ships the `match` block. |
| 5 | 30 min each | Object rotation composes `Rx*Ry*Rz` about world axes, so `[90, yaw, 0]` lay a cylinder across the road; `frame --eye` from 300 m rendered pure sky because `camera.far` was 200. | `describe objects` states the composition and the nested-group recipe; **`frame` warns** when the eye is farther than `camera.far`. |
| 6 | Many small edits | Adding one concept touched many files (scene key list, `check_sections`, `Scene` literal in two places, `describe`, SPEC, protocol, server, session, client, HUD, features.json), and a regex mass edit corrupted signatures. | `PlayerSnap` and `Snapshot` derive **`Default`** (a test builds them with `..Default::default()`); the rest is still open (below). |
| 7 | 10 min x3 | Test-only clippy lints passed my lib-only runs and were caught by the CI stage. | Written into the findings note: run the full `--all-targets` stage before committing. (The gate already does; the trap is local shortcuts.) |

## Still open (highest value first)

1. **Per-player rule variables.** Laps, lives, per-player scores could not be expressed as rules, which is why `sim::race` is native. The largest missing capability for every future game; it would have covered laps and would make "round ends when" data too.
2. **Round end as data.** `rules_outcome`, score and now "race complete" are hard-coded in the server's `step_flow`; a scene-level `match.ends_when` would make new game modes configuration.
3. **One registry for scene keys.** A new scene block still touches the key list, `check_sections`, the `Scene` struct and parser, the `menu.rs` literal, `describe` and SPEC. A builder/`Default` for `Scene` and a single table the others derive from would make it one edit.
4. **Clients receive a destructured `NetEvent::Snapshot { own, ack, race }`**; every new snapshot part widens it in the session and the bot. Hand clients the decoded snapshot.
5. **Prediction glue is duplicated** between `net::session` and `net::bot` (partly shared through `reconcile_snapshot`).
6. **The green light costs one 0.1 m correction per race** (the predictor learns the phase from a snapshot after the server released the kart). Needs per-step race phase in replay.
7. **`sfx` is graphics-only**, so a pure cue module that returns clips had to be graphics-gated; split cue logic from clips so it tests headless.
8. **`race-track` makes one shape** (a rounded rectangle). Splines or a polyline centre line, branches (shortcuts) and jumps would cover real tracks; Great Outdoors' shortcut, mud and scenery are still in its own generator.
9. **Not measured**: a physical gamepad, windowed feel of the picker and sounds, eight real clients on the hosted service, play from outside the LAN.

## Advice to the next agent

* Start a kart game with `new-game --kind race`, `race-test`, `frame`, and change numbers before you change code.
* When a tool reports a failure, ask whether it says *where*: two of this build's lost hours were "did not finish" and "sky only" with nowhere to look. Add the where.
* Do not trust a doc's promise about behaviour you have not seen (the bot's stuck recovery); read the function or test it.
* Run the whole gate before committing; test-only lints and headless-build gaps (`sfx`) only show there.
* Never `pkill -f` (it matches your own shell) and never `git stash` mid-session by reflex.
