# 0011. Two playable characters, a launch menu, and exact melee hit-testing
Status: accepted

## Context
Prop hunt needs a hider that is not a person: a small lab rat, **Cheddar**, next to the human
(who carries the bat). The launch has to ask which one you are. Separately, the bat thunked when it
hit nothing, because "did the swing hit?" tested a ray against one bounding box per top-level
object — a ray through a doorway, or the air over an L-shaped counter, is inside such a box.

## Decision
- **Characters are numbers + parts.** `player::Character` / `BodySpec` hold everything that differs
  (collision radius, eye height, speeds, camera distance, whether there is a bat). Cheddar's ordinary
  speed is one pace (originally a human sprint, 6.5 m/s; now `RAT_SPEED` = 4.0 m/s) and Shift adds nothing; the body also has a height (`band_top` 0.25 m) so overhead geometry above that is run under. Gravity, stepping and floors are shared,
  so the analysis tools still use the same `step_horizontal` (with a radius parameter).
- **Models are `characters::human_parts` / `rat_parts`**: lists of coloured primitives placed from a
  few pose numbers, built once per mesh and re-placed per frame (like props). `humanoid` (redesigned as
  an ordinary person: shirt, skin, hair, jeans, shoes, face) and a new `rat` object type use them;
  the renderer, bounds, hit tests and tools all go through `CharPart`.
- **Launch menu**: `re2` starts in `Phase::Menu`, a tiny 3-D scene (`menu::menu_scene`) drawn by the
  ordinary `LiveRenderer` plus a CPU-painted RGBA overlay (`overlay.rs`, the 5x7 font). Picking builds
  the map renderer. `--as human|rat` / `RE2_CHARACTER` skips it.
- **Melee uses `hit::raycast_shapes`**: the ray is tested against every leaf shape (box, sphere,
  cylinder, cone, capsule, plane) in its own space. No hit, no sound, no flash, no gold crosshair.

## Consequences
- A new character = a `parts` function + a `Character` arm + an `ObjectKind` arm; nothing else.
- The menu backdrop is a real scene, so it costs one extra `LiveRenderer` until a character is chosen.
- Hit shapes are sampled at t=0 like colliders: animated/moving objects are hit where they start.
- Rats have no collider as scene objects; the rat *player* collides as a 0.12 m circle.
