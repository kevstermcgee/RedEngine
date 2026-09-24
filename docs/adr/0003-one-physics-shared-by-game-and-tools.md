# 0003. The game and the analysis tools run the same physics functions
Status: accepted

## Context
`lint`/`reach`/`walk` must answer "can the player actually get there?". A separate approximation
(a geometry heuristic) drifts from the game and gives false confidence.

## Decision
`re2` and the tools call the *same* functions: `player::step_horizontal`, `player::vertical_step`,
`viewer::ground_height_at`, `viewer::colliders_on_floor`, `viewer::resolve_collision`, with the
constants in `player.rs`. `reach`/`lint` flood-fill a grid by calling them; `walk` replays a route
tick by tick. `tests/house_walk.rs` walks every room of the house map as the regression net.

## Consequences
- "lint is clean and the walk test passes" == "playable". Bugs found this way were real engine bugs
  (no staircase could climb onto a slab; a 2.0 m door header blocked the doorway; stairs had no side
  colliders) — fixed once, for both consumers.
- **Any physics rule change goes in these shared functions**, never in `re2.rs` only.
- Structural wart: the shared physics lives in `viewer.rs` (the wgpu renderer) and `player.rs`
  imports it. A headless server cannot use it without dragging in rendering. See ADR 0010.
