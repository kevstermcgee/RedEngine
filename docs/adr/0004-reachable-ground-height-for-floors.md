# 0004. Floors work by "highest reachable surface", not floor bookkeeping
Status: accepted

## Context
Gravity used to target a hardcoded y=0 and wall colliders were gated by a fixed absolute Y band.
Both silently broke the moment a second floor existed.

## Decision
- `ground_height_at(candidates, xz, current_foot_y)` returns the highest walkable surface at `xz`
  whose height is `<= foot_y + GROUND_SNAP_EPS` (~0.35 m). `0.0` is the fallback. A deck that is one
  floor up is simply *unreachable* until the player is near its height — no "which floor am I on".
- Candidates (every `box` top and stairs ramp) are collected once at load.
- Every collider keeps its `min_y`/`max_y`; `colliders_on_floor` re-filters against the player's
  *current* body band (`foot_y+0.05 .. foot_y+2.0`) every tick.
- Stairs are drawn as box treads but walked as a smooth ramp (no washboard). Not an XZ collider.
- One threshold everywhere: a collider blocks only if its top is > `GROUND_SNAP_EPS` above the feet.

## Consequences
- Upstairs walls/rails must be separate objects at the upstairs `y` (they don't inherit from below).
- Stairs are climbed from their bottom end only; approaching the tall end at ground level is
  correctly rejected. Design hallways as dead-ends past the stairs.
- Cheap to test deterministically (`viewer::ground_tests`) — write that test before live-walking.
