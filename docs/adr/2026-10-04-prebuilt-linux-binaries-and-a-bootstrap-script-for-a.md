# 2026-10-04. Prebuilt Linux binaries and a bootstrap script for a fresh machine
Status: accepted
Summary: A tag publishes Linux x86_64 archives (the full set and a headless CLI that needs only libc) with checksums, and scripts/bootstrap.sh installs them with no Rust toolchain and runs doctor.

## Context
An author's first report on a small game said it plainly: on a fresh machine they needed Rust, native dependencies and an engine build before the self-describing CLI could be used at all (feedback item 5). Windows players already download prebuilt games from RedEngineGames, but nothing published the authoring tools for Linux.

## Decision
- `.github/workflows/release.yml`: a tag `v*` (or a manual run, which only keeps the archives as workflow artifacts) builds on the oldest Ubuntu runner, so the binaries run on anything newer, and publishes two archives with a `SHA256SUMS`: the **full set** (`red_engine2` with rendering, `re2`, `red_server`, `red_bot`, `red_relay`; needs `libasound2` and, for rendering, a GPU or Mesa lavapipe) and a **headless CLI** (lint, reach, walk, verify, sim, plan, patch; links nothing but libc).
- `scripts/package_release.sh` makes the archives and `scripts/smoke_release.sh` unpacks each into an empty directory and does what a newcomer would (`describe`, `doctor`, `recipe --new`, `lint`, `verify`, and `frame` for the full set), so a release cannot ship a CLI that only works inside a checkout. It runs in the release job before anything is published.
- `scripts/bootstrap.sh` (`curl ... | sh`): picks the latest release (or `--tag`), chooses the headless archive when `libasound` is missing (and says how to get the full one), verifies the checksum and refuses a mismatch, installs under `~/.local/share/red_engine2/<tag>`, links the binaries into `~/.local/bin`, and runs `red_engine2 doctor` (which already says what is missing and how to fix it). `tests/bootstrap_script.rs` runs it against a release made on the spot (`RED_RELEASE_BASE=file://`).

## Consequences
Easier: a Linux author goes from nothing to `describe --brief` in one command and a download. Harder: the first tagged release has to be cut (nothing is published until a tag exists: push `v0.1.0` or run the workflow by hand to see the archives), the archives are x86_64 Linux only, and the full set depends on the system's `libasound`. Not covered: macOS, Windows authoring (the Windows download of a game already ships its own engine), and pinning a game project to a downloaded release instead of a source checkout (`scripts/red` still builds the pinned engine).
