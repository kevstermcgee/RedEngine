# Interned ids and ledger slots, measured (2026-10-15)

Review follow-up item 3 = roadmap item 1 of docs/analysis/2026-10-14-game-systems-milestone.md ("indexed runtime data"). Decision: ADR
2026-10-15-a-game-system-s-tick-runs-on-interned-ids-and-ledger. Every number below is from ubuntu-pc (Intel N97, 4 cores), the same machine
as the milestone's, load average about 1 (nothing else building), "before" being main at f23b763 and "after" this branch.

## Acceptance, item by item

| criterion (roadmap item 1) | result |
|---|---|
| scale probe N = 1000, p95 at most a third of 1.73 ms (0.577 ms) | **met:** 17.8-18.4 us (three runs); before, the same day: 1.56-1.59 ms |
| same systems hash as the string version over 3000 ticks of `workshop.json` and `workshop_plus.json` | **met:** `tests/golden.rs` pins the hash every 500 ticks and an FNV of the save text at 3000, pinned on main before the change (first commit); also a 100-machine workshop |
| zero allocations per steady-state tick (`alloc_budget` pattern) | **met after warm-up:** 0.0000 per tick at N = 10, 100, 1000 (600 ticks after a 3000-tick warm-up); before: 200.5, 1958.5, 19538.9 |
| `describe systems` unchanged | **met:** byte-identical output |
| content files unchanged | **met:** no file under `games/workshop/content/` changed |

Also identical: `workshop_server cli sim <scene> --inspect <target> --json` for 12 targets (systems, both machines, both stores, the mine, the
sink, a recipe, a resource, a machine kind, a machine's input store, an expansion machine) in both scenes, 190 KB of output.

## Scale probe

`cargo test -p workshop_server --release --test scale -- --ignored --nocapture` (N machines, half furnaces, half presses, one shared storage,
through `MatchSim::tick_once`; 120 warm-up ticks, 600 timed). Medians of three runs:

| N | p50 before | p50 after | p95 before | p95 after | p99 before | p99 after | max before | max after |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 10 | 9.2 us | 0.49 us | 12.9 us | 0.76 us | 19.2 us | 1.2 us | 32 us | 9.4 us |
| 100 | 124 us | 1.8 us | 140 us | 2.2 us | 193 us | 4.7 us | 300 us | 8.3 us |
| 1000 | 1.42 ms | 16.6 us | 1.58 ms | 18.3 us | 2.11 ms | 37.2 us | 3.30 ms | 61.8 us |

Units shipped after the run (3700 at N = 1000) are the same before and after, as are save bytes (261,214) and snapshot part bytes (8,028).
Save and load times are unchanged (about 12 ms at N = 1000): they serialize JSON and were not the target.

## Allocations per tick

`cargo test -p workshop_server --release --test alloc_budget -- --nocapture`: a counting global allocator; the same scene without its
`workshop` block is the engine's share (0 at every N); Workshop's share is the difference and its budget is 0.

| N | before | after (warm-up 600) | after (warm-up 3000, the test) |
|---:|---:|---:|---:|
| 10 | 200.54 | 0.0000 | 0.0000 |
| 100 | 1958.52 | 0.0017 (1 in 600 ticks) | 0.0000 |
| 1000 | 19538.90 | 0.0017 (1 in 600 ticks) | 0.0000 |

The one allocation between ticks 600 and 1200 is a scratch buffer reaching its high-water mark (gone with a longer warm-up), not per-tick work.

## What changed where the time went

The milestone's `perf` at N = 1000: `memcmp` 21.6% (string-keyed map lookups), `malloc`/`free` about 27%, `Ledger::count`/`free` 13%.
After: no id is cloned or formatted in the tick (an `Id` is a `Copy` handle), the machines' stores and resources are slots resolved once,
a transaction applies in place with a reused undo log and no working copy, and a refused transaction (a full output, every tick while it is
full) is a two-word `Fail` instead of a formatted `Diagnostic`. `perf` was not re-run after; the totals above are the evidence.

## Not covered

Other platforms: every number is Linux; the golden test runs on Windows only where CI runs the game crates there (pushes to main). A server that interns text from peers (`Id::find` is the rule;
no such path exists today). Save/load cost (JSON, unchanged).
