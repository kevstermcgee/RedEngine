# 2026-10-11. Compile-time includes are changes
Status: accepted
Summary: affected and iterate plan a source file that include_str!s or include_bytes!s a changed file as changed too, so a guard test that reads another file through an include is never skipped.

## Context
The agent-loop trace (docs/analysis/agent-loop-trace.md, task c, sink 2) added a lint code to `src/tools/lint.rs` and ran the ladder. `scripts/dev affected`
answered **"verified (closure scope)"** in 202 s, yet CI would have failed: the unit test that keeps `describe lint` complete
(`tools::describe::tests::lint_table_covers_every_code_the_linter_can_emit`) lives in `src/tools/describe.rs`, reads `lint.rs` through
`include_str!("lint.rs")`, and belongs to another feature (`self_description`) that is neither an owner of `lint.rs` nor built on its feature. The planner
maps files to features (`docs/features.json`, ADR 0033) and never saw the dependency. ADR 0007's promise ("adding a lint code makes `cargo test` name the
doc to update") held only for the full suite.

A compile-time include is the one dependency the feature index cannot express and the compiler does create: when the included file changes, the including
file's compiled code changes with it. The tree has 17 files with literal-path includes (shaders, recipes, `SPEC.md`, the kart roster, the lint guard).

## Decision
- `affected::includers(root, changed)` scans the Rust sources (`src`, `crates`, `games`, `tests`, `benches`, `examples`) for `include_str!("..")` /
  `include_bytes!("..")`, resolves each literal path against the including file, and returns the includers of changed files. `run_affected` adds them to
  the changed set with a note (`src/tools/describe.rs compiles in src/tools/lint.rs ...: planned as changed too`), for every tier: `iterate`, `--quick`,
  closure, and the green stamp (which keys on the changed files' contents).
- No hand-maintained list: a new include is picked up the day it is written. Paths built with `concat!`/`env!` are not resolved (none exist today).

## Consequences
Re-running the trace's case (only `lint.rs` changed, the doc row missing):

| tier | before | after |
|---|---|---|
| `iterate` | unit filter `tools::lint`: passes | `tools::describe tools::lint`: **fails at the unit step** with "lint codes not described in describe::LINT_CODES: [\"ui-no-rules\"]" |
| `affected --quick` | passes | fails at the unit step |
| `affected` | **"verified" in 202 s** (100 CPU-s, suites already built); CI would fail | **fails in 67 s** (216 CPU-s) at the unit step |
| `affected`, change complete (doc row added) | — | verified in 208 s (426 CPU-s, suites rebuilt); 18 suites instead of 16 (`cli_envelope`, `features_index` from `self_description`) |

- A false green costs a CI round trip (~5 min hosted plus re-loading the change); the plan for `lint.rs` grows by two suites and the `describe` unit tests.
- Changing a shader, a recipe JSON or `SPEC.md` now also schedules the includer's tests and owners; that is what the compiler already does.
- Undo: drop the `includers` loop in `run_affected`.
