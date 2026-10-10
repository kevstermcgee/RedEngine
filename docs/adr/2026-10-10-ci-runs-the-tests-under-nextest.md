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
- The serial suites become a nextest test group with `max-threads = 1` whose tests take every slot (`threads-required = 'num-test-threads'`): one at a
  time with nothing else running, the same isolation they had at the end of the old run. The configuration is written by `ci.sh` from `docs/features.json`
  into `target/nextest-ci.toml` on every run, so the index stays the single list and there is no committed copy to drift from it.
- CI installs nextest with `taiki-e/install-action` (a prebuilt binary). Without nextest, or with `RED_CI_NO_NEXTEST=1`, the three cargo-test runs are used
  as before, so a machine without it still runs exactly what CI ran last week. `RED_CI_SERIAL_ALL=1` passes `--test-threads 1`.
- `RED_CI_PARTITION=hash:K/N` passes `--partition` (ADR 2026-10-10-ci-shards-the-linux-tests); doctests run on shard 1 only.
- `scripts/dev affected` (the partial, local tiers) keeps its own cargo-test steps; `affected --full` is `scripts/ci.sh` and gets nextest.

## Consequences
- Tried and rejected: letting the serial suites run *alongside* the rest (local 220 s against 263-268 s exclusive). Two `net_e2e` tests failed under that
  load (the 15% loss / 40 ms latency test on hosted CI, the reconnect-by-timeout test locally even with two slots reserved): they time packets and
  physics in real time and were written for an idle machine. Exclusive is the correct setting; sharding (ADR 2026-10-10-ci-shards-the-linux-tests)
  divides their run instead. The loss/latency test also now listens until the barrel has stopped before comparing positions.
- The gain that remains is real: nextest runs tests from different binaries at the same time (`cargo test` ran one binary at a time, so a slow binary held
  everything up) and runs each test in its own process. Numbers: docs/analysis/2026-10-10-ci-wall-clock.md.
- Undo: `RED_CI_NO_NEXTEST=1` in the workflow's `env`, or delete the `use_nextest` branch of `run_tests`.
