# 2026-09-29. Open worlds, peaceful play and a real sea: what an endless beach walk needed from the engine
Status: accepted
Summary: A `world` block makes one axis loop (the seam is invisible: drawn copies, mirrored collision, short-way-round rules and interpolation) and clamps the others; `terrain` objects are walkable heightfields; `sky` and `ocean` draw a sun at infinity and water to the horizon; `hud` and `player.mode` remove the shooter's screen and weapons; F toggles fullscreen from every screen.

## Context
Eventide Shores, an AI-built endless beach stroll, was playable but wrong in six ways its player named: the beach ended, the screen was full of shooter and debug
readouts (`SHELL_1: 1`, `+5 MORE`), the "ocean" was flat boxes with visible edges and the sun, a sphere at a world position, sank *below* the water when you walked to the
end of the map, "dunes" and "rocks" were scaled spheres you walked through, `F` did nothing on most screens, and you could walk off the map. Each is the engine assuming an
arena: a finite closed map, a weapon in hand, a screen-space sky, primitives for everything. The game authored around them for as long as it could; the honest fix is in the engine.

## Decision
- **`world` block** (`expanse.rs`): `wrap {axis, min, max}` makes the world a circle along that axis; `bounds {x, z}` are the walkable limits of the others. The rule lives in
  `PlayerTuning.expanse`, applied at the end of `sim::player::step_player_on_tuned` (clamp, kill the momentum into the limit, then wrap), so the single-player game, the server and
  client prediction agree bit for bit. The map is authored as one period. The seam is hidden by: the renderers drawing the scene three times (`render::wrap_offsets`, per-copy
  frustum culling), static colliders and ground near the seam existing on both sides (`collide::add_collider_images`, `add_ground_images`), `enter`/`exit` rule volumes measured to
  the nearest copy of the player (`RulesEngine::with_wrap`), remote players kept continuous across the seam while interpolating (`RemoteWorld`) and the prediction error and the
  camera interpolation taken the short way (`Expanse::delta`). Not wrapped: loose physics props (they do not cross the seam; keep them away from it), interest cells.
- **`terrain` objects** (`terrain.rs`): a heightfield from `heights`, a grey `heightmap` PNG, or a deterministic `generator` (a smooth cross-section `profile` plus fractal value
  noise, periodic along a wrapped axis) with a height `palette` that becomes vertex colour (`Vertex.color`, white for every primitive). Where a terrain exists it *is* the ground
  (`collide::ground_height_at` uses it instead of the `y = 0` fallback), rapier gets the same triangles, and `lint` reports slopes over 45 degrees where players walk
  (`terrain-slope`). `"on_terrain": true` on an object turns its `y` into a height above the ground under it.
- **`sky` and `ocean`** (`atmosphere.rs`, `shaders/sky.wgsl`, `shaders/ocean.wgsl`, `ocean_pass.rs`): a view-direction sky with a sun at infinity (it can only set behind whatever
  occludes the horizon), and an analytic water plane drawn after the opaque world with frag-depth, whose colour and opacity follow the depth over the scene's first terrain
  (heights in a storage buffer), foam at the waterline, a Fresnel sky reflection and sun glitter, fading into the horizon colour. Scenes without them draw exactly as before.
- **`hud` block and `player.mode = "peaceful"`** (`hud_config.rs`): which on-screen elements are drawn (ping, round, scoreboard, combat, crosshair, events, rule variables or
  a whitelist), with peaceful defaults that show none of the fighting UI. Peaceful also allocates no weapon meshes, ignores attack/reload/switch on the server, and maps the primary click
  to `interact` (an `interact` rule event when there is nothing to pick up).
- **Fullscreen**: `F`/`F11` are handled before any screen sees the key (the launch menu, pause menu, lobby and results ignored `F`; only live play handled it), the mode is read from the
  window, the mouse grab is retaken after the resize that follows, the pause menu has a button and `--fullscreen` starts there.
- **Music**: `"music": false` on a scene starts silent (`N` still toggles); `new-game` writes it (the default for older maps stays on).

## Consequences
- Protocol and replay: unchanged on the wire (the world rules ride the scene, which client and server already share); the `GlobalUniform` gained sky fields, so custom clients that
  build their own uniform must go through `render::build_globals_common` (they already do).
- A wrapped world costs three copies of every mesh in the draw list (culled per copy) and a few duplicated colliders near the seam; nothing when there is no `world.wrap`.
- Left out on purpose: bullets and melee do not hit terrain, the ocean reads only the first terrain, an underwater camera draws nothing for the water, heightmap tiling streams (a
  procedural or tiled infinite terrain with no period) are not attempted: a long period with a seam-free generator covers "endless" for a walk.
