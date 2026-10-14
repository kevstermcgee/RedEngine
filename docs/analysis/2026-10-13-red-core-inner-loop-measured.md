# A one-line simulation edit through `iterate`, after the core moved (2026-10-13)

Phase 1 item 3 of ADR 2026-10-13-the-next-phase ("done: ... a one-line edit in `sim/` goes through `iterate` in under 20 s warm on the 4-core box,
measured against today's ~50 s"). The ~50 s is from the verified review (docs/analysis/2026-10-13-next-phase-review-verified.md): during #115 a
`rules.rs` edit took fmt 2 + check 10 + unit 11 + red2d 24 s.

## Method

Main at `1148ed0` (red_core moves 1-3 merged: the headless server's whole tree but its binary), on the 4-core development box, one checkout,
its `target/` warm. Each run changes one line of `crates/red_core/src/sim/rules.rs` to a new spelling of the same value, so every run compiles a
real change (`iterate` remembers content it has verified: an edit back to an earlier spelling checks nothing), waits for the 1-minute load average
to fall below 1.5 before the first run, and times `scripts/dev iterate` end to end with nothing else uncommitted:

```
F=crates/red_core/src/sim/rules.rs
for v in "11 - 3" "2 + 6" "7 + 1" "12 - 4"; do
  sed -i "s|^pub const MAX_PLAYER_VARS: usize = .*;|pub const MAX_PLAYER_VARS: usize = $v;|" $F
  S=$(date +%s.%N); scripts/dev iterate > run.txt 2>&1; E=$(date +%s.%N); echo "$v: $(echo "$E - $S" | bc) s"
done; git checkout $F
```

## Results

The move alone (main `1148ed0`, the planner as it was) kept the inner loop where it was: for a file outside `src/`, the partial planner found no
target and fell back to re-checking the whole engine, then planned the 2D crate too.

| run (planner as it was) | load | total | fmt | engine check | core clippy | core tests | red2d clippy + tests |
|---|---|---|---|---|---|---|---|
| `9 - 1` | 1.4 | 45.2 s | 2.1 | 10.3 | 12.0 | 14.9 | 2.9 + 3.1 |
| `3 + 5` | 2.3 | 78.1 s | 3.2 | 50.2 | 3.5 | 13.9 | 2.8 + 4.3 |

With the partial plan of this pull request (a change only in the core checks and tests the core: fmt, `cargo clippy -p red_core --all-features
--all-targets`, `cargo test -p red_core --all-features --lib -- sim::rules`; the engine and the 2D crate, which build on it, are left to `affected`
and the plan says so):

| run | load | total | fmt | core clippy | core tests |
|---|---|---|---|---|---|
| `11 - 3` (the first with this plan: the filtered test build is new) | 1.3 | 50.3 s | 2.1 | 16.0 | 32.1 |
| `2 + 6` | 2.5 | **9.4 s** | 2.1 | 4.4 | 2.9 |
| `7 + 1` | 2.5 | **13.9 s** | 2.1 | 3.6 | 6.5 |
| `12 - 4` | 2.3 | **8.7 s** | 2.1 | 3.6 | 3.0 |

A one-line edit in the simulation goes through `iterate` in 9-14 s warm, against ~50 s before: the item's measure is met. What `iterate` no longer
does for a core-only change is the engine's check and the 2D crate's steps; `scripts/dev affected` (the closure) still runs them, and `iterate`
prints that it did not.

## What is left of item 3

`red_server` is still a binary of the engine crate, so `cargo build --no-default-features --bin red_server` still compiles the engine library.
Everything it uses but one module is in red_core (`src deps --reach bin::red_server`: the engine's `scene_blocks`, through which it registers the
museum block). Building it from red_core alone means a crate of its own, moving the three suites that start it through `CARGO_BIN_EXE_red_server`
(`net_processes`, `server_env`, one `ai_tasks` test), and the generic server registering no game's scene blocks.
