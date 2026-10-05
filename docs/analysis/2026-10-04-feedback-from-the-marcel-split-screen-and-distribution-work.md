# Feedback: Marcel's world, split-screen co-op, versioned installers, cascaded shadows and a new boy (revision 4021e3a)

Source: the AI author's notes from one long session on RedEngine: the audio phases, Linux authoring binaries, native split-screen (up to four local players), a
distribution rebuild (installers, in-game updater, every version on the site), cascaded sun shadows and a redesigned Marcel. Everything below happened; **Triage**
lines say what the author would do first. Anything not measured is marked *unmeasured*.

## What worked (keep)
- **Look before you believe.** `frame`, `ui-shot`, `splitshot` and headless `--script` shots turned every visual change (shadows, the boy's face, the pause menu) into
  a picture checked in seconds. The pause-menu change took one `ui-shot` and the layout audit (`ui-check`) said it was sound at nine sizes.
- **Pure modules with tests before pixels.** `splitscreen.rs`, `shadow.rs` and the release planner were arithmetic first; the GPU work then had little left to go wrong.
- **`preflight` and `features.json` ownership.** They caught a missing `RE2_PLAYERS` help entry, an unowned module and an unowned test, each with the exact edit.
- **Hosted CI on Windows.** It found a real Direct3D shader failure no Linux run could (item 1). Keep Windows in the matrix.
- **Dry-run releases on a real Windows runner.** Installing, updating, uninstalling and starting a game on the runner caught problems (a hung `Start-Process -Wait`,
  relative paths in the installer script) that reading the script never would.

## Opportunities, ranked by what they would have saved

1. **Windows-only shader errors are invisible until hosted CI.** My shadow lookup sampled a shadow map (`textureSampleCompare`) inside a loop with a varying
   trip count. wgpu on Linux accepted it; Direct3D's compiler (FXC) refused the pipeline: `gradient instruction used in a loop with varying iteration`. It cost a
   full CI round trip (roughly half an hour) and, had it merged, would have broken every Windows player. *Want:* a test that compiles every WGSL shader with naga's uniformity
   analysis treated as an error (and, better, translates it to HLSL), so this fails on a laptop. **Triage: do first.**
2. **`src/lib.rs` edits escalate `affected` to the whole suite.** Adding one `pub mod` line made `scripts/dev affected --quick` run `scripts/ci.sh` (18 minutes, 2401
   tests). That happened for both new modules this session. *Want:* escalate on feature-gate or `use` changes, not on a bare `pub mod` addition (the new file's
   own feature already owns its tests).
3. **The `RE2_*` help test only scans a hand-written list of files.** `help.rs` `include_str!`s ten of the eighteen files in `src/bin/re2/`; `ambient.rs`, `avatar.rs`,
   `cards.rs`, `controller.rs`, `coop.rs`, `project_browser.rs` and `window.rs` are not scanned, so a switch added there passes while undocumented. A name read in the library
   (`RE2_SAVE_DIR`) cannot be listed at all: the test then fails with "documented but no code reads it". *Want:* scan the directory (build script or `include_dir`),
   and let the table mark library-read names.
4. **Identity by folder path lost players' progress.** Settings and saved variables were keyed by a hash of the game's directory, so every ZIP update (a new folder)
   dropped, for example, Marcel's days lived. Fixed here with `RE2_SAVE_DIR`, but it points at a general rule: save data needs a stable game id, not a location.
   There are now three save mechanisms (`settings.rs`, Killchain's `stats.rs`, the host identity directory). *Want:* an `id` in `game.json` and one `savedata`
   module that all three use.
5. **Characters cannot express a smile, a ring or a decal.** `PrimKind` is box, sphere, cylinder, cone, capsule and plane. A mouth needed two flat ellipsoids
   turned to lie along the face (one dark, one skin-coloured laid over it); a chain of capsules read as a string of beads because the silhouette-outline pass
   draws an edge at every capsule join. *Want:* a torus/arc primitive and a lathe or extrusion profile; an outline pass that ignores edges between parts of the same
   object (an id buffer per object, not per part).
6. **Free-camera shots are in world coordinates only.** To photograph the boy I guessed his position (the spawn, (0,0,0)) and hand-placed the camera, the same trap the
   Moonlight feedback names for pickups. *Want:* `{"shot": "x", "camera": {"around": "player", "distance": 2.2, "yaw": 30, "look_at": "head"}}`, and a
   `character <name> --sheet out.png` command that renders any character from front, side and back plus a face close-up (I built a throwaway scene file for this).
7. **The clock cannot be set in the live client.** `--hour` exists for `frame` only. To see Marcel at 10:00 in the headless client I copied the whole scene and edited
   `clock.start` (and found `day_secs` is limited to 20..86400). *Want:* `re2 --hour 16.5` and a script step `{"set": {"hour": 16.5}}`.
8. **GPU cost is only visible as an ignored test on a software rasteriser.** The shadows cost about +30% on llvmpipe (232 to ~305 ms, geometry-bound); I could not
   say what a real GPU does (*unmeasured*). The data exists (`DrawStats`, now with `shadow_tris`), but nothing records it per change. *Want:* a non-gating job that
   appends triangle counts and frame times for a fixed set of scenes to a file, so a trend is visible.
9. **Shader constants are copied by hand.** `scene.wgsl` repeats the atlas size (`3072` and `2048`) that `shadow.rs` owns. `gpu::tests` checks the `Globals` struct
   layout, not constants. *Want:* prepend generated `const` lines to the shader source (as `common.wgsl` is already prepended), or add a test comparing them.
10. **The `App` is one player's state spread over flat fields.** Split-screen works by swapping a `PlayerCtx` into it (`swap_player!`). The macro destructures the
    context so a field missing from the *context* fails to compile, but a per-player field added to `App` and forgotten is silent. *Want:* move the per-player fields into
    one `Local` struct on `App`; then the swap is one `mem::swap` and the check is total.
11. **Positional UI helpers do not grow.** `pause_layout(w, h, map, message, hover, music_on, sfx_on)` has seven positional arguments; adding the game's own line needed a
    second function (`pause_layout_with`) and enum. *Want:* a `PauseScreen` options struct like `ScreenOpts`, so the next addition is a field.
12. **`lint` rejects the flagship world.** `red_engine2 lint examples/marcel/marcel.json` reports `ERROR [leak] the player can walk off the map`, which cannot be true of an
    endless procgen world (CI does not run `lint` on it, so nothing flagged it). *Want:* skip the perimeter check for scenes with `procgen`.
13. **Every game carries its own engine.** Each release is two ~7.5 MB files per game because the game folder includes `RedEngine.exe`, and an engine fix reaches a
    game only when that game is re-released (`force`). *Want (larger):* install the engine once per version and games as content packs that name the engine they need;
    updating the engine then updates every game, and a game download is kilobytes.
14. **Index contracts are comments.** `human_parts` promises "parts `0..12` are the bones in this order" (the bat welds to part 3). The new `boy_parts` keeps it by
    construction, but no test pins it. *Want:* a test that every style's first twelve parts are the bones' shapes.

## Not engine defects (the author's own)
- Several hosted-CI failures early in the session were clippy warnings in test code I had not run `cargo clippy --all-targets` on; `scripts/dev iterate` covers this
  and I should have used it.
- The first dry run of the installer smoke test hung until I cancelled it (PowerShell's `Start-Process -Wait` also waits for the game the installer starts); a second run
  failed once and passed twice with no code change, which I attribute to a game process left running, not shown.
- The first shadow bias and the first smile were wrong and were corrected by looking at renders; neither needed an engine change.

## Not verified
Code signing (no certificate), the installers on a player's own desktop (only the runner), any physical gamepad with split-screen, and GPU timings.

## Status (2026-10-05)
- **Done:** 1 (shader validation test, ADR 2026-10-05), 2 (`affected` no longer escalates on a bare `pub mod`), 3, 9 and 14 (drift guards), 8 (`splitshot --stats`, `benches/render_trend.py`, the render-trend workflow), 4 (`game.json` `id`, ADR "A game's progress is filed under its id"; the Killchain stats file and host identity were left alone on purpose, see the ADR).
- **Open:** 5 (torus/lathe primitive, per-object outlines), 6 (`around: player` camera, `character --sheet`), 7 (`--hour` in the live client), 10 (`Local` struct), 11 (`PauseScreen` options), 12 (`lint` on procgen worlds), 13 (engine installed once).
