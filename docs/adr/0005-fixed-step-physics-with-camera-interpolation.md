# 0005. Fixed 1/60 s player physics, interpolated camera
Status: accepted

## Context
Variable-`dt` movement tunnelled through walls on frame hitches and made the tools' replay differ
from the game.

## Decision
`App::fixed_step_physics` (`src/bin/re2.rs`) advances movement, collision and gravity in `FIXED_DT`
steps from an accumulator (capped at 8 steps so a long stall resumes rather than replaying minutes).
The rendered camera interpolates between the last two physics states (`alpha = accumulator/FIXED_DT`).
Only camera-critical physics is fixed-step; walk-cycle pose, crouch/FOV blends, swing and flash
timers stay on render `dt`.

## Consequences
- Deterministic per-tick movement: `walk` and `reach` replay it exactly (ADR 0003).
- A networked simulation (ADR 0010) can reuse the same tick as its authoritative step.
- Extending interpolation to full third-person joint poses was judged not worth the cost.
