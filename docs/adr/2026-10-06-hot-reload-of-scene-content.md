# 2026-10-06. Hot reload of scene content
Status: accepted
Summary: re2 and web serve --watch re-validate a saved scene or game and apply it in place, keeping the player; an invalid save changes nothing and shows validate's words

## Context
Every content edit cost a restart: for `re2` a new process, scene load and (windowed) device start; for a 2D game `web build`, a manual page refresh and a lost play position. An agent making
many small edits pays that on each one, and an invalid edit used to mean a crashed or refused start rather than a message while the old content kept running.
Baseline, measured on the 4-core dev box (examples/test_lab, house, office): a headless `re2` process start to first frame takes 0.65-0.72 s, 0.65-0.72 s in process as well; a windowed restart also
pays the window and the GPU and could not be measured here (no display). 2D: `web build` of coin-dash reports 0.2 s, then a refresh by a person, and the game starts over.

## Decision
**`re2` (scene content).** `reload.rs` polls the scene file every 250 ms and acts only when its *content hash* changes (an editor that touches or saves twice causes one reload). Valid content
replaces the running content in place: `App::apply_scene` mirrors the offline half of `start_game` and derives again everything that comes from the scene (the scene with the player's body, spawns, rules,
loose props, the collision world, hit shapes, the effects pool and the renderer's meshes) while the window, device, audio, input, character, weapon, ammunition, position and view are left alone. Rule variables and loose
props start again from the new scene (their state belongs to the old content); audio and music are not restarted. Invalid content changes nothing: the old content keeps running, the same `error: ...` lines `validate`
prints go to the terminal and the first is shown on screen. Off online (the server's map hash would reject a different map), in split screen, behind the connect form, and with `RE2_RELOAD=0`. The check runs at the top of
`App::update`, so the windowed client and `re2 --headless` share it, and the tests drive the real `App`.

**2D games (browser).** `web serve DIR --watch G.game2d.json` validates `G` on every save (`game2d::validate`, the words `validate` prints) and serves the newest valid text from memory as `assets/game.json`; the
package on disk is never rewritten, so its manifest hashes still hold. Only this server adds `dev_reload` to the manifest it serves, and only a page that sees it polls `__dev/state` (400 ms), so a published package never
polls. A valid save calls the new wasm export `reload`, which re-initialises the game and `Sim::carry_over`s every entity that has a scene id in both versions (place, velocity, facing; the player is `p` by
convention), the camera and the music setting; variables and timers start as the new game says, the persisted save is loaded again. An invalid save shows validate's words on the page and the game keeps running. Snapshots now
say where the named entities are (`named`). The package self-containment check allows exactly one non-package URL, `__dev/state`, by name.

## Consequences
Measured with the real code: `re2` hot reload applies in 4-5 ms (test_lab), 8-10 ms (house) and 43-47 ms (office) plus at most 250 ms of poll, from save to the next frame about 0.13-0.30 s, against the 0.65-0.72 s restart
(`cargo test --bin re2 measure_reload -- --ignored --nocapture`; the restart figure is a lower bound). 2D in real Chromium (`crates/red2d/web/dev_reload_check.py`, run by `scripts/web_check.sh` in CI): save to visible
381-405 ms (median 403 over 5 saves), the player where they were, no page reload, validate's diagnostics on an invalid save and cleared by the fix, and a plain `web serve` never asks for `__dev/state`.
Not verified: a windowed `re2` on a real GPU (the tests drive the same `App` headlessly), Windows (hosted CI runs the unit tests), split screen (off), content that changes the character or the player's object model,
and games whose named entity is removed by an edit (it simply starts as the new game says). Not carried: rule variables and loose-prop state in `re2`; any 2D entity without a scene id.
