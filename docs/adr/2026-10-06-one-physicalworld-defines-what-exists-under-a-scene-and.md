# 2026-10-06. One PhysicalWorld defines what exists under a scene and a collision state
Status: accepted
Summary: Every consumer of the static world (client, server and LocalSession, online prediction and bots, and all analysis tools) asks collide::PhysicalWorld, so a rule that opens a gate changes exactly that object and the generated terrain stays.

## Context
After the Marcel fixes the generated world was part of the ground in the client, `MatchSim` and `LocalSession`, but four places still assembled the static world from per-object groups by hand: the client's rebuild, `MatchSim`, `ClientWorld` (online prediction and bots, which started from an empty ground) and `MapWorld` (every analysis tool, which dropped the scene's terrain as soon as a rule or phase opened any gate). Each fix had to be repeated in each place, and the next consumer would forget it again.

## Decision
`collide::PhysicalWorld` is the one answer to "given this scene and this collision state, what physical world exists": the scene's own ground (`scene_ground`: the generated world and the loop) plus the colliders and standable surfaces of every top-level object whose collision is on. `set_collision_disabled` rebuilds from cached parts. The client, `MatchSim`, `ClientWorld` and `MapWorld` all use it, and `tests/ground_consistency.rs` fails if any other module calls the group-assembling functions.

## Consequences
A rule that opens a gate removes exactly that object's colliders and surfaces; hills and trees stay, in play and in analysis, in both states, which a pen scene with a gate, a lever rule and a named phase proves across all five consumers. Online clients now predict on the generated ground. Harder: `PhysicalWorld` holds per-object groups in memory (as `MatchSim` already did) and `ClientWorld` keeps a copy of the current result for its public fields. To undo: inline the groups again; the tests show what each consumer must agree on.
