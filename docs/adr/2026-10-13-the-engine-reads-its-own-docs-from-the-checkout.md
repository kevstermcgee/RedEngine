# 2026-10-13. The engine reads its own docs from the checkout
Status: accepted
Summary: Dev and test builds read SPEC, AGENTS, the reference, the glossary, ADRs, analysis notes and features.json from the checkout at run time; only release-type builds embed copies, as the fallback outside a checkout

## Context
`search` and `describe` were built from documents compiled into the library: `include_str!` of SPEC.md, AGENTS.md, docs/AGENT_REFERENCE.md,
docs/PLAY_2D.md, docs/GLOSSARY.md, docs/adr/README.md and docs/features.json, plus every ADR and analysis note through `build.rs` (ADR
2026-09-28-generated-bookkeeping). So every doc edit, ADR or analysis note recompiled the library and relinked every test binary: one added line in the
glossary cost 86 s for the CLI and 108 s for the library's test binary on the 4-core box, and a docs-only pull request (#116) rebuilt the CLI for 157 s before
`preflight` could run. This is Phase 1 item 1 of ADR 2026-10-13-the-next-phase. `docs/features.json` already preferred the file on disk for planning; the
rest did not.

## Decision
- `tools::own_docs` is the one place the engine's own documents come from. `own_docs::docs()` reads them once per process from the engine checkout:
  `$RE2_SRC`, else the tree the binary was built from, else the current directory or its nearest parent that has `src/lib.rs`, `SPEC.md` and `docs/adr/`
  (so a game project is never taken for one). `search` builds its corpus from them (`corpus_with`), `describe glossary|decisions` and the feature index
  (`features::index`) read them too.
- `build.rs` embeds copies only for release-type builds: Cargo's `PROFILE` is `release` (`--release`, and the `fast` and `bench` profiles, which inherit it),
  or `RED_EMBED_DOCS=1`; it then sets the `red_embed_docs` cfg and generates the ADR and analysis registries as before. A dev or test build compiles no
  copy and emits no `rerun-if-changed` for the folders, so a doc never enters its dependency graph. The copies are the fallback of an installed binary
  run outside any checkout; inside one, even a release binary reads the checkout's files.
- Recipes and 2D patterns stay compiled in: they are content the tests verify, not prose, and they change rarely.

## Consequences
A doc edit, a new ADR or an analysis note now costs 0.3 s of `cargo build` (nothing to compile) instead of 20-108 s, and the next `search` sees it
(`tests/docs_fresh.rs` runs the built binary against a scratch checkout holding an ADR written after the build). Measurements and method:
`docs/analysis/2026-10-13-docs-read-at-runtime.md`. Hosted CI still compiles the workspace crates on every Rust job (its cache holds dependencies only), so
a docs-only pull request still costs CI minutes until item 4, which runs the doc-reading suites from main's compiled test archive (the docs are read at run
time, so main's tests check the pull request's docs); item 2's prebuilts no longer count the docs as build inputs either. A long-lived process (`red_engine2 mcp`) reads the docs once and sees later edits after a restart. The embedded path is compiled
by the `bench` profile in CI ("benches compile") and tested locally with `RED_EMBED_DOCS=1 cargo test --lib own_docs`. To undo: drop the cfg in `build.rs`
so it always embeds, and point `own_docs::docs` at `embedded()`.
