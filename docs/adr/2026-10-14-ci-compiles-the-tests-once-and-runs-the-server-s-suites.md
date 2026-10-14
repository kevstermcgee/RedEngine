# 2026-10-14. CI compiles the tests once and runs the server's suites headless on pull requests
Status: accepted
Summary: Phase 1 item 4: one job builds cargo-nextest's archive and the doctests, the five Linux shards run from it; a pull request runs the server's suites headless in one job (the whole suite headless on main), builds the container image only when its own files change, and a docs-only one runs the doc suites from main's archive

## Context
ADR 2026-10-13-the-next-phase, item 4: "Build the nextest archive once and fan it out; on PRs run headless as one shard of the net_*, server_env and
sim_replay suites, with the full headless matrix on main; `save-if` keyed on Cargo.lock; Docker on main only; a docs-only PR runs the doc-reading
suites from main's archive." Measured on #131 (a Rust change, every job running; `gh api .../actions/runs/38495669361/jobs`): 57.0 job-minutes. Each
of the five Linux shards compiled the same tests (3 min 2 s to 4 min 6 s of `Finished` each, about 18 minutes together), each of the three headless
shards compiled the headless tests (about 2 min 40 s each), and the container image (4.3 minutes) was built for every pull request that touched the
source. A local archive of the suite (`cargo nextest archive`) is 96 binaries, 1.0 GB compressed, packed in 1.5 minutes. The repository is public, so
the artifact costs nothing; the Actions cache is at its 10 GB limit (15 caches), so a cache saved for nothing evicts one that was needed.

## Decision
- `scripts/ci.sh test-archive` compiles the suite once into a nextest archive and runs the doctests (an archive cannot hold them). The `build-tests`
  job runs it and uploads the archive (one day's retention); the five `test` shards download it and run `scripts/ci.sh tests` with `RED_CI_ARCHIVE`,
  which extracts it into the checkout's own `target/`: the paths the tests were compiled with (`CARGO_BIN_EXE_*`, `CARGO_MANIFEST_DIR`) are the
  runner's, as every job checks out to the same directory. Locally `scripts/ci.sh tests` is unchanged.
- On a pull request the headless job runs `scripts/ci.sh headless-net`: the suites that run the server itself (`net_*`, `server_env`, `sim_replay`),
  without graphics, in one job. On main the whole suite runs headless in three shards, as before. What a pull request no longer runs headless is the
  rest of the suite without graphics, which it runs with them.
- The container image is built on a pull request only when its own files change (Dockerfile, `deploy/`, CI); main builds it for every change it is made
  of. The source it is built from is built and tested by the other jobs.
- `save-if` stays as it is: rust-cache saves only on main and never over an exact key (its key hashes the manifests and `Cargo.lock`), so this item adds no cache.
- A pull request that changes only documentation (`*.md`, `docs/`) runs no compiling job: the `docs` job runs the suites that read the docs
  (`scripts/ci.sh docs`: the ten suites whose source names a doc, and the engine's `tools::` unit tests) against its docs, from the archive main
  compiled from the same code, the newest successful run on main whose commit differs from it only in documentation (main keeps its archive a
  week). Dev and test builds embed no doc (only release builds do, `cfg(red_embed_docs)`), so that archive is exactly this code. When none is left the
  suites are compiled. Main still runs everything after the merge.

## Consequences
Fewer job-minutes per pull request (measured in docs/analysis after the first runs), and one more job on the critical path: the shards wait for the
build. A test that compiled code at run time would compile on a shard (none does: no test runs `cargo`). Undo: the `test` shards with rust-cache and
no `RED_CI_ARCHIVE`, the headless matrix `[1, 2, 3]` on pull requests, the docker job's `docker == 'true'` condition alone, and no `docs_only`.
