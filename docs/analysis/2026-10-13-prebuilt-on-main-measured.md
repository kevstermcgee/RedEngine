# The first prebuilt from main, and a fresh clone with no compile (2026-10-13)

Phase 1 item 2 of ADR 2026-10-13-the-next-phase ("done: a fresh clone of main reaches `describe --brief` with no compile, measured");
decision in ADR 2026-10-13-a-prebuilt-for-every-change-to-what-the-binaries-do.

## Method

After #119 merged (main `7e86eb3`), `release.yml` ran on the push (run 38425886040). Then, on the 4-core development box, with no `CARGO_TARGET_DIR`:
`git clone --depth 1 https://github.com/kevstermcgee/RedEngine fresh`, then in it `scripts/dev start "<a plain game prompt>"` and
`scripts/dev red describe --brief`, each timed with `/usr/bin/time`, their logs searched for `Compiling` or `build red_engine2`.

## Results

| step | result |
|---|---|
| `release.yml` on main | fingerprint `a7c3e49c9bcf` had no prebuilt, so it built, smoke-tested and published the pre-release `prebuilt-a7c3e49c9bcf` in 773 s (full set 28 MB, headless CLI 6.7 MB) |
| `git clone --depth 1` | 1.6 s |
| `scripts/dev start "..."` | 3.9 s: "fetched release prebuilt-a7c3e49c9bcf (full), checksum verified, built from exactly this checkout's sources: no compile" |
| `scripts/dev red describe --brief` | 0.09 s, nothing compiled; `target/` holds only the 69 MB prebuilt |
| before (no prebuilt could match) | a cold build: 185 s in a seeded worktree, 385 s in a worktree's first build, 2-7 minutes by the backlog's runs |

The release binary reads the clone's own docs: an ADR written into the clone after the download was found by its `search`, and the fingerprint stayed
`a7c3e49c9bcf`. Docs-only, game-only and script-only commits on main reuse the prebuilt (the workflow's first step finds the manifest and builds nothing).
