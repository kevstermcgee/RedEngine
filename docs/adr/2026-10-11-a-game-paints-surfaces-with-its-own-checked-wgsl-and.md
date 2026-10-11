# 2026-10-11. A game paints surfaces with its own checked WGSL and passes numbers to its shaders
Status: accepted
Summary: `museum.art` defines `fn surface(s: SurfaceIn) -> vec4<f32>` composed into the object shader (and in front of the game's sky); an object's uniform carries `surface` numbers and a four-colour palette; `game_param(i)` gives a game's shaders eight numbers its client sets every frame.

## Context
A museum's paintings must be unique and alive: every one a different picture, moving slowly, and the same picture all around the player once they step
inside it. Vertex colours on boxes cannot do that, textures are imported assets (ADR 0008), and the sky shader (ADR 2026-10-11-a-game-draws-its-own-sky-with-a-checked-wgsl-function)
reaches only the background.

## Decision
- `ObjectUniform` gains `surface: vec4` and `palette: array<vec4, 4>` (80 bytes, so the stride stays 256). `scene.wgsl` passes the object-space point
  and normal to the fragment shader and, where `surface.x > 0.5`, asks `surface_of(SurfaceIn)` for a colour and how much of it glows. A game's own
  `fn surface(s: SurfaceIn) -> vec4<f32>` (`museum.art`) replaces that hook (`atmosphere::scene_source`); `SurfaceIn` carries the point, normal, pixel
  size, world point and normal, the numbers and the palette. The same file is composed in front of the game's sky (`atmosphere::background_source`), so
  the sky can draw a painting's world with the painting's own functions.
- The noise helpers and `sky_time`/`sky_seed` moved from `sky.wgsl` to `common.wgsl`: every module has them, so a game's code reads the same helpers
  wherever it runs. `Globals.game` (eight vec4) and `game_param(i)` give a game's shaders numbers its client sets every frame
  (`LiveRenderer::set_game_params`, `Renderer::set_game_params`): the museum says which painting's world surrounds the player and how far it has opened.
- Checked like the sky: `atmosphere::check_art_shader` composes the scene module and runs `shader_check` (naga parse and validate with uniformity,
  no derivatives in loops, HLSL), mapping lines back to the game's file; a sky that calls the art functions is checked with them in front.

## Consequences
- Frameless's 26 painting styles, 10 sculpture materials and its mirror water are one 900-line WGSL file; anything with a `surface` look can use it.
- A painted surface costs what its function costs per pixel, unmeasured on this machine (no GPU here), the same open item as a game's sky.
- Scene objects cannot ask for a surface yet (only the museum layer sets the numbers); `material.surface` is the natural next step when a game wants it.
