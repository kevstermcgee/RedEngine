# 0039. Authored player tuning and deterministic jump pads
Status: accepted
Summary: Scene-authored player tuning, starting weapon and deterministic jump pads

## Context

Games built on the framework shared one hard-coded human movement profile. That kept analysis,
single-player and networking consistent, but forced every game toward the same pace. Arena games
also had to fake launch pads with geometry, which could not produce an intentional vertical route.

## Decision

A scene may author a bounded `player` block for a fixed character, base FOV, walk/sprint speed,
crouch multiplier, jump speed and gravity. A fixed character bypasses the generic picker and is
enforced by the server; omission preserves selection and the established defaults. A scene may also author
rectangular `jump_pads`; touching one at its floor height sets an upward launch velocity.

The tuning and pads are data passed through the shared headless movement function. Offline play,
the authoritative server, bots, prediction and reconciliation all call that function. The graphical
client derives its base FOV from the same scene tuning. Maps may also select any built-in weapon with
`weapons.starting`.

## Consequences

Games can establish distinct movement identities without forking the engine, while prediction stays
aligned with the server. Tools retain conservative default movement unless they are explicitly given
the scene tuning. Jump pads currently provide vertical launch only; directed launch volumes or
momentum/air-control models should be separate, reviewed additions rather than hidden pad behavior.
