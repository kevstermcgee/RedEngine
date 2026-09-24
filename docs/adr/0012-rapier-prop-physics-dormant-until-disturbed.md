# 0012. Loose props run on rapier and stay dormant until disturbed
Status: accepted (the "dormant fixed body" mechanism was replaced by static instances in ADR 0014; the rest stands)

## Context
Both characters can pick things up with E and drop them; a dropped prop must fall, bounce, tumble and
knock smaller things over. That needs real rigid-body physics (rotation, stacking, contacts), which
the player's own 2-D kinematic physics (ADR 0003) does not do and should not grow into. A map has
hundreds of props (the school has ~190), authored to sit exactly where they are.

## Decision
- **rapier3d** does the rigid bodies (`physics::PropWorld`). It is pure CPU with no window/GPU types,
  so a future headless server (ADR 0010) can run it; `enhanced-determinism` is on.
- **What is loose** is a rule, not a list: `classify` picks lift-able props and floor-mounted prefabs
  that are not fixtures; `"movable"` on an object overrides it. Big furniture, walls, floors and stairs
  are fixed colliders (round primitives are solid to props, though not to the player).
- **Dormant until disturbed.** A loose prop is a *fixed* body at its authored pose. It becomes dynamic
  only when the player walks into it, a bat hits it, a moving prop touches it, or it is picked up — and
  then drags along whatever rests on it. This keeps idle maps free and exactly as authored. We tried
  "dynamic but asleep" first: rapier wakes sleeping bodies on the first step and whenever they are moved,
  which shuffled overlapping clutter at load (a globe fell off its table).
- **The player stays out of rapier**: it is a kinematic cylinder that shoves props; its walking still
  uses `player::step_horizontal`, with loose props removed from the static collider lists.
- **Carrying** disables the body and pins the object in front of the player (upright, pulled in by walls);
  drop re-enables it with the player's velocity.

## Consequences
- `lint`/`reach`/`walk` keep treating loose props as solid furniture at the authored spot (a person can
  push a chair aside, so this is conservative, not wrong).
- The player cannot stand on a loose prop (it is shoved instead) and does not get blocked by one.
- Adding a fixture kind = add its name to `FIXTURE_PROPS`; a new pickable thing needs no code.
- A dropped or thrown prop that falls out of the map returns to where it started.
