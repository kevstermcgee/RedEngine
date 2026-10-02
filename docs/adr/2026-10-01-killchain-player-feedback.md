# 2026-10-01. Killchain player feedback
Status: accepted
Summary: Killchain gains sprint controls, clear team spawns, optic reticles, a synchronized knife slash, directional strides, and restrained feedback without footsteps.

## Context
Players reported an inactive F shortcut, overlapping team spawns, no sprint, empty optical sights, awkward knife and walking presentation, crowds of bots at pickups, and cheerful or intrusive audio. Sprint input was absent and Ironworks gave walking and sprinting the same speed. Farthest-spawn scoring ignored teammates. Open optics had no aiming mark and the full knife name was ellipsized by the HUD.

## Decision
- Killchain accepts F and F11 for fullscreen; F still types into focused text fields. Shift and left-stick click hold sprint, with aiming and crouching taking priority. Q retains quick switching. Ironworks sets sprint to 7 m/s and raises the movement cap accordingly.
- Both spawn policies reject occupied positions when another candidate offers 2.5 metres of clearance. Farthest scoring still prioritizes concealment from enemies; crowded maps use the largest available clearance.
- Bots penalize teammate pickup claims, drop consumed pickup goals immediately, and give overlapping bodies priority over their waypoint direction. Claims live only in existing brain state; no protocol change is needed.
- Open optics use a camera-centered illuminated cross, including the Rook. Magnified scopes retain their range marks. Knife HUD text is KNIFE.
- The procedural knife has a tapered, beveled blade and textured grip. Its compact slash peaks at the authoritative melee hit tick. Walking blends stride strength with speed and follows actual movement direction, including strafing and backward movement, with supporting-shoe height correction.
- Killchain selects its own damped, mechanical feedback bank instead of melodic kill, pickup and match cues, and suppresses step playback. Other games retain their existing audio bank.

## Consequences
Changes to spawn policy and bot inputs affect future simulation traces; old recordings containing those choices may differ. Spawn clearance is limited by the authored spawn candidates. Presentation changes do not alter hitboxes or weapon damage. Regression tests cover full-team spawn clearance, competing and consumed pickups, sprint input, the knife hit time, directional strides, reticle pixels and bounded audio. Ironworks must pass lint and navigation with its updated movement tuning.
