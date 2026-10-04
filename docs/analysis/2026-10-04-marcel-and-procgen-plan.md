# Marcel and procedural generation: survey and plan (2026-10-04)

The brief (from the user): add procedural generation to the engine, tested by a game, **Marcel**: an infinite, procedurally generated world of fields and forests you can run around in forever; each day fades
to night (a gradual sunrise, sunset, sunrise, over and over); breathtaking stars; soothing sunrises and sunsets; a young boy as the character; a heartwarming, almost euphoric feel; the main menu says how many
days you have spent in the game; a variety of flowers and trees based on real plants; nature sounds that suit the time of day (birdsong, wind, leaves ...); unique emotional ambient music built with the new audio
tools, toggleable in settings, on by default.

## What the engine has
- **Sky**: a dome with horizon/zenith colours and ONE fixed sun disc (`sky` block). No time of day, no moon, no stars. A single directional light (with a follow-the-camera shadow map, `shadow_follow`), otherwise point lights.
- **Ground**: a `terrain` heightfield (generator: a cross-section profile plus fractal noise), at most 1 M samples, finite; a `world.wrap` can loop it along one axis. No infinite world, no streaming. `collide::ground_height_at`
  reads the stored terrain, so ground must be stored data.
- **Drawing**: one draw call per object (`draw_indexed(.., 0..1)`), no instancing, no frustum culling per object, meshes built once at start with a small pool for runtime additions. A forest of thousands of plants as objects is out of the question.
- **Plants**: a handful of indoor/garden props and prefabs (`assets/outdoor.json`); nothing botanical.
- **Character**: `humanoid` (height, build, hair, skin, clothes) and the fixed player bodies (human, rat, ...). No child.
- **Persistence**: `settings.rs` saves music/sfx toggles per game and nothing else. **Menus/cards**: the scene `ui` block's start card (PR #10) can say `{var}`, but vars reset every run.
- **Audio** (this session's work): voices, scores with seeded generative walks, delay/reverb, loop-safe LUFS-targeted rendering, `audio report|picture|check`. Not yet: nature sounds (needs modulation and event scheduling), a scene `audio` block,
  a runtime that plays scene music/ambience and changes it with a clock.
- **Verification** works here: `frame` renders offscreen (software GPU), `audio picture/report` measure sound, headless scripts drive the client. So looks and sounds can be checked in this environment, not by eye on a monitor.

## Decisions I am taking (say if you want any different)
- **A game is data.** Marcel is a scene plus a few JSON files; the engine grows scene blocks, not a one-off client: `clock` (time of day), `procgen` (the infinite world and its flora), a `boy` character, persistent vars, and the
  `audio` block. Anything Marcel needs, the next game gets for free.
- **Art style**: soft, stylised, low-poly with vertex colours and gentle lighting (cheap enough for an infinite forest, and it reads as warm and storybook-like); the "euphoria" comes from colour grading, bloom, haze, light
  shafts and motion (wind, fireflies, drifting pollen), not from polygon count.
- **A day lasts 12 real minutes** (a `clock.day_secs` you can change), dawn and dusk lingering, night shorter than day.
- **Infinite = chunks.** Everything is a pure function of (seed, chunk coordinate): height, biome, plant placement. Chunks near the player are generated, baked into **one merged mesh per chunk** (so a forest is a few draw calls),
  culled by distance, and dropped when far. Same seed, same world, on every machine.
- **A "day" in the menu** counts in-game sunrises you have lived through, summed across sessions and saved.

## Phases (each a reviewed, CI-green PR, and each leaves something you can see or hear)
1. **Time of day and sky.** A `clock` block drives the sun and moon, a sky colour model (night, dawn, day, dusk) with soft sunsets, a procedural **star field** (depth layers, colour, twinkle, a milky-way band, the odd shooting star),
   and the light/ambient/fog following the sun. `frame --time` renders any moment. This is the "breathtaking sky" and the soothing sunsets, and it is checkable with pictures.
2. **Procgen core** (pure, headless): deterministic hashing and noise, chunk coordinates, an analytic ground function, biomes (meadow, wildflower field, forest, grove), per-chunk species placement with spacing, and a CLI to
   draw a top-down map of any region, so a world can be looked at without rendering it.
3. **Flora.** Parametric meshes for real species (trees: pedunculate oak, silver birch, Norway spruce, weeping willow, Scots pine, cherry; flowers and grasses: oxeye daisy, common poppy, cornflower, lavender, bluebell, dandelion,
   buttercup, red clover, foxglove, wild grasses), as data with their Latin names, plus a contact-sheet command to look at all of them.
4. **Streaming world in the client.** Chunk meshes baked and culled, ground and tree-trunk collision from the generator, wind sway in the vertex shader, haze to the horizon.
5. **The boy and the feel.** A child character (smaller body and eye height, simple warm clothes), colour grading, bloom, light shafts, fireflies and pollen at the right times.
6. **Sound.** Nature voices (birdsong by species and hour, wind, leaves, crickets, an owl, water), the scene `audio` block, a runtime that crossfades ambience layers by the clock and plays emotional ambient scores (dawn, day, dusk, night),
   all measured with `audio check`. Music toggle in settings, on by default.
7. **Marcel.** The game project: main menu with the day count and the persistent save, settings, polish, and a frame-time budget test.

## Risks, said plainly
- Rendering performance cannot be measured on a real GPU here; I will budget (draw calls, triangles, chunk build time) and test those numbers, but the "smooth" claim needs your machine to confirm.
- "Breathtaking" and "euphoric" are judgments; I can make the physics of the sky right (colour, glow, twinkle) and verify with pictures, but taste will need your eyes and a few rounds.
