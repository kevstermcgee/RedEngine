# 2026-10-10. CI shards the Linux tests
Status: accepted
Summary: The Linux suite and the headless suite are split across runners with nextest --partition; clippy/benches and the public-API games are their own jobs, so nothing waits serially.

## Context
After the Windows pull-request leg became build + Direct3D (ADR 2026-10-10-windows-ci-on-pull-requests-is-build-and-direct3d) and the suite moved to nextest
(ADR 2026-10-10-ci-runs-the-tests-under-nextest), the Linux `test` job set the pull request's wall-clock: 477 s, in which `lockfiles`, `clippy`, `tests`,
`benches`, `external-client` and `killchain` ran one after another on one runner (run 38080453489: 4 + 17 + 310 + 9 + 48 + 41 s, plus about 45 s of
checkout, cache and apt). The headless job (374 s) ran its build, clippy and tests the same way. None of these stages needs another's output. Standard runners
are free on a public repository, so the cost of more jobs is job-minutes nobody pays for, and the goal is wall-clock per pull request.

## Decision
- `test` is a matrix of shards. Each runs `scripts/ci.sh tests` with `RED_CI_PARTITION=hash:K/N`, which `run_tests` passes to `cargo nextest run --partition`
  (doctests on shard 1). The shards share one cache (`shared-key: linux-tests`), saved by shard 1 on main. The number of shards is the measured choice in
  docs/analysis/2026-10-10-ci-wall-clock.md.
- `lint` runs `lockfiles clippy benches`; `api` runs `external-client killchain`; `games` runs the generated-games suites for a `games_only` pull request.
  Each has its own cache.
- `headless-linux` is two shards: shard 1 also runs the tree, build and clippy checks (seconds), and both run their part of `headless-tests`.
- `scripts/ci.sh` stays the single list of what runs: the workflow only chooses which stages each runner calls and sets the partition.

## Consequences
- Every shard compiles the test binaries itself (the dependencies come from the cache); sharding pays that compile N times in parallel to divide the run.
  So it helps only while the run is a large part of a shard; the analysis note has the measured split.
- `hash` partitioning balances the number of tests, not their duration: a shard that draws `net_e2e` and `net_sim` is the slowest. If one shard is
  consistently long, move to more shards or to `count:` before anything cleverer.
- A failure names its shard; `RED_CI_PARTITION=hash:K/N scripts/ci.sh tests` reproduces exactly that shard locally.
- Undo: a single-entry matrix (`shard: [1]`) gives one unsharded job with the same steps.
