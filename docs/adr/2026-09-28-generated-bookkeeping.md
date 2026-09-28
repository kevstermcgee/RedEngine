# 2026-09-28. Generated bookkeeping: preflight, dated ADR ids, derived facts
Status: accepted
Summary: Every registry CI bounced on is generated or derived, and `red_engine2 preflight` finds the rest in a second and prints the exact edit.

## Context
Building Trigger Happy on the engine (ADRs 0050-0057), every feature bounced off CI on repository bookkeeping, and each bounce cost a full test cycle to discover. An ADR lived in four places
(the file, its README row, the hand-kept `ADRS` list in `search.rs`, the count in CLAUDE.md); `docs/features.json` had to own every file; `tests/headless_boundary.rs` kept a hand list of the
graphics-only modules; `describe` had a byte budget that nothing reported until the test ran; a protocol bump meant editing SPEC and two docs by hand. Parallel AI branches also collided on ADR
numbers (0043-0046 were already taken on other branches) and on the shared index lines.

## Decision
- **ADRs are files.** `build.rs` embeds `docs/adr/` and `docs/analysis/` (no Rust list). `docs/adr/README.md` holds a table generated from each file's `Summary:` line (`adr index --write`),
  merged with git's union driver so two branches that each add a row keep both. A new ADR gets a *dated* id (`adr new`), which cannot collide with the one another branch is writing; 0001-0057
  keep their numbers.
- **Facts the code knows are derived.** The CLAUDE.md block holds binaries, features and the wire protocol; inline `<!--fact:protocol-->8<!--/fact-->` values in SPEC and AGENT_REFERENCE follow
  `PROTOCOL_VERSION`. Counts (suites, maps, ADRs) left the block: they changed with every feature and made every branch conflict on one line; `status` prints them.
- **The graphics-only set is read, not listed:** `#[cfg(feature = "gfx")]` modules of `src/lib.rs` plus the `required-features` binaries of `Cargo.toml` (`tools::preflight::gfx_only_paths`).
- **`red_engine2 preflight [--fix]`** runs all of that plus the `describe` byte budgets, hand-written test counts, stale claims, missing doc paths, commands absent from the tool table and rustfmt.
  The checks are functions in `tools::preflight` and the test suites call the same ones, so preflight and CI cannot disagree. About a second, nothing compiled. Each problem prints the exact
  edit; the mechanical ones (index, feature ownership guessed from the neighbouring files or the imports of a test, facts, tool-table rows, fmt) are applied by `--fix`.
- The overview shows each command's first sentence capped at 96 characters on a word boundary (it used to cut at "e.g." and run to 200).

## Consequences
- A new ADR is one command; a new source file needs at most the one-line ownership edit `--fix` makes; forgetting either is a finding of a fraction of a second instead of a CI round trip.
- `docs/features.json` and the tool table can still conflict between parallel branches (the same array, the same end of a table): re-run `preflight --fix` after the merge.
- Dated ids are longer to type than numbers: refer to an ADR by its full stem (`2026-09-28-generated-bookkeeping`).
- To undo: delete `build.rs` and restore the two lists; nothing else depends on them.
