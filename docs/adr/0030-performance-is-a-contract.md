# 0030. Performance is a contract: `perf` and `checks.perf`
Status: accepted

## Context
Performance was checked by criterion benches (`benches/sim.rs`, compared by `benches/check.py`) and by budget constants in
`tests/net_budget.rs`. Neither answers the question an author or an AI actually asks: "is *this map* cheap enough for N players?"
BlueEngine's `inspect-performance` / `validate-budget` measure one hard-coded 2-player lab world for 60 ticks and print a mean.

## Decision
`red_engine2 perf scene.json` measures the scene with real players walking in it (swinging and grabbing, which is what wakes props
into full physics bodies) in two in-process, window-free passes: the authoritative `MatchSim` alone, and a real `Server` on loopback
with handshaking clients. It reports microseconds per tick (mean, p50, p95, p99, worst), bytes per client per second, the largest
datagram, and the number of props physics promoted. Judgement is against a budget from the scene's `checks.perf` block (the same
block `verify` runs with every other check) or `--budget file.json`; unset keys keep generous defaults. Failing output carries advice
(promoted props, colliders, interest management).
- **Time only errs upward** (a busy machine makes a tick slower, never faster), so every time figure is the *minimum over several
  windows*. Byte counts are deterministic.
- Budgets are numbers in the map's own file: raising one is a visible diff, and an AI editing a map is told when it made it slower.

## Consequences
`examples/test_lab.json` carries a `checks.perf` block, so `verify` fails if the lab stops holding four walking players inside it.
`tests/net_budget.rs` still guards protocol-level sizes and `tests/alloc_budget.rs` the per-tick allocation count. Not measured:
GPU time, memory, allocations in the CLI (a counting allocator only exists in test binaries), or a real network. Wall-clock budgets are
inherently machine-dependent: set them with headroom (the defaults are one to two orders of magnitude above what the lab needs) and use `perf` before and after a
change on one machine to compare.
