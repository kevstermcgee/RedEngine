# 0032. Provable releases: `package` and `package --verify`
Status: accepted

## Context
A release should answer "is this the build I think it is?" mechanically. BlueEngine's `be2.py package` zips tracked files plus
binaries with a SHA-256 manifest and the commit; it needs Python, cannot verify what it made, and its zip embeds wall-clock times.

## Decision
`red_engine2 package OUT.zip` (Rust, one binary):
- Builds the client and offline tools with the default features and the dedicated server and bot with `--no-default-features` into a
  **separate target directory**, so a graphics-enabled server cannot be packaged by accident; refuses to package a headless binary whose
  bytes contain the graphics stack (`wgpu`, `winit`, `rodio`, `cpal`, `ffmpeg-sidecar`).
- Writes tracked source (`git ls-files`, symlinks refused), the binaries under `bin/`, and `PACKAGE-MANIFEST.json`: SHA-256 and size of
  every file, the commit and its time, whether the tree was dirty and which files, the hash of `Cargo.lock`, the toolchain.
  A dirty tree is refused unless `--allow-dirty`. **New untracked files count as dirt** and are included and listed when it is allowed: a plain
  `git ls-files` leaves them out and produces a package that cannot be rebuilt (found by packaging the change that introduced this tool).
- **Reproducible:** entries sorted, fixed timestamps, no wall clock (the commit's time is used), deterministic deflate: the same
  commit and binaries give the same bytes (tested).
- `package --verify X.zip` re-hashes everything, reports files added, missing or changed, and re-runs the headless check on the
  binaries actually in the zip, so a forged manifest does not help. Exit 1 on any failure.
- New code depends only on `flate2` and `crc32fast`, both already in the tree for PNG.

## Consequences
The manifest says what was built, not that tests passed (it says so): run `scripts/ci.sh` first. Binaries are not signed; the SHA-256 in
the manifest is an integrity check against corruption and swapping *within* a zip, and needs a signature or a trusted channel for the
zip's own hash to protect against a malicious publisher. The rustc version is recorded, but bit-identical binaries across machines
also need the same toolchain and dependency sources (`--locked` is used).
