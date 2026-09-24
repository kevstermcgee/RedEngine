# 0014. Sim foundations: fixed tick, scratch buffers, change tracking, static props
Status: accepted

## Context
Groundwork for a headless 1v1 match server (ADR 0010) with a sub-50 ms latency goal. Five increments,
each tested and benchmarked on its own; this record grows with them. Facts that differ from the brief the
work started from: there is **no ECS** (a scene is a `Vec<Object>`), **no networking**, **no Health**
component and **no frying pan** (the melee weapon is the bat, `weapons.rs`). The sim code lives in
`src/sim/`, renderer-free (a test enforces it); nothing here couples to another engine.

## Decision — 1. Fixed timestep
- `sim::clock::TickClock`: 60 Hz (`TICK_RATE_HZ`), frames push real time in, whole ticks come out, the
  remainder is `alpha` for render interpolation; backlog capped at 8 ticks. `player::FIXED_DT` is now an alias.
- Weapon timing is in ticks (`weapons::*_TICKS`, `sim::combat`): swing windup 5 / strike 7 / recover 10,
  switch 20, revolver cooldown 25, dry-fire 18. The animation constants are derived back from the ticks.
  Input events (`attack_queued`, `switch_queued`) are queued and consumed by the next tick; the bat strike
  and the revolver shot originate from `tick_eye()` (tick state), not the render frame's eye.
- **Why 60 Hz:** tick quantisation costs at most 16.7 ms of the 50 ms budget (30 Hz: 33 ms); the shortest
  weapon phase (swing windup, 90 ms) is 5 ticks, within half a tick (8 ms) of its design value
  (`weapons::tests::timings_survive_the_tick_rate`); 120 Hz doubles server CPU per match for no gameplay gain
  (the revolver is hitscan, so bullet time is irrelevant). Click-to-hit is windup + <= 1 tick = 100 ms.

## Determinism hazards in the tick path (flagged, deliberately NOT fixed)
A fixed step does not make the sim deterministic. Found, most serious first:
1. **The tick reads live input**, not a per-tick input struct: `fixed_step_physics` reads `self.keys`,
   `self.camera.yaw/pitch` and the queued click/jump flags directly. A server or a replay cannot feed it.
2. **`eye_height` is blended on render `dt`** (crouch transition) and feeds `tick_eye()`, the origin of every
   shot and swing. Same inputs, different frame rate -> slightly different eye height at fire time.
3. **Platform libm:** `physics::object_body_mat`/`write_pose`/`hold_pose` use `Quat::from_euler`, `to_euler`,
   `atan2` (std f32 trig = the platform's libm: MSVC on the Windows client, glibc on a Debian server, different
   last bits). rapier itself is fine (`enhanced-determinism`, single-threaded, no SIMD/parallel features).
4. **Order-dependent cascade:** `PropWorld::activate` walks a LIFO queue capped at 40 bodies, fed by
   `wake_disturbed`/`props_in`, whose order is rapier's broad-phase query order. A cluster of >40 touching
   props promotes a history-dependent subset.
5. **`HashSet`/`HashMap` with random state** in `physics.rs` (`by_body`, `movable_indices()`) and `re2.rs`
   (`keys`): only ever used for lookup/`contains` today, so not a bug, but any future iteration would be
   nondeterministic run to run. Use `BTreeMap`/`Vec` if they ever get iterated.
6. **Wall-clock catch-up cap:** dropping time beyond 8 ticks is correct for a client but a server must
   instead tick from a schedule, or two runs diverge.
No unseeded RNG exists anywhere in the tick path (the only RNG is the seeded `scatter` map tool).

## Consequences
- Hit timing is identical at any render rate; tests: `sim::clock`, `sim::combat`, `weapons`.
- The determinism list above is the to-do for the lag-compensation / prediction session.
