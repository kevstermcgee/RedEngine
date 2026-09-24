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
5. **`HashSet` with random state** in `physics.rs` (`movable_indices()`, a load-time set) and `re2.rs`
   (`keys`): only ever used for lookup/`contains` today, so not a bug, but any future iteration would be
   nondeterministic run to run. Use `BTreeSet`/`Vec` if they ever get iterated. (`by_body` no longer exists:
   step 4 maps colliders to props through `user_data`.)
6. **Wall-clock catch-up cap:** dropping time beyond 8 ticks is correct for a client but a server must
   instead tick from a schedule, or two runs diverge.
No unseeded RNG exists anywhere in the tick path (the only RNG is the seeded `scatter` map tool).

## Decision — 2. Tick-scoped scratch buffers
`sim::scratch::ScratchVec` (`take`/`give_back`, reset not freed, counts growth). `PropWorld`'s per-tick lists
(wake queue, promotion order, moving bodies, changed slots) use it. Measured with a counting global allocator
(`tests/alloc_budget.rs`, asserts budgets): **idle map 5.03 -> 0.03 heap allocations per tick** (the win was
a real bug: `step` took `&mut` on every prop body each tick, making rapier re-process and allocate for all of
them); a tick with awake props 39 -> ~16-21 (all inside rapier's CCD solver and EPA contact code, only while
props are awake; not poolable without patching rapier; a speed-gated CCD experiment did not reduce it and was
reverted). Collision contacts: the engine never collects them (rapier keeps its own); **network delta events**
have no consumer yet, but `snapshot::encode_delta` appends into a caller-owned buffer.

## Decision — 3. Change tracking
`sim::change`: `Generation`, `GenClock`, `ChangeCursor`, `Tracked<T>`, `TrackedColumn<T>` over
`sim::components::{Transform, Health}`. `changed_since(gen)` is O(1) for a column (it remembers its newest
change) and for a slot. `ChangeCursor::catch_up` records `now` *and closes the generation*: without that a write
stamped in the same generation as a read is invisible next time (found and fixed before use). Its first real
consumers are the render sync and `snapshot`; nothing is wired to a network (none exists).

## Decision — 4. Static-prop promotion
`sim::statics`, `sim::entities`, `PropWorld`. A loose prop is a **static instance**: fixed colliders at its
authored pose (solid, ray-hittable), no rigid body, no entity, nothing to replicate. It is **promoted** the first
time it is touched (player, bat, bullet, moving prop, pick-up; plus what rests on it, capped at 40): a dynamic
body takes over its colliders and it gets a tracked-`Transform` entity. Colliders carry `prop id + 1` in
`user_data` (this also removed the `HashMap<RigidBodyHandle, usize>` hazard). Promotion is one-way.
Measured (`benches/history/2026-09-24-static-promotion.md`), 4000 props: untouched tick **8.7 -> 0.43 us**, per-frame
scene sync **36.6 us -> 8 ns**, world build **-37%**, rigid bodies 4001 -> 1; the price is promotion itself,
**~3.4-4.3x slower** (0.7-0.9 -> 2.4-3.8 us), paid only by touched props. Known limitation: a ray query in the
same tick a prop is promoted cannot see it until the next `step` (rapier's query BVH refreshes on step).

## Decision — 5. Benchmarks
`benches/sim.rs` (criterion), `benches/check.py` (PASS/FAIL against `benches/baseline.json`, 35% tolerance,
`--bless`), `benches/README.md`. Deterministic guards (allocation counts, body/entity counts) are tests; wall-clock
numbers are machine-specific and not gated in CI. `sim::snapshot` is a **placeholder layout, not a protocol**: a
4096-entity full snapshot encodes in 34.7 us, a 10% delta in 4.2 us, a quiet world in 4 ns.

## Consequences
- Hit timing is identical at any render rate; tests: `sim::clock`, `sim::combat`, `weapons`.
- The determinism list above is the to-do for the lag-compensation / prediction session.
