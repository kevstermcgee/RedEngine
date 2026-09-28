# 2026-09-28. Seeing what the player sees: a headless client, offscreen captures, a scripted player, and a counter for every silent failure
Status: accepted
Summary: The real client can be stepped without a window, photographed offscreen, played by a script and asked what it draws; every "skip it and carry on" now counts itself and `lint`/`game check` catch the classic causes.

## Context
Trigger Happy shipped with invisible bots. Its tests had a human opponent on a duel map, while the shipped game gave its opponents other bodies, and the client's avatar pool had been sized by a rule that ignored bots' bodies; the code that draws remote players said
`None => continue` when it had no avatar for one, so nothing anywhere said "seven fighters exist and none is drawn". Looking at the game took about nine one-off PowerShell scripts (focus tricks, `CopyFromScreen`, contact sheets) that photograph whatever is on top of the desktop; two runs
at once photographed each other. The glue that decides what a player sees lives in `bin/re2`, which is welded to window and GPU types, so it had no test.

## Decision
- **The client runs without a window.** `re2 --headless` steps the same `App` the windowed client steps (`update` + `draw` at 60 game frames a second, no event loop), with a null renderer, or an offscreen one when pictures are asked for. `--script play.json` plays it, `--dump state.json`
  writes what it would show. The exit code is 1 when an expectation fails, the session reported a warning (a player nobody can see) or a picture could not be taken.
- **Pictures come from the frame buffer, not the screen.** `capture::Capture` is an offscreen target of the renderer's format; `re2 --shot-at 5,10 --shot-dir out/`, F12 and a script's `shot` step render a second time into it and read it back. No focus, no visible desktop, no cross-talk between runs.
- **A player as data.** `playscript` (`{"policy", "steps"}`: `wait`, `look`, `turn`, `hold`, `jump`, `interact`, `switch`, `fire`, `aim_at`, `view`, `shot`, `snapshot`, `wait_for`, `expect`) is a pure runner over a `Driver` trait, tested with a fake driver; the client implements the trait for `App`.
  `expect` addresses the state dump by JSON pointer, so "8 fighters means 7 drawn" is `{"expect": {"at": "/remote/drawn", "eq": 7}}`.
- **The state dump (`re2-dump/1`)** lists the remote players in view and which avatar wears each, the HUD text, the sounds played, the crosshair and the session's counters. Its shape is documented by `re2 --debug-help`, whose table of `RE2_*` switches and hotkeys a test keeps complete.
- **`red_engine2 playtest <map>`** finds `re2`, hosts a match, plays a generated script (spin, walk, aim, fire, overview shots) and prints a verdict plus a labelled contact sheet (`tools::sheet`), or the JSON with `--json`.
- **Nothing fails silently.** `NetSession` counts every way a remote player can fail to appear (`RemoteStats`: in view / drawn / stand-ins / undrawn / unposed / hidden by interest, the avatar pool's size and use; `SessionCounters` over the whole session) and records a warning the first time a player cannot be drawn (it names the player, the body and the pool); `RE2_STATS` and the F3 overlay show the counters.
  A player whose body has no avatar now wears a stand-in of another body rather than being skipped. Lints catch the causes before play: `interest` (a multiplayer map whose zones are not linked by portals, or whose portals are further apart than `interest.hops`), jump clearance computed from the map's
  own `player` tuning, and `game check` verifies that every roster body has an avatar.
- **Pools have an owner.** `scene_pool::ScenePool` owns sizing, claiming and hiding of the scene objects that must exist before the renderer does (avatars, tracers, sparks, remote weapons). Hidden objects are skipped by the renderer (`LiveRenderer::set_hidden_objects`, a precomputed per-mesh flag) instead of being drawn scaled to
  0.0005, so the pool costs no draw calls.

## Consequences
- What the player sees is asserted in CI without a GPU window: the headless client runs anywhere the engine builds, pictures need only an adapter.
- Games get a way to test the real client with the real roster (the mistake behind Trigger Happy's invisible bots); `game check` and `lint` name the most common silent failures.
- The dump and script formats are versioned (`re2-dump/1`) and are meant to grow additively; the existing `RE2_*` debug switches are unchanged.
- Costs: the client has a second entry path to keep working (`headless.rs`), and a little state is only meaningful with a renderer (the null renderer reports what it would have drawn, not pixels).
