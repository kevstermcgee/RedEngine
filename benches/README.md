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

## Render trend (`render_trend.py`, `red_engine2 render-trend`)

What the live client's renderer draws and how long it takes, for the fixed scenes in `render_scenes.json` (Marcel at morning, dusk and night, Marcel with four
players on one screen, the house). `red_engine2 render-trend` draws them through the same path a player's window uses (`splitshot`'s renderer) and writes one record;
`render_trend.py` keeps the records in `benches/history/render.json` and compares them.

```bash
python3 benches/render_trend.py run --label "what changed"   # draw the scenes here, append to benches/history/render.json
python3 benches/render_trend.py compare                      # the two newest runs ON THE SAME ADAPTER, scene by scene
python3 benches/render_trend.py list                         # every run, with its adapter and whether it is `software` or a `gpu`
```

**Three kinds of number, kept apart.** Triangles, shadow triangles and draw calls are exact and identical on every machine: a change in them is a change in the engine or the
content. Milliseconds belong to **one adapter**: every record names its adapter and says `software` (a rasteriser such as llvmpipe, which is what a dev box or a CI runner
without a usable GPU has, and says little about a player's GPU) or `gpu`; it moves about 2-3% run to run on its own. What a player's GPU actually does is a third thing,
measured only by running the scenes there. `compare` therefore never sets milliseconds beside a run on a different adapter: it pairs the newest run with the previous run on
the same adapter (or says there is none; `--adapter TEXT` picks one). Nothing gates on any of it: the hard guards are the triangle budget tests (`tests/split_render.rs`).

**On a real GPU** (a player's PC, or this one once its GPU is usable: `red_engine2 doctor` names the permission problem when a GPU exists that the user cannot open):

```bash
red_engine2 render-trend --label "RTX 3060, driver 560" --out gpu.json   # no Python needed there; cargo run --release --bin red_engine2 -- render-trend ... in a checkout
python3 benches/render_trend.py add gpu.json                             # back here: files it next to the software runs, never compared with them
```

The `render-trend` workflow runs the software path on pushes to `main` and keeps the result as a downloadable artifact.

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

## Renderer preparation and long-session scaling (no GPU, printed not gated)

```bash
cargo bench --bench render_prep      # old rebuild-everything vs cached object staging: p50/p95/p99 us, allocations, upload bytes per frame
cargo bench --bench settled_world    # step / sync_scene / awake_count / per-client props_to_send after every prop was disturbed and settled
cargo test --test render_prep        # the deterministic guards: byte-equality with the full rebuild after every mutation kind, 0 allocations
```
GPU execution and presentation are not measured by either (needs a real GPU; see the 2026-09-29 note in `benches/history/`).
