# 0021. Deterministic traces, replay, and split checksums
Status: accepted

## Context
A multiplayer bug that appears once in a hundred matches is unfixable unless the match can be reproduced without the network, the
renderer or the people. `MatchSim::checksum()` existed but nothing recorded inputs or compared runs.

## Decision
- A **trace** (`sim/trace.rs`) records what determines a match: the header (engine version, tick rate, map hash, seed, spawn group,
  platform), every join / leave / input / server-applied impulse in the order applied (stamped with the tick), the rule events,
  a checksum triple (**players, props, rules**) plus a coarse one every N ticks, and periodic readable state dumps. Floats are
  stored as bits so they round-trip exactly. Every server-side push goes through `MatchSim::apply_impulse` so it is recorded.
- `replay` (`sim/replay.rs`) rebuilds the sim, applies the entries at their ticks, and compares checkpoints: the report names the
  **first divergent tick**, which component differs, and a state diff from the nearest dump (`--dump-every 1` for the exact tick).
  `--against` compares two traces (a desync between machines). `red_server --record` and `sim --trace` produce traces.
- **Determinism across platforms:** simulation maths uses `libm` (glam's `libm` feature and `libm::sincosf`/`atan2f`) instead of the
  platform C library, rapier runs with `enhanced-determinism`, and rule arithmetic is `f64`. Exact checksums are still only
  *guaranteed* on one platform; the **coarse** checksum (floats to millimetres) separates last-bit noise from a real divergence, and
  a trace from another platform yields a note instead of a false alarm.
- CI replays a committed fixture (`tests/fixtures/coin_run.trace.json`, re-record with `RE2_BLESS_TRACE=1`) on Windows and Linux,
  and a test records a **real UDP server session** and replays it bit-exactly.

## Consequences
Any change that alters simulation results (physics constants, movement) makes the fixture diverge — deliberately: re-bless it in
the same commit. Traces grow with match length (about 100 bytes per tick at checkpoint interval 1; the server default is 6).
Seeds are recorded but unused until something random enters the simulation. Encryption/authentication remain a transport concern.
