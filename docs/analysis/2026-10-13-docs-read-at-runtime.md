# Docs read at runtime: what a doc edit costs, before and after (2026-10-13)

Phase 1 item 1 of ADR 2026-10-13-the-next-phase; decision in ADR 2026-10-13-the-engine-reads-its-own-docs-from-the-checkout.

## Method

One script, run in the same worktree on the 4-core development box: before on `04d12fa` (main), after on the change. For each state it warms the build
(`cargo build --locked --bin red_engine2`, `cargo test --locked --lib --no-run`), then appends one empty line to `docs/GLOSSARY.md` and runs both
commands again, then restores it and adds a new file `docs/adr/2099-01-01-measurement-probe.md` and runs both again, then removes it. Each line is the wall
time of one command and whether its log contains `Compiling red_engine2`. Other agents' builds may share the machine: read the seconds as orders of
magnitude; the compile/no-compile column is exact.

## Results

| edit | command | before | after |
|---|---|---|---|
| one line in `docs/GLOSSARY.md` | `cargo build --bin red_engine2` | 86.2 s, compiles | 0.3 s, nothing compiled |
| | `cargo test --lib --no-run` | 107.9 s, compiles | 0.3 s, nothing compiled |
| a new ADR file | `cargo build --bin red_engine2` | 20.7 s, compiles | 0.3 s, nothing compiled |
| | `cargo test --lib --no-run` | 22.8 s, compiles | 0.3 s, nothing compiled |

The new ADR is found by the binary built before it existed: `red_engine2 search "quokka zebra marmalade probe" --kind adr` printed the probe ADR first,
and `tests/docs_fresh.rs::search_finds_an_adr_written_after_the_binary_was_built` does the same against a scratch checkout. For scale: the docs-only
pull request #116 rebuilt the CLI for 157 s before `preflight` could run, and spent 46.5 job-minutes of hosted CI.

## Not changed by this

Hosted CI: `Swatinem/rust-cache` keeps dependencies only, so every Rust job compiles the workspace crates whatever changed; a docs-only pull request still
compiles in CI. Phase 1 item 4 is what lets such a pull request skip that: it runs the doc-reading suites from main's compiled test archive, which now
read the pull request's docs at run time.
