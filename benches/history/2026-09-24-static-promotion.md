# Static-prop promotion: before / after (ADR 0014, step 4)

Same benchmark (`benches/sim.rs`), same machine, same bench profile. **Before** = every loose prop was a
fixed rapier body that turned dynamic when disturbed ("dormant"). **After** = a prop is a static instance
(fixed colliders, no body, no entity) and is promoted to a dynamic entity when touched.
Ratio < 1 is faster. Raw before-numbers: `before-static-promotion.json`; current: `../baseline.json`.

| benchmark | before | after | after/before |
|---|---|---|---|
| `build/world/100` | 570.11 us | 542.55 us | 0.95x |
| `build/world/1000` | 6.48 ms | 4.00 ms | 0.62x |
| `build/world/4000` | 28.72 ms | 18.16 ms | 0.63x |
| `frame/sync_scene/100` | 775 ns | 7 ns | 0.01x |
| `frame/sync_scene/1000` | 7.70 us | 7 ns | 0.00x |
| `frame/sync_scene/4000` | 36.56 us | 8 ns | 0.00x |
| `promote/one/100` | 703 ns | 2.37 us | 3.37x |
| `promote/one/1000` | 728 ns | 2.49 us | 3.42x |
| `promote/one/4000` | 880 ns | 3.81 us | 4.33x |
| `tick/16_awake/100` | 124.07 us | 85.51 us | 0.69x |
| `tick/16_awake/1000` | 146.16 us | 116.91 us | 0.80x |
| `tick/16_awake/4000` | 243.24 us | 216.32 us | 0.89x |
| `tick/untouched/100` | 736 ns | 702 ns | 0.95x |
| `tick/untouched/1000` | 1.96 us | 435 ns | 0.22x |
| `tick/untouched/4000` | 8.70 us | 433 ns | 0.05x |

Reading it:
- `tick/untouched/N`, `frame/sync_scene/N`: the win. Untouched props now cost ~nothing per tick or per frame and the
  cost no longer grows with N (rigid bodies in the world: N+1 -> 1, entities: 0). At 4000 props: tick 8.7 us -> 0.43 us,
  per-frame scene sync 36.6 us -> 8 ns.
- `build/world/N`: load time -37% at N >= 1000 (no per-prop rigid body).
- `promote/one/N`: **3.4-4.3x slower per promotion** (0.7-0.9 us -> 2.4-3.8 us): promotion now builds a body, re-inserts
  the colliders and spawns an entity instead of flipping a body type. Paid only by props that are actually touched.
- `tick/16_awake/N`: 0.69-0.89x; dominated by rapier's cost for the awake bodies (the "before" numbers already include the step-2 fix
  that stopped marking every body modified each tick, so this gain is step 4 alone).
- Not measured here: memory. Rigid bodies for a 4000-prop map went from 4001 to 1.
