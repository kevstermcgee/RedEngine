# 2026-10-09. A prebuilt binary replaces the cold compile when the sources match
Status: accepted
Summary: start, scripts/dev red and bootstrap.sh install the release built from exactly the checkout's sources (a git-tree fingerprint, a verified checksum) instead of compiling, and fall back to the build with the reason said.

## Context
On a fresh checkout the first useful command (`describe --brief`) was a cold build: 437 s for the CLI alone on the four-core dev box (about 14 minutes for the headless test build on a two-core
machine), longer than a 120 s tool timeout, and every Idea Forge feedback note lists the cold build or the seeding as a cost. `release.yml` already builds the Linux binaries (the full set and the
headless CLI) and `bootstrap.sh` installed the newest release into `~/.local`, but nothing tied a release to a checkout: a release records no revision, so the resolver refused to prefer one
over the checkout, and an old binary running against a newer checkout hides every change since (the failure `scripts/red_resolve.py` exists to prevent).

## Decision
A binary built from exactly the checkout's sources is the binary a build there would produce, so it may be used instead of building. "Exactly" is a fingerprint (`scripts/prebuilt.py`):
the git tree entries of everything that goes into a binary (Cargo.toml, Cargo.lock, build.rs, `src/`, `crates/`, `assets/`, the folders build.rs embeds, and every file an `include_str!`/`include_bytes!` names,
found by reading the sources so a new embedded file needs no list) hashed together. Documents that are not embedded, tests, scripts and CI files are not inputs.
- `package_release.sh` (so `release.yml`) writes `MANIFEST.json` and `MANIFEST-<12 hex>.json` beside the archives: the fingerprint, the commit, each archive's kind (full or headless), binaries and sha256; both are
  listed in `SHA256SUMS`. The fingerprint is in the file name so one releases-API request finds the match without downloading anything.
- `scripts/dev red` (and everything that needs the CLI: `iterate`, `affected`, `context`, `preflight`), `scripts/dev start` and `scripts/bootstrap.sh` run in a checkout call `prebuilt.py ensure`: no usable
  binary built here, a clean tree, Linux x86_64, a release whose manifest has this fingerprint: download the archive, check its sha256 against both `SHA256SUMS` and the manifest, unpack flat into
  `<target dir>/prebuilt/<fingerprint>/` (full set when libasound is installed, so `re2` comes with the CLI; the headless CLI otherwise, said out loud) and record it in `prebuilt.json`.
- `scripts/red_resolve.py` selects an installed prebuilt only while the checkout still has its fingerprint (no mtime test: the fingerprint is the proof) and prefers a binary built here. An edit to any input,
  committed or not, ends the match: the next command builds.
- Every other outcome installs nothing and says why in one line: no release yet, none built from these sources (the newest with a manifest is named), uncommitted edits (named), no network, a checksum that
  differs (refused), another platform. `start` then names the build as the next action, as before; `bootstrap.sh` exits 3 with the build command and `--latest` for the old behaviour. A miss is remembered for an
  hour (a failed request for two minutes) so repeated commands do not ask GitHub again; `--refresh` and `bootstrap.sh` always ask. `RED_NO_FETCH=1` / `--no-fetch` keeps everything offline.
- `start` is no longer read-only in the strict sense: it may make one request and one download into the target directory. It still never compiles, never runs verification, and writes nothing in the repository.

## Consequences
A fresh clone whose sources a release covers runs `describe --brief` in a fraction of a second after one small download, with no toolchain at all. Releases are cut from tags, and the fingerprint covers embedded docs
and ADRs, so a checkout of an arbitrary `main` commit matches only a release cut from sources that are identical: in practice the feature is dormant until releases are published often. Publishing a prebuilt
for every `main` commit that changes an input (a `release.yml` trigger on pushes, pruned to the newest few) would make it routine; that costs a release build per such merge and is the owner's decision,
recorded in STATUS.md, not made here. Undo: delete the `ensure` call in `cli()` of `scripts/dev` and in `launchpad.py`; releases without manifests keep working with the old `bootstrap.sh --latest`.
