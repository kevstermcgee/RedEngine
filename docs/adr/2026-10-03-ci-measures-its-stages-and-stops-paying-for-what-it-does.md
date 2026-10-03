# 2026-10-03. CI measures its stages and stops paying for what it does not need
Status: accepted
Summary: ci.sh times every stage, type-checks the benches instead of building them, builds the headless server in dev locally, and the edit loop lints.

## Context
A full local run (`scripts/ci.sh`, what `scripts/dev affected` escalates to) took 25 minutes and the usual guess was "too many tests, slow linking". Neither
held up when measured. The suites themselves run in about five minutes in total (lib 8 s, parallel integration suites about 20 s, the real-time network
suites about 2.5 minutes). Replacing the linker with `mold` changed a cold test build from 598 s to 621 s: no gain, so the 50-odd test binaries are not a
link problem and merging them would not have helped. The time was in stages that do work nobody uses. Per-stage timing (added here) on the 4-core box:
`benches` 475 s of a 1167 s run, because `cargo bench --no-run` builds the whole engine optimised only to prove the benchmarks compile; `headless-build`
112 s, an LTO release build of two binaries that `headless-tests` then rebuilds in another profile anyway; `tests` 293 s and `headless-tests` 243 s,
most of which is compiling the engine crate once per feature set and running the network suites twice.

## Decision
- `ci.sh` times every stage and prints a table at the end. It is the first thing to read when CI feels slow.
- `benches` runs `cargo check --benches`. A compile question is a type-check question. `RED_CI_BUILD_BENCHES=1` restores the full build.
- `headless-build` builds the dev profile locally, which shares artifacts with `headless-tests`. The hosted job (`CI=true`) and `RED_CI_RELEASE=1` keep
  the LTO release build, since that is what a small VPS runs.
- The edit loop (`iterate`, `affected --partial`) runs `cargo clippy ... -- -D warnings` where it ran `cargo check`: the same targets, and the failure
  CI's clippy stage would give, in about 20 s instead of at the end of a full run.

## Consequences
- The same local run measured 409 s after the first two changes, against 1167 s before (warm caches both times; the benches stage alone went from
  475 s to 25 s cold). The hosted `test` job loses the same eight minutes.
- A release-only failure of the headless binaries is no longer caught by a local run; the hosted headless job still catches it before merge.
- Not done, and why: `mold` (no measured gain; summed over a core-edit rebuild, 127 links took 66 s with the default linker and 54 s with mold, across 4 cores),
  merging test binaries (a test crate whose source changes rebuilds in about 0.7 s, 45 s for all fifty), running the headless job without the network suites
  (they are the point of that job).
- Measured and rejected: splitting the engine crate. A rebuild after touching a core file costs 126 s of compiler time with one job at a time: the library
  itself 54 s (42 s normal profile + 12 s test profile), two test crates 36 s (`net_processes`, `net_flow`, about 18 s each for 120 to 240 lines of source),
  the `re2` client 11 s, the other ~45 test crates under 1 s each. A split would save part of the 54 s for edits that stay in one crate, and the modules form
  cycles (`schema`, `sim`, `player`, `physics`, `collide`, `net`, `tools` all use each other), so it is a large refactor for a minor gain. What is left to look
  at is why those two test crates compile for 18 s each.
