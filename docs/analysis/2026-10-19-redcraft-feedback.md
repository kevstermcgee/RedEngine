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

## Engine gaps, ranked (the first seven are closed in this change; see the ADR)

1. **Per-vertex data for a painted surface** (closed: `SurfaceIn.vertex`, `material.z`). Without it a voxel game draws ~4,000 times a frame.
2. **Haze that begins at a distance** (closed: `clock.fog_start`, `ModeContext::set_haze`). Exponential haze from the eye turned dark foliage
   blue-grey at 40 m while the world's edge still showed.
3. **A mode that moves the body reads the keys and the pad** (closed: `key_held`, `pad`); scripts' `hold` reaches it too.
4. **See-through layer surfaces** (closed: `WorldLayer::draw_blended`).
5. **Save on close** (closed: `GameMode::closing`).
6. **Time of day in a save** (closed: `time_of_day`, `set_time_of_day`).
7. **Typing in a mode's text field** (closed: `GameMode::typing`).
8. **Open: the client's own body is not a first-class "frozen" body.** With `input_frozen` the client still steps its body; in a headless run
   (which stays "grabbed") it falls through the world behind any open screen, so RedCraft pins it every frame with `teleport`. A mode that
   owns the body should be able to say so once.
9. **Open: no gamepad navigation for a mode's screens.** The client's pause menu walks with the D-pad; a mode that owns its screens gets no
   cursor or focus help, so RedCraft's inventory needs a mouse.
10. **Open: the clarity pass's outlines** draw a dark line along every silhouette against the haze; a block game turns them off (`post.outline:
    0`). Outlines that fade with the haze would let games keep them.
11. **Open: a headless `shot` is taken at the next frame**, after any later key steps in the same frame (a script needs a `wait` after each shot).
12. **Open: per-frame models are uploaded as uniforms one slot each** (512 slots shared by mobs, drops, particles and the hand): enough here;
    instancing would serve crowds.

## Not done (scope)

Gamepad control of the inventory screens; beds and sleeping; farming; more mobs (sheep, cows, skeletons); villages and structures; the Nether;
multiplayer (the engine's server runs its own simulation, not RedCraft's world). Publishing waits on RedEngineGames PR #7 (a game's own binary).
