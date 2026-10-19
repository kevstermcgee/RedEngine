# 2026-10-19. A block game's world rides on the game-mode and world-layer seams, widened a little
Status: accepted
Summary: RedCraft (games/redcraft) is a voxel game in its own crate; the engine gains only general seams it needed: a painted surface may read its vertex colour as data (material.z), a layer draws see-through surfaces, a mode reads held keys and the pad, sets the time of day and the haze, saves on close and can hold the keyboard for typing; a clock's haze can begin at a distance (fog_start).

## Context
The owner asked for "a Minecraft clone called RedCraft" on the latest engine (2026-10-19). Such a game needs an endless world of blocks that the
player digs and builds in, textures, light that floods through caves, water, a body that walks on the block grid, an inventory with crafting
screens, a day and night, mobs and a save. The engine has the pieces to host a world it does not know (ADR
2026-10-14-a-game-ships-as-its-own-binary-the-client-is-a-library: `client::GameMode`, `world_layer::WorldLayer`, a scene block defined above the
core), and Frameless proved them. ADR 2026-10-14-a-system-stays-in-the-engine-when-two-games-use-it-or says a system one game uses stays in that
game. So the voxel world, its generator, light, meshing, physics, inventory, crafting, mobs and screens are all `games/redcraft`.

Building it found seven places where the seams stopped short (each measured or forced by a picture, `docs/analysis/2026-10-19-redcraft-feedback.md`):

1. **A painted surface could not vary per vertex.** Objects have no textures; a game paints them with its WGSL `fn surface`, which sees the
   point, the normal and per-draw numbers. A block world wants a texture per face and light baked per corner (the original's smooth light and
   ambient occlusion). With one draw per block type per section a 10-chunk view is about 4,000 draws; the vertex colour, which only tinted,
   was the one per-vertex channel left.
2. **A layer could draw solid and additive glow, not see-through.** Water, ice and glass need the blended pipeline the scene's own
   `opacity < 1` surfaces use.
3. **A mode that moves the player itself could not read the keys.** The client's capsule against `Collider2D` boxes cannot stand on a block
   grid (0.35 m step-up, no sneaking at edges, no swimming or flight), so the mode freezes the client's body (`input_frozen`) and steps its own;
   it had no way to see the held keys (scripts' `hold` writes the same key set) or the gamepad.
4. **No save when the window closes.** Frameless saves on events; a world of edits must be written when the program ends.
5. **The time of day could not be restored.** The clock's start is in the scene; a world saved at dusk opened at dawn.
6. **The haze began at the eye.** A world that ends at a view distance needs fog that starts far out (the original's), or near surfaces turn
   blue-grey (dark spruce forests did, in pictures) while the edge still shows.
7. **`F` toggled fullscreen inside a name field.**

## Decision
Widen the seams, each in the smallest general form; keep everything about blocks in the game.

1. **`SurfaceIn.vertex` and `material.z`.** `surface` receives the vertex colour; when the object's `material.z` is 1 the painted colour is used
   untinted, so the vertex colour is the surface's own data (RedCraft: texture number, sky light, block light). `material.z` was unused; every
   existing object writes 0 there, so their pixels are unchanged (`select` keeps the old product). One draw per section; textures are computed
   from position, so the game can also merge evenly lit faces into large quads.
2. **`WorldLayer::draw_blended(pass, alpha)`**, called by both renderers after the scene's own see-through surfaces and before glows; default
   does nothing.
3. **`ModeContext::key_held(KeyCode)` and `ModeContext::pad()`**: the keys the player (or a script) holds and the last gamepad sample.
4. **`GameMode::closing`**, called from winit's `exiting` and at the end of a headless run.
5. **`ModeContext::time_of_day()` / `set_time_of_day(f)`**: moves the clock's start so the time reads `f` from now; the sky and the sun follow.
6. **`clock.fog_start`** (metres, default 0; typed in `content::scene::Clock`, schema regenerated, `describe` updated), passed to the shaders in
   the global uniform's spare `counts.w`; `scene.wgsl` hazes `max(dist - start, 0)`. **`ModeContext::set_haze(start, density)`** lets a game whose
   view distance the player changes move it.
7. **`GameMode::typing()`**: while true the client leaves `F` to the mode.

`games/redcraft` registers its `redcraft` block and layer like Frameless and is a workspace game crate (`ci.sh game-crates`, `affected`'s
`GAME_CRATES`, packaged as its own binary by `package::game_binaries`).

## Consequences
- Any game with a generated world it edits (a voxel or tile game, a dig site) can paint per-vertex data, draw water, walk its own body, save on
  close, keep its time of day and hide its world's edge. A future second block game is the cue to move the block world itself into the engine.
- `material.z` and `counts.w` now mean something; a later use of either must pick another lane.
- Undo: each seam is additive with a do-nothing default; removing one breaks only `games/redcraft`.
- Publishing RedCraft needs the release tool to ship a game's own binary (RedEngineGames PR #7, still open): plain `re2` cannot read the
  `redcraft` block. Until then it is played with `cargo run -p redcraft --release -- games/redcraft/content/redcraft.json`.
