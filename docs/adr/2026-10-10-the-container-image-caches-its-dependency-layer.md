# 2026-10-10. The container image caches its dependency layer
Status: accepted
Summary: The Dockerfile compiles dependencies in a cargo-chef layer that changes only with Cargo.toml/Cargo.lock, and the CI docker job keeps BuildKit layers in the GitHub Actions cache.

## Context
The `docker` job ran `docker build` with no cache of any kind: `COPY . .` came before the only `cargo build`, so every change to any file invalidated the
one layer that compiles everything, and every run compiled the release dependencies from nothing (200-233 s per job on hosted runners; median 3.8 min over
39 runs). Three ways were on the table: BuildKit cache mounts (a mounted `target/`, but GitHub's hosted runners start empty, so a mount survives only with
an extra cache-dance action), building the image from the binary the test job built (couples two jobs and ships a dev-profile binary, or adds a release
build to the test job), and cargo-chef (a layer per dependency set).

## Decision
- The Dockerfile has a `chef` stage (the official `rust:1-slim-bookworm` plus `cargo install cargo-chef --locked --version 0.1.78`), a `planner` stage
  (`cargo chef prepare` writes `recipe.json`, which depends only on the manifests and the lock file) and a `build` stage that runs
  `cargo chef cook` (the dependencies) before `COPY . .` and the real `cargo build`. Same flags, same binaries, same runtime stage.
- The CI job builds with `docker/setup-buildx-action` + `docker/build-push-action` (`load: true`), `cache-from`/`cache-to: type=gha,mode=max,scope=red-server`.
  `mode=max` is needed because the cooked layer is in an intermediate stage, not in the final image.
- A plain `docker build -t red-server .` still works on any machine (BuildKit is the default builder); it simply uses that machine's own layer cache.

## Consequences
- A change that leaves `Cargo.toml`/`Cargo.lock` alone reuses the dependency layer and compiles only the workspace crates; the measured numbers are in
  docs/analysis/2026-10-10-ci-wall-clock.md. A dependency change pays the full build once, then is cached again.
- The image is built by the same commands as before, so the release-build coverage the headless job now relies on
  (ADR 2026-10-10-ci-builds-the-headless-server-in-the-dev-profile) is unchanged.
- The GitHub Actions cache is shared by the repository (10 GB); `mode=max` layers for one scope are a few hundred MB. Evicted layers just mean one cold build.
- Undo: delete the `chef`/`planner` stages and the `cook` line, and use `docker build -t red-server .` in the job.
