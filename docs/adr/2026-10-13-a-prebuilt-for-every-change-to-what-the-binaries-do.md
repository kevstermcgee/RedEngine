# 2026-10-13. A prebuilt for every change to what the binaries do
Status: accepted
Summary: Every push to main whose source fingerprint has no prebuilt publishes one as a pre-release named by the fingerprint, keeping the newest ten; docs are not part of the fingerprint

## Context
`scripts/prebuilt.py` (ADR 2026-10-09-prebuilt-binary-when-sources-match) installs a release built from exactly a checkout's sources instead of a 2-7 minute cold compile, and
`scripts/dev start`, `scripts/dev red` and `bootstrap.sh` already ask it. But releases were cut only from `v*` tags and none was ever published, so it never
matched: the cold build stayed the Idea Forge backlog's top recurring cost (`cold-worktree-build-and-preflight-rebuild`, 12 runs, 100 minutes). The owner chose
on 2026-10-10 to publish a prebuilt on every main commit that changes a build input; the change was held back because it publishes and deletes releases, and
the owner enabled it on 2026-10-13 (Phase 1 item 2 of ADR 2026-10-13-the-next-phase). Item 1 made the docs runtime reads
(ADR 2026-10-13-the-engine-reads-its-own-docs-from-the-checkout), which is what keeps a docs-only commit from needing a new build.

## Decision
- `release.yml` also runs on every push to `main`. Its first step computes the source fingerprint (`scripts/prebuilt.py fingerprint`) and looks for a
  release asset `MANIFEST-<fingerprint>.json`; when one exists (a docs-only, game-only or script-only commit) every later step is skipped and nothing is built.
  Otherwise it builds, packages and smoke-tests exactly as for a tag, and publishes a **pre-release** `prebuilt-<12 hex of the fingerprint>` targeting the commit,
  then deletes all but the newest `KEEP_PREBUILTS` (10) `prebuilt-*` releases with their tags. Pre-releases are never "latest", so `bootstrap.sh --latest` and a
  tagged `v*` release are unaffected.
- The fingerprint no longer counts the engine's own docs: the includes in `src/tools/own_docs.rs` and the doc folders `build.rs` embeds are the fallback copies
  of a release build, and a binary in a checkout reads the checkout's files. Content that changes behaviour (recipes, patterns, assets) still counts.
- Linux x86_64 first, as `prebuilt.py` supports today; a Windows prebuilt is the next step of the same workflow when a Windows agent needs one.

## Consequences
A fresh clone or worktree of main runs `describe --brief`, `search` and `context` from a download of a few megabytes instead of compiling, whenever main's
sources have been built once. Each engine merge costs one release build on a hosted runner (dependency cache keyed `release`); merges that change only docs,
games or scripts cost one fingerprint step. Ten prebuilts cover a day or two of merges; an older checkout falls back to compiling, and says why. To stop it:
remove `branches: [main]` from `release.yml`'s trigger and delete the `prebuilt-*` releases.
