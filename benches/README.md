# Benchmarks

```bash
cargo bench --bench sim            # criterion; writes target/criterion (bench profile: thin LTO)
python benches/check.py            # PASS/FAIL against benches/baseline.json (exit 1 on regression)
python benches/check.py --bless    # accept the current numbers as the new baseline (commit baseline.json)
cargo bench --bench sim -- --baseline <name>   # criterion's own % change vs a saved run (--save-baseline <name>)
```

| id | measures |
|---|---|
| `tick/untouched/N` | one `PropWorld::step` with N loose props nobody touched (the common Prop Hunt case) |
| `tick/16_awake/N` | one tick with N props, 16 kept awake by an impulse each tick |
| `frame/sync_scene/N` | the per-frame write of prop poses into the scene |
| `promote/one/N` | promoting ONE static prop to a dynamic entity, in a world of N |
| `build/world/N` | building the physics world (load time) |
| `snapshot/{full,delta_quiet,delta_10pct}/N` | encoding N dynamic entities (`sim::snapshot`, placeholder layout) |

**Two kinds of signal.** Wall-clock numbers are machine-specific and noisy, so `check.py` uses a 35% tolerance and
a 40 ns noise floor; re-bless when you change machine. The *deterministic* signals never flake and live in tests:
`tests/alloc_budget.rs` (heap allocations per tick, counted by a global allocator) and the body/entity-count
assertions in `physics::tests` / `tests/prop_physics.rs`. Prefer adding those when you can.

`benches/history/` keeps dated before/after tables for significant changes (e.g. static-prop promotion).
Add a bench: write it in `benches/sim.rs` with a stable id, run, `--bless`.

## Idea-to-game flows (`flow_bench.py`)

Not a micro-benchmark: how long each step of making a whole game takes, and how much output an agent would read doing it. A flow
(`benches/flows/*.json`) is a list of CLI steps (scaffold, build, check, prove the rules, measure the server, try the network); the runner
records wall time, exit code and output bytes per step, plus `approx_tokens` (bytes / 4, an estimate: nothing here calls a model, so a run
costs no API tokens).

```bash
python3 benches/flow_bench.py run benches/flows/hello_game.json --label "what changed"   # appends to benches/history/flows.json
python3 benches/flow_bench.py compare hello_game                                          # last two runs, step by step
python3 benches/flow_bench.py list
```

Build the CLI first (`--profile fast|release` selects `target/fast|release`). The load average at the start is recorded; compare runs taken under
similar load, and treat differences under about 20% as noise. A failed flow is printed but never recorded.
