# CI wall-clock per pull request: measured before and after (2026-10-10)

Goal: the time a pull request waits for CI (the slowest job), not job-minutes (standard runners are free on this public repository).
Six changes, each measured on hosted runners on PR #97 before the next one went in; local numbers are from the 4-core development box.

## The result in one table

Wall-clock of an engine pull request (every job runs), warm caches, hosted runners. "Slowest" is what the pull request waits for.

| step | change | slowest job | wall-clock |
|---|---|---|---|
| before | (run 38076883035, A1 already in; the A1 change itself is 40 s of a job that was not the slowest) | `full build (windows-latest)` | **1386 s (23.1 min)** |
| A1 | headless server built in the dev profile, not release + LTO | (Windows) | headless job 498 -> 451 s; `headless-build` 99 -> 58 s |
| A2 | Windows on a PR = build + Direct3D; full Windows suite on main | `full build (ubuntu-24.04)` | **651 s** (Windows 1386 -> 219 s warm) |
| A3 | one `cargo nextest run`; serial suites a max-threads=1 group | `full build (ubuntu-24.04)` | **477 s** (tests stage 434 -> 310 s; headless 457 -> 374 s) |
| A4 | Linux tests in 3 shards, headless tests in 2, clippy/benches and the API games their own jobs | `headless (shard 1/2)` | **320 s** (test shards 202/226/268 s) |
| A4' + A5 | headless build+clippy its own job, 4 test shards; container image caches its dependency layer | see below | see below |

The 43-run baseline before any change (median per job, runs with the full matrix): Windows 19.0 min, Linux 9.3, headless 8.1, container 3.8, Python tools
1.0 / 2.6 (Linux / Windows). The pull request waited for Windows in every one of them.

## What each step found

**A1, headless dev build.** `headless-build` compiled the engine with release + LTO only to prove the server links without `gfx`; the dev artifacts answer
that. 99 s -> 58 s (run 38076883035). Small, because the job was not on the critical path; it matters again once Windows is out of the way.

**A2, Windows.** The Windows leg was 348 s of test compile, 278 s of parallel tests (two software-rendering suites: `split_render` 143 s,
`procgen_render` 46 s), 180 s of serial network suites and 141 s of the external client (run 37962855231). On a PR it now runs `scripts/ci.sh direct3d`:
`shader_validation` plus `shadow_render`, which creates a real Direct3D device on WARP and compiles the renderer's pipelines through Microsoft's compiler
(the one thing only Windows can say), and builds and links every binary on the way. Cold cache (a new job name has no cache from main yet): 711 s;
warm: **219 s** (engine compile 2 m 38 s). Finding: `shader_validation` alone proves nothing on Windows that Linux does not, since naga is the same
everywhere; the Direct3D authority comes from a test that creates a device.

**A3, nextest.** Local, warm build, 4 cores, two samples each: the old three `cargo test` runs took 426 and 479 s (about 370 s of it running tests), one
`cargo nextest run` took **220 and 222 s**. Hosted: the Linux tests stage 434 -> 310 s, the headless tests 350 -> 287 s. The gain is the serial suites
(about 170 s, one at a time) no longer waiting for everything else, and slow binaries no longer holding up the others. The 18 serial suites passed in
every run while sharing the machine (4 local nextest runs, 6 hosted jobs so far); no flake seen yet, but this is a small sample.

**A4, shards.** With 3 shards each shard still compiles every test binary (about 120 s from cached dependencies), so sharding divides only the run:
test shards 202 / 226 / 268 s against 477 s for the one Linux job; `hash` partitioning gave the shards 175 / 196 / 236 s of tests. The headless job's
shard 1 also ran the headless build and clippy and became the slowest job (320 s): moved to its own job in A4'.

**A5, container cache.** (numbers below)

**A6, fewer test binaries: not done, and why.** After touching `src/lib.rs`, a warm `cargo test --no-run` took 105 s locally (`--timings`): the
library 35 s on the critical path (15.8 s normal + 19.2 s test profile), more than 70 test crates at about 1 s each, and four crates at 22-24 s each
(`interactions`, `gate_meadow`, `net_join_flow`, `sim_replay`, 200-470 lines, no unusual macros) that start together at 75 s and are the last 24 s of
the build. Grouping 77 binaries into ten saves at most the per-binary link, about 70 CPU-seconds, about 17 s of wall-clock on 4 cores, while it would
break `scripts/dev affected`'s mapping of suites to `tests/<name>.rs` and the `serial_suites` binary names. ADR 2026-10-03 measured the same for
rebuilds. Not worth it; the next thing to look at is why those four crates take 23 s each.

## Method (to repeat)

- Per-job wall-clock: `gh run list --workflow CI` + `gh run view <id> --json jobs`, `completedAt - startedAt` of each job; a "representative PR" is a
  run where the full native matrix ran.
- Per-stage: `gh api repos/<owner>/<repo>/actions/runs/<id>/jobs` for step times, and each job log's `-- stage times` table (`scripts/ci.sh` prints
  one per invocation), plus the log timestamps of `Finished \`test\` profile` and the `-- ` group markers for compile vs run inside the tests stage.
- Cold vs warm: `Swatinem/rust-cache` keys on the job (or `shared-key`) and saves on main only, so a renamed or new job runs cold on its PR. To measure
  the steady state before merging, a commit marked "TEMPORARY, measurement only" let the new jobs save on this PR, then the run was re-run
  (`gh run rerun`); those commits are reverted before merge.
- Local: alternating runs (old, nextest, nextest, old) of `scripts/ci.sh tests` on a warm build, `RED_CI_NO_NEXTEST=1` for the old path, load average
  checked before each.
- Limits: hosted runner speed varies 20-30% between runs on identical sources, so single-run differences under about a minute are noise; most steps here
  have one or two hosted samples.
