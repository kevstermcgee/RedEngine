# 2026-10-10. CI runs the tests under nextest
Status: accepted
Summary: scripts/ci.sh runs the suite with one `cargo nextest run` (serial suites as a max-threads=1 test group generated from docs/features.json) plus `cargo test --doc`, instead of three cargo-test runs.

## Context
`run_tests` in `scripts/ci.sh` ran the suite as three `cargo test` invocations: the parallel group (lib, bins and every suite not listed under `serial_suites`),
the doctests, and the 18 serial suites with `RUST_TEST_THREADS=1`. The serial group runs after the parallel one, so for its whole length (about 170 s on
hosted Linux, 180 s on Windows: `net_e2e` 36 s, `net_sim` 32 s, `net_processes`, `net_quic`, `net_interest` about 16 s each, ...) three of the runner's four
cores are idle, and inside the parallel group `cargo test` runs one test *binary* at a time, so a slow binary (`split_render`, `procgen_render`) holds the
run up while the others wait. The serial suites must not run two at a time (real-time UDP, spawned servers, fixed timing), but nothing requires the rest of
the suite to wait for them.

## Decision
- When `cargo nextest` is installed, `run_tests` is one `cargo nextest run --lib --bins --tests` plus `cargo test --doc` (nextest does not run doctests).
- The serial suites become a nextest test group with `max-threads = 1`. The configuration is written by `ci.sh` from `docs/features.json` into
  `target/nextest-ci.toml` on every run, so the index stays the single list and there is no committed copy to drift from it.
- CI installs nextest with `taiki-e/install-action` (a prebuilt binary). Without nextest, or with `RED_CI_NO_NEXTEST=1`, the three cargo-test runs are used
  as before, so a machine without it still runs exactly what CI ran last week. `RED_CI_SERIAL_ALL=1` passes `--test-threads 1`.
- `RED_CI_PARTITION=hash:K/N` passes `--partition` (ADR 2026-10-10-ci-shards-the-linux-tests); doctests run on shard 1 only.
- `scripts/dev affected` (the partial, local tiers) keeps its own cargo-test steps; `affected --full` is `scripts/ci.sh` and gets nextest.

## Consequences
- The serial suites run one at a time *alongside* the rest of the suite instead of after it, and each test runs in its own process (a test that leaked
  global state into another now cannot). Measured numbers, and the flakiness check of the serial suites under load: docs/analysis/2026-10-10-ci-wall-clock.md.
- A serial suite now shares the machine with up to three other tests while it runs. Its timing budgets were written for an otherwise idle machine; if one
  turns flaky, give that group `threads-required` (reserve cores) in `nextest_config` rather than going back to running it after everything else.
- Undo: `RED_CI_NO_NEXTEST=1` in the workflow's `env`, or delete the `use_nextest` branch of `run_tests`.
