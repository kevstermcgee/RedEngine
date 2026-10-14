# CI after compiling the tests once (2026-10-14)

Phase 1 item 4 of ADR 2026-10-13-the-next-phase ("done: before/after in docs/analysis with a method, against #115's 59.0 job-minutes and ~27
job-minutes of compilation"); the decision is ADR 2026-10-14-ci-compiles-the-tests-once-and-runs-the-server-s-suites.

## Method

A run's job-minutes are the sum over its jobs that ran (skipped ones excluded) of `completed_at - started_at`, from the Actions API; its wall clock is
the first job's start to the last job's end:

```
gh api "repos/kevstermcgee/RedEngine/actions/runs/<run id>/jobs?per_page=50" \
  --jq '.jobs[] | select(.conclusion != "skipped") | [.name, .started_at, .completed_at] | @tsv'
```

A job's compilation is the `Finished ... target(s) in` lines of its log (`gh api repos/kevstermcgee/RedEngine/actions/jobs/<job id>/logs`).

Before: run 38495669361, #131 (red_core move 3), a Rust change with every Rust job running. After: run 38503426545, #134 (this change). #134 also
changes CI, so the Python tools ran (4.0 minutes, which #131 did not) and the container image was built (4.0 minutes, which a pull request now builds
only when its own files change): "like for like" leaves both out.

## Results

| | #131 | #134 |
|---|---|---|
| job-minutes, all jobs that ran | 57.0 | 40.8 |
| job-minutes, like for like | 57.0 (52.7 without its image) | **32.8** |
| Linux tests: compile + run | 5 shards x 4.6-5.8 = 26.5 | build 3.9 + 5 shards x 1.6-2.2 = 13.3 |
| compilation in the Linux test jobs | 5 x 3.0-4.1 = 18.3 | 1 x 2.9 (the build job; packing the 1 GB archive 11 s, each shard extracts it in ~23 s) |
| headless | 3 shards x 4.5-5.0 + build/clippy 1.5 = 15.8 | server suites 4.9 (compile 1.6, 92 tests in 3.0) + build/clippy 1.4 = 6.3 |
| container image | 4.3 (every source change) | 0 on a pull request that does not change its files |
| wall clock | 6.0 min | 6.3 min |

A Rust pull request costs about 33 job-minutes instead of 57 (against #115's 59.0), with the same wall clock: the build job is on the critical path
(3.9 minutes, then a shard of about 2), where each shard used to compile for itself. Everything still runs on main: the whole headless suite in three
shards and the image for every change.

## Not yet

A pull request that changes only documentation still runs the test jobs (#133, an ADR and its index line: 47 job-minutes). The docs are read at run time
since item 1, so main's compiled archive can check a pull request's docs without compiling: the next step.
