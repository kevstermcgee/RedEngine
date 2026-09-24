# 0007. Docs are generated or test-checked; the source index is on demand
Status: accepted

## Context
Documentation that an AI trusts must not lie. Hand-written tables of commands, lint codes and
object types go stale silently.

## Decision
- `describe` builds its command list from the real clap definition; its per-type examples are parsed
  and its lint-code table is checked against `lint.rs` by tests (`src/tools/describe.rs`). Adding an
  object type, lint code or CLI command makes `cargo test` name the doc to update.
- `search` indexes `SPEC.md`, `AGENTS.md`, `docs/` (glossary + ADRs), the catalogue, lint codes,
  recipes, commands and public Rust symbols in one ranked BM25-ish list with synonyms.
- `src map|find|show|refs|deps` scans the tree on demand: no database, nothing to go stale.
- **Embeddings were deliberately skipped**: lexical search + synonyms is enough at this size, needs no
  model/vector store, and is deterministic and testable.
- Every module opens with a `//!` purpose line; `src map` prints it. Public items get `///` docs;
  `src coverage` lists the ones missing one.

## Consequences
- Compile-time `include_str!` of docs means a docs edit needs a rebuild to be searchable.
- New doc files must be registered in `search.rs` (`ADRS`); a test fails if one is missing.
