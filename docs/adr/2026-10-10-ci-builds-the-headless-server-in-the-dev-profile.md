# 2026-10-10. CI builds the headless server in the dev profile
Status: accepted
Summary: The hosted headless job proves red_server and red_bot build and link without gfx from the dev artifacts its tests build anyway; the LTO release build is release.yml's and the container image's.

## Context
ADR 2026-10-03-ci-measures-its-stages-and-stops-paying-for-what-it-does made `headless-build` use the dev profile locally but kept the LTO release build when
`CI=true`, "since that is what a small VPS runs". Measured on hosted CI (run 37962855231, an engine PR): `headless-build` took 99 s of a 498 s headless job,
all of it compiling the engine a third time, in a profile nothing else in the job uses. The job's question is "does the server build and link without the
`gfx` feature" (ADR 0017), and that is a property of the feature set, not of the optimisation level. The release binaries are built elsewhere on every path
that ships them: `release.yml` builds `--release --no-default-features` and smoke-tests the archive from an empty directory, and the CI `docker` job builds
the image (`cargo build --release --no-default-features --bin red_server ...` in the Dockerfile) on every change to `src/`, `crates/`, `Cargo.*` or the
image files, which is every change that could break the release build.

## Decision
`stage_headless_build` builds the dev profile everywhere, `CI=true` included. `RED_CI_RELEASE=1` still asks for the release build.

## Consequences
- The headless job does not compile the engine in a profile it then throws away; the before/after numbers are in docs/analysis/2026-10-10-ci-wall-clock.md.
- A failure that only appears under release + LTO (an optimiser bug, a `debug_assertions`-only code path) is caught by the `docker` job on the same pull
  request, not by the headless job. A pull request that changes none of the image's inputs cannot change the release build either.
- Undo: put `[ "${CI:-}" = "true" ] ||` back in front of the `RED_CI_RELEASE` test in `stage_headless_build`.
