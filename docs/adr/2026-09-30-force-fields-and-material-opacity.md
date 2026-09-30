# 2026-09-30. Force fields and material opacity
Status: accepted
Summary: Scene fields push loose props toward a target speed (currents, belts, wind); material.opacity blends see-through surfaces; underscore variables are declarable.

## Context
Building the Physics Sandbox (a game of eight experiments: fire, a river, a mirror, a light hall, toppling, launching, blasts,
materials) hit the same walls an author of any physical scene would. A river current and a conveyor each took three rules per
floating prop (`enter` sets a variable, `exit` clears it, an `every` rule pushes) and a repeating `impulse` accelerates without bound
(props flew off the belt at 17 m/s). Water, glass and a flame's glow could not be drawn: every surface was opaque, so a mirror pane
hid the room behind it and the river read as blue blocks. And a `_`-prefixed variable, documented as internal, was silently dropped
from `vars` (the extension-key filter ate it) and then rejected as unknown wherever a rule read it.

## Decision
- **`fields`** (`sim::rules::Field`, applied by `MatchSim::apply_fields`): a volume plus a target `velocity [x, z]` and/or `lift`, and
  a `rate`. Loose props inside are pulled toward the target speed each tick through the same unrecorded `shove` a rule `impulse`
  uses, so replays re-derive it. A speed target cannot run away; floor friction leaves a prop a little under it (documented).
  Players are not pushed (a later `who` key). The sandbox went from 87 rules to 51.
- **`material.opacity`**: 0..1, default 1. A second pipeline (`Pipelines::main_alpha`: alpha blending, no depth write, no culling) draws
  blended leaves after the solid ones, far to near (`SceneStaging::blended`, per object); they cast no shadow. Both the live and the
  offline renderer do it. Not done: order-independent blending, refraction, real reflections (a mirror is still a mirrored twin room
  behind a translucent pane).
- `_` variables are declarable (only `x-*`, `$comment` and `notes` are notes inside `vars`).

## Consequences
Currents, belts and wind are one line each; glass and water are possible. Crossing blended objects can show a sorting seam. The
uniform's alpha channel, which the shader ignored, now carries opacity, so an old scene renders unchanged (opacity 1).
