# RedCraft: what a block game needed from the engine (feedback, 2026-10-19)

RedCraft (`games/redcraft`) is the owner's "Minecraft clone called RedCraft", built in one session on main c78f6b7. Plan:
`docs/analysis/2026-10-19-redcraft-plan.md`. Decision: ADR 2026-10-19-a-block-game-rides-on-the-mode-and-layer-seams.

## What was built

| Part | Where | Checked by |
|---|---|---|
| About 50 kinds of block (70 ids with the facings of furnaces, chests and wall torches and 7 flowing-water levels), 28 other items, tools in 4 tiers, the original's mining times | `block.rs`, `item.rs` | unit tests (mining times, names both ways) |
| Generator: continents, oceans, beaches, rivers, plains, forests, birch woods, taiga, snowy plains, deserts, ridged mountains; caves (tunnels + caverns, lava below y 10); ores by depth; oak, birch and spruce trees whole across chunk borders; flowers in patches, grass, cacti, dead bushes | `gen.rs`, `noise.rs` | unit tests; `redcraft map` (biome shares printed); pictures |
| Light: sky and block light flood-filled like the original's, exact per chunk from its 3 x 3 neighbourhood, and exact after an edit by re-working the 31-column box | `light.rs`, `world.rs` | unit tests (roof shade, torch falloff, walls, open shafts) |
| Meshes: visible faces with smooth light and ambient occlusion in the vertex data, evenly lit faces merged into large quads, plant and torch models, water drawn lower as it flows | `mesh.rs` | unit tests (winding, culling, occlusion, merging) |
| Streaming: 1-3 worker threads generate, light and mesh, nearest first; edits meshed at once; far chunks dropped, their edits kept | `stream.rs` | unit tests (settle, unload and reload keep edits) |
| Look: 57 procedural 16 x 16 textures in the game's `surface` WGSL; a CPU twin paints the inventory's isometric icons; the aimed block's outline and cracks; drifting clouds | `content/shaders/blocks.wgsl`, `icons.rs`, `clouds.rs` | `redcraft shot --gallery`, `redcraft icons`, shader compiled for every backend at load |
| Play: the original's body (box collision per axis, auto-jump, sneak edge guard, swimming, creative flight), mining with tool wear, placing with facing and support rules, falling sand, flowing water, furnaces, chests, health, falls, drowning, lava, cactus, TNT | `physics.rs`, `play.rs`, `play/water.rs`, `craft.rs` | unit tests (walk speed, jump height, edges, slides, swimming, flight, falling sand, a fall's damage, water spread and drain) |
| Mobs and drops: items pop out and fly to the player; pigs by day, zombies (burn at dawn) and creepers (fuse, blast) in the dark; combat with knockback; death drops everything | `entities.rs`, `play/life.rs` | unit tests; headless night fight |
| Screens: hotbar, hearts, air, crosshair, F3 lines; inventory with 2 x 2 crafting, crafting table, furnace, chest, creative list; pause menu (music, sound, render distance, auto-jump, fullscreen, quit); title over a turning world; new-world form; world list; death screen | `ui.rs`, `screens.rs`, `mode.rs`, `mode/paint.rs` | unit tests (clicks land on what was drawn; crafting through the screen) |
| Sound and music: synthesized dig / break / place / step per material and the rest; five calm piano scores as an endless journey; nature ambience | `sound.rs`, `content/audio/` | `red_engine2 audio check` (0 failures); spectrogram |
| Saves: a folder per world, only edited chunks stored, written every 30 s, on pause and on close | `save.rs` | unit test; headless test: the next launch continues |

End to end: `cargo test -p redcraft --test play` runs the real binary headless twice (a survival morning that digs, walks, crafts and builds,
then continues the saved world on the next launch) and fights a zombie at night.

## Measured (Intel N97, dev profile, software Vulkan for pictures)

- A chunk costs 1.1 ms to generate, 4.3 ms to light and 3.7 ms to mesh (`cargo test -p redcraft what_a_chunk_costs -- --ignored`); a 6-chunk
  view settles in 0.7 s with three workers.
- Triangles drawn for one view (`redcraft shot`): a taiga forest from 95 m with a 10-chunk view **1,400,606 -> 541,668** (merging evenly lit
  faces, then grass as tapered strips: 155 -> 44 triangles a tuft); a beach **61,572 -> 21,862**. One draw per section and pass: 398 sections
  for the 10-chunk view.
- Biome shares over 4096 x 4096 blocks (seed 2): ocean 24.8 %, plains 17.9 %, forest 14.8 %, taiga 11.3 %, desert 7.5 %, birch 6.5 %, snowy
  5.0 %, mountains 4.6 %, frozen ocean 2.6 %, river 2.5 %, beach 2.3 %.
- Not measured: frames per second on a real GPU, a real window's mouse and gamepad, Windows (the shader passed the engine's HLSL check).

## Engine gaps closed while building it (see the ADR)

1. **Per-vertex data for a painted surface** (`SurfaceIn.vertex`, `material.z`). Without it a voxel game draws ~4,000 times a frame.
2. **Haze that begins at a distance** (`clock.fog_start`, `ModeContext::set_haze`). Exponential haze from the eye turned dark foliage
   blue-grey at 40 m while the world's edge still showed.
3. **A mode that moves the body reads the keys and the pad** (`key_held`, `pad`); scripts' `hold` reaches it too.
4. **See-through layer surfaces** (`WorldLayer::draw_blended`).
5. **Save on close** (`GameMode::closing`).
6. **Time of day in a save** (`time_of_day`, `set_time_of_day`).
7. **Typing in a mode's text field** (`GameMode::typing`).
8. **Every game's painted surface is compiled on Direct3D in CI** (`tests/shadow_render.rs`,
   `every_games_painted_surfaces_build_the_renderers_pipelines_on_this_adapter`). Before it, a game's WGSL reached the Windows HLSL compiler
   first on a player's machine.

## Publishing it (2026-10-19)

RedCraft was the first game published with its own binary through RedEngineGames PR #7 (a playable may name `"binary"`). Three dry runs of
`releases.yml` (`dry_run: true`) found two bugs that only show on the Windows runner, both fixed in #7: Python's output read by bash kept its
`\r` (`cargo build -p "frameless\r"`), and the install smoke test stopped only `RedEngine*` processes, so the game's own client kept its files
open and the reinstall failed with exit 5. Merging #7 re-released Frameless (v3) and Killchain (v4) with their own clients. Then the Pages
deploy failed for the whole site: `site/generate.py` requires a wiki page for every game, which nothing on the engine side checks. Lesson:
rehearse release changes with a dry run before merging; it cost three runs and found what review did not.

## Open gaps, ranked by what they would save the next game

Each is evidence from this build, not an instruction; check it against main before acting.

1. **A mode cannot own the player's body.** `input_frozen` only zeroes the input (`src/client/frame.rs:94`): the client's body still steps
   with gravity, and a layer's world (RedCraft's terrain) is not its ground, so it falls for ever. RedCraft calls `teleport` + `set_camera`
   every frame, even behind screens, and a headless run (which stays "grabbed") showed the camera falling through the world until it did.
   *Change:* a `GameMode::owns_body()` (or `input_frozen` meaning it) under which the client skips its body's physics and takes the camera
   from `set_camera` only. *Done when* RedCraft's per-frame teleport goes and `cargo test -p redcraft --test play` still passes.
2. **A mode's `frame` does not run while the cursor is free.** `update` returns early when not grabbed offline (`src/client/frame.rs:518`),
   and a mode's own screens (an inventory, a furnace) need a free cursor. RedCraft keeps its own clock in `sync_gpu` and repaints from
   `window_event`, and its world cannot keep running behind the inventory (the original's does). *Change:* call `frame` every frame and tell
   the mode whether the client is paused; the mode decides whether its world waits. *Done when* RedCraft's `sync_gpu` holds no clock.
3. **A painted surface has no CPU or offscreen path.** The inventory's isometric icons need the same 57 textures the WGSL paints, so
   `games/redcraft/src/icons.rs` (1,084 lines) re-implements `content/shaders/blocks.wgsl` (674 lines) in Rust, and every texture change is
   made twice. *Change:* an engine call that renders a game's `surface` function for given `SurfaceIn` values into an image (one offscreen
   draw, cached), usable for icons and thumbnails. *Done when* RedCraft's icons come from the shader and `icons.rs` keeps only the layout.
4. **The HUD and the window overlay are one texture**, and **`Canvas::rect` blends every pixel in floats.** `set_hud_overlay` and
   `set_window_overlay` both write `live.overlay` (`src/client/mode.rs:367-378`), so a screen over the HUD is composited by the mode into one
   window-sized canvas on every change. Filling that canvas with `Canvas::rect` (`src/ui/mod.rs:133`) costs 38 ms in release and 43 ms in
   the dev profile at 1920 x 1080, opaque or not; a straight copy is 0.8 ms. RedCraft wrote its own fill. *Change:* two layers (HUD below,
   window above) and a row-wise `rect` with an opaque fast path. *Done when* a full-window dim costs under 2 ms in release.
5. **A game binary cannot name its default scene.** `client::run` defaults to `examples/room.json` (`src/client/mod.rs:781`), which an install
   does not ship; RedCraft runs only because the installer's shortcut passes `content/games/redcraft/redcraft.json`. Run with no arguments
   it stops with "failed to load scene examples/room.json", and its `--help` introduces the engine ("AI-first Rust game engine"), not the
   game. *Change:* `client::run_with(factory, defaults)` naming the default scene and the program's name and description. *Done when*
   `redcraft` with no arguments starts RedCraft.
6. **Publishing checks stop short of what the site needs.** `scripts/publish_games.py check` passed and the sync ran, then RedEngineGames'
   Pages deploy failed for every game ("redcraft: needs a wiki page", `site/generate.py:142`) until `site/wiki/redcraft/wiki.json` was
   pushed. Thumbnails are rendered with `red_engine2 frame` (`site/gen_thumbs.py`), which cannot draw a world that a game's mode or layer
   makes, so RedCraft's came from its own `redcraft shot`. *Change:* `publish_games.py check` reports a playable without a wiki page or
   thumbnail in the RedEngineGames checkout, and `gen_thumbs.py` runs a playable's own `binary` with a one-shot headless script. *Done when*
   the next game's first sync cannot fail the site build.
7. **No gamepad navigation for a mode's screens.** The client's pause menu walks with the D-pad; a mode that owns its screens gets no cursor
   or focus help, so RedCraft's inventory, crafting and furnace need a mouse.
8. **A headless `shot` is taken at the next frame** (`src/client/shots.rs:39`), after any later key steps in the same frame: every picture
   script needed a `wait` after each `shot`, and a missing one showed the wrong screen. *Change:* take it at the end of its own step, or make
   `shot` wait one frame itself.
9. **The clarity pass's outlines** draw a dark line along every silhouette against the haze; a block game turns them off (`post.outline: 0`).
   Outlines that fade with the haze would let games keep them.
10. **`validate games/redcraft` reports a false error.** Named, a game crate's folder is checked on the engine's keys only, so its own block
    is "unknown field" (`src/content/validate.rs:92`), while the whole-checkout run rightly leaves the crate to its own test. *Change:* say
    that the block belongs to the crate and which test checks it, instead of failing.
11. **Adding a game crate always runs the full suite.** `affected` escalates any `Cargo.toml` or `Cargo.lock` change (`src/tools/affected.rs:440`),
    and a new workspace member changes both without adding a dependency. *Change:* a diff that only adds a `games/*` member and lock entries
    without a `source` is not a dependency change. *Done when* a PR that adds a game crate runs the game-crates stage, not the full suite.
12. **Windows-only timing flakes on main.** Two of the eleven CI runs on main on 2026-10-19 failed only on the Windows full suite and passed
    on rerun: `relay_server::tests::registration_resolve_and_pairing_caps_are_enforced_under_bounded_local_load` (1aea7f1) and
    `net_sim::a_cruel_link_still_keeps_everyone_connected_and_nobody_teleports` (be8eb03, 1.179 m against a 1.000 m limit).
13. **A layer's moving models are one uniform slot each.** The engine has no instanced pipeline a layer can use, so mobs, drops, particles
    and the hand each take a dynamic-offset slot of the scene pipeline's object uniform (RedCraft keeps 512): enough here; crowds would need
    instancing.

## Not done (scope)

Beds and sleeping; farming; more mobs (sheep, cows, skeletons); villages and structures; the Nether; multiplayer (the engine's server runs
its own simulation, not RedCraft's world; not tried). Not verified: frames per second on a real GPU, a real window's mouse and gamepad,
playing the Windows install.
