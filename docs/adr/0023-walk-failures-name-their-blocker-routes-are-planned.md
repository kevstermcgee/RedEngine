# 0023. Walk failures name their blocker; routes are planned, not guessed
Status: accepted

## Context
An AI building the Cheddar game on this engine reported its costliest loop: a `verify` walk check failed with
"stuck on leg 4 toward (-8.4, 3.0); stopped at (-8.4, 3.74)". That says *where* the player stopped, not *what* stopped
them; the AI guessed (a loose crate), moved something, re-ran a 25 s verify, and the next leg failed. `reach` had passed
for the same area, and the AI wrote waypoints by hand. The engine already knew the answer: the collision code that
stopped the player is the same code the tools call.

## Decision
`tools/pathing.rs` (renderer-free, built on the game's own `collide`/`player` functions, never a second physics model):
- `diagnose(world, pos, foot_y, target)` finds the colliders touching the player where a walk gave up, maps each back to the
  scene object that owns it (by volume overlap, so a wall segment resolves to `wall_x.seg2` and its top-level id), says which
  ones lie ahead of the heading, measures the passage across the route against the body width (0.7 m), and asks `reach`
  whether the target is reachable from the stop point. Output: `BLOCKED BY 'crate_a' [prop:crate] ...`, a "passage 0.62 m,
  body needs 0.70 m -> TOO NARROW" line, and the next command to try.
- `plan_route(world, from, to)` is grid A* over the same walkability model as `reach`, with a body margin (tries 0.15, 0.08,
  0 m extra so routes keep off corners), string-pulled by real-physics segment walks, then validated end to end with the
  per-tick `walk`. Waypoints are rounded before validation, so a printed route is exactly what was proven.
- `walk --auto --from --to` prints the waypoints and a paste-ready `checks.walk` entry; `walk --explain out.png` draws the
  route, the stop ring and the blocker boxes with their ids (via a new `Overlay` facility in `plan`).
- `verify`: `checks.walk` entries accept `{"from", "to", "auto": true}` (the route is planned each run and printed so it can be
  pinned); a stuck walk prints the diagnosis and writes `out/verify/<scene>_walk<N>_explain.png`; `--only walk[N]` or any
  check-name text selects one check; every check is timed and slow ones are marked.

## Consequences
A failing walk is one command to understand instead of a guess-and-check loop, and authoring a walk needs no coordinates.
`walk` and `verify` stay the regression net for the physics: they call the same functions as the game (ADR 0003).
Limits: blocker matching is by volume overlap (a collider with no matching item prints `?` and its bounds); planning is 2-D
per floor level (stairs work, ledge jumps do not); a route is one valid path, not the best one. To undo, drop the module:
`walk --path` behaves exactly as before.
