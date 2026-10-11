# 2026-10-11. A game draws its own sky with a checked WGSL function
Status: accepted
Summary: `sky.shader` names a .wgsl file defining `fn sky(dir, px) -> vec3<f32>` that the background pass calls instead of the engine's dome, with the engine's noise helpers, scene time and a per-launch seed; it is validated with naga (parse, uniformity, no derivatives in loops, HLSL) when the scene is; `player.still` makes a viewer who only looks.

## Context
Wormhole (examples/wormhole) is a fall through an endless tunnel of light: nothing to walk on, no objects, every pixel a function of where you look and
when. The engine had a sky dome (gradient, sun, a clock's stars) and nothing a game could add to it: the only way to draw such a world was a crate of
its own on `red_engine2::app` with raw wgpu, which the release pipeline cannot ship (it packages `re2`/`re2d` plus data). Games are data (ADR 0001), so
the question was how to let data reach the GPU without letting it break a player's machine.

## Decision
- `"sky": {"shader": "sky/x.wgsl", "seed": "launch"}`: the file defines `fn sky(dir: vec3<f32>, px: f32) -> vec3<f32>` (linear colour along the unit world
  direction; `px` is the angle a pixel spans, for antialiasing). `atmosphere::background_source` puts it in place of one hook function in
  `background.wgsl` (`sky_of`), after the engine's declarations, so it may call `sky_time()`, `sky_seed()`, `hash13`, `hash33`, `vnoise`, `fbm` and read
  `globals`. Both renderers build their background pipeline from that source (`gpu::create_pipelines(.., sky)`); nothing else changes. The ocean still
  reflects the engine's sky.
- **Checked like the engine's own shaders.** `tests/shader_validation.rs`'s checker moved into the library as `shader_check` (naga parse + validate with
  uniformity analysis, the rule against derivative lookups inside loops, HLSL translation), and scene validation runs it on the composed module, mapping
  naga's lines back to the game's file. So a sky Direct3D would refuse fails `validate` on Linux, and the test also checks every `.wgsl` in examples/ and
  recipes/. naga became a normal dependency (it was a dev-dependency; wgpu already builds it).
- **Seed.** `sky.seed` is a number (default 0) or `"launch"`; the live client draws one number per launch (`Scene::fix_launch_seeds`, printed as `seed N`), so
  a sky that builds its variety from the seed is new each time; offline frames use 0 and repeat. It rides in the unused `globals.sky.w`: no uniform layout change.
- **`player.still: true`**: a player who only looks around (walk, sprint, jump, gravity and acceleration are 0; naming them as well is an error). The
  `leak` lint does not apply to them (there is nowhere to walk off).

## Consequences
- Wormhole is one scene, one 330-line shader and five scores, shipped by the same release pipeline as every other game.
- A game shader can be slow: nothing bounds its cost per pixel, and real-GPU frame time is not measured on this machine (`render-trend` needs a GPU). A
  sky that is too expensive shows up as a low frame rate, not an error.
- naga's checks are not Microsoft's compiler: an FXC-only error in a game sky would still reach a player on Windows (the same limit the engine's own
  shaders have, where hosted Windows CI is authoritative).
- Undo: drop `sky.shader`/`sky.seed` from `atmosphere::parse_sky_shader` and the `sky` argument of `create_pipelines`; `still` is `parse_player` in schema.rs.
