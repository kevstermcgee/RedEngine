# 0043. A reusable client layer for games that are not first-person
Status: accepted

## Context
Games belong in their own crate that depends on `red_engine2` (ADR 0024), but the public surface stopped at scene loading and schema
types. The renderer only accepted an `FpsCamera` and always carried the bat/firearm meshes and crosshair; window/GPU setup, frame
acquisition, input tracking and HUD repainting were private to `re2`'s `App`. A Gauntlet round (`.gauntlet/feedback.md`, RedEngine-01)
saw an agent build a parallel client for a non-first-person game (44 tool calls, 7 engine source files opened). Its other main finding,
that the standard client ignored rule state, had already been fixed (ADR 0035/0036) and was re-checked, not rebuilt.

## Decision
`src/app/` is the client layer, split by what needs graphics:
- Headless: `ViewCamera` (eye/target/up perspective, `top_down`, `screen_ray`, `pick_ground`, `world_to_screen`; `FpsCamera` moved here
  as one policy that produces a `ViewCamera`), `LocalSession` (one player in the authoritative `MatchSim`: strict load, fixed-tick
  `advance`, interpolated feet, rules state, hidden objects, outcome; a presentation scene separate from the simulated one), and
  `HudState`/`RecentEvent` (rule HUD data laid out by the audited `ui` kit).
- `gfx`: `WindowGpu` (device, swapchain, resize, acquire/present), `InputState` (held/pressed keys, clicks with positions, wheel,
  focus-loss clearing), `HudPainter` (repaint on change only), `Offscreen` (world + HUD to RGBA/PNG for presentation checks), and
  `shell::run` with a four-method `ClientGame` trait (scene, update, camera; hidden/HUD/title optional).
- `LiveRenderer::render_view(camera: &ViewCamera, fps: Option<FpsLayers>)` draws the world; weapons, viewmodel and crosshair are the
  optional `FpsLayers`, and `LiveRenderer::world` never uploads weapon meshes. `render_ex` is now a thin first-person wrapper.
- `re2` uses `WindowGpu`, `FpsCamera::view` through `render_view`, `HudState`, `RecentEvent` and `HudPainter`.
- `examples/external/topdown_switch` is a separate crate (own `[workspace]`, engine by path) proving it: top-down camera with turn and
  zoom, WASD relative to the view, click-to-move via `pick_ground`, a rule-driven gate, and an outcome banner, with no engine code
  copied. CI's `external-client` stage builds, lints and tests it; `tests/custom_client.rs` covers the headless half.

Gameplay stays in `sim`: the session exposes `MatchSim`, it does not wrap or duplicate rules.

## Consequences
A custom game is: load a `LocalSession`, turn input into `PlayerInput`, return a `ViewCamera` and a HUD. Not provided yet: application
actions that feed rules (a `use` key) need a declared, recorded, networked event; an online custom client still has to drive
`net::session::NetSession` itself; the renderer uploads a scene's objects once (move/hide, do not add); perspective cameras only.
`LocalSession` is single-player. Undo: `app` is additive; `render_ex` and `FpsCamera` keep their old signatures.
