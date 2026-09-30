# 2026-09-30. Bounded iteration, explicit configuration and safe green stamps
Status: accepted
Summary: scripts/dev iterate is a partial tier that can never count as verification; plans state their features and toolchain and stamps key on them; the feature index is read from the checkout so planning is never stale; preflight can run tree-only; heavy jobs take a lock.

## Context
Measured on the 4-core dev box that also hosts the game server (`benches/history/build-times.json`, run `2026-09-30-dev-workflow-iterate`; wall seconds,
steady state after a real fmt-clean edit, one command at a time):

- The dev-profile loop was already fast: `cargo check` 2.4 s, clippy 4.3 s, rebuilding the lib tests 3.8 s. The expensive habit was `--release` (a release binary
  for a one-line edit took 3 min 18 s, the release suite about 15 min); the docs never said not to, and `CLAUDE.md` contradicted itself (it forbade running the suite
  in the edit loop, then told you to run `scripts/dev test` after changing Rust).
- `scripts/dev` planned with a *stale compiled-in* feature index. `planner()` saw `docs/features.json` newer than the binary and called `cli()`, whose
  `needs_build()` only watches `src/`, `assets/` and `Cargo.*`, so nothing rebuilt: after editing the index, `scripts/dev context` still printed the old text
  (reproduced). Had it rebuilt, it would have cost a full default-features build (97 s after a batch of edits).
- `preflight` ("compiles nothing") went through `scripts/dev red`, which rebuilt the graphics-enabled CLI after every source edit: 5.9 s against 1.3 s.
- `affected` compares with the merge base with `origin/main`. On a feature branch that made "quick" cover the whole branch: 17 files, 96 s of steps, 140 s wall,
  303 CPU-seconds and 1.2 GB for a one-file edit; `--base HEAD` existed but no document mentioned it, and a boundary file (`Cargo.toml`, `src/lib.rs`) escalated
  any run to the full CI.
- The quick plan ran a `doc` step for 5.3 s and 0 tests: a fenced `json` block in a `//!` comment counted as a doc example.
- Stamps keyed on scope, package version, the compiled index and the changed files' contents only: not the base commit, the feature set, the toolchain or the
  plan variant.
- Not a lever: cargo `-j2`/`-j3`/`-j4` made no difference to rebuilding all 42 test binaries after a central edit (means 48.5 / 43.5 / 49.5 s; 36-61 s within one
  setting). That step writes about 1.3 GB and is disk-bound (25-44 s of I/O stall), 57-64 CPU-seconds and about 1.7 GB peak tree memory at every setting.
  Headless `cargo check` was only 0.3-0.4 s faster than default when warm.

## Decision
- **`scripts/dev iterate` = `red_engine2 affected --partial`** is the edit loop and a new lowest tier, `Scope::Partial`. It looks only at changes since `HEAD`, runs
  `cargo fmt --check`, `cargo check` of the touched targets and the unit tests of the touched modules (`--check-only` drops the tests), and a changed test file runs
  itself. It never escalates: a boundary file is reported under `Plan::full_required`. It prints "NOT verification", lists the integration suites it did not run, and
  its JSON says `"verified": false, "partial": true`. The ladder is `iterate` (every edit), `affected --quick`, `affected` (before "done"), `affected --full` (before
  pushing); `CLAUDE.md`, `AGENTS.md` and `docs/AGENT_REFERENCE.md` say the same thing.
- **A partial pass is isolated.** `scopes_covering(Partial)` is `[Partial]`, no other scope's lookup reads a partial stamp, and a full pass is not assumed to have
  run the partial steps. Regression tests cover both directions.
- **Plans state their configuration** (`Config`: cargo feature set, `rustc -vV`, result-affecting environment such as `RUSTFLAGS` and `RUST_TEST_THREADS`; jobs and
  target directory are shown but are not part of the identity) and **stamps key on** that configuration, the resolved **base commit** and the plan variant, plus the
  feature index **as on disk** and a planner revision. A list of files typed on the command line is never cached (it says nothing about the rest of the tree).
- **Headless is explicit.** `--headless` (partial only) type-checks with `--no-default-features` only when every changed file is provably graphics-free
  (`headless_safe`: a `src/` file whose top-level module is not `gfx`-gated in `src/lib.rs`, not the live client, with no `feature = "gfx"` in its text); otherwise
  it uses the default features and says why. It is not automatic: warm, it saves 0.3-0.4 s, and a first headless check costs a separate dependency check build.
- **The feature index is read from the checkout** (`features::load_at`, `serial_suites_at`, used by `affected`, `context`, `impact`, `features --check`); the compiled
  copy is only the fallback outside a checkout. `planner()` now watches only the planner's *logic* files, so editing the index needs no rebuild and cannot be stale.
- **`preflight --tree`** skips exactly the two checks that read the compiled binary (the CLI command table and the `describe` byte budgets) and names them;
  `scripts/dev preflight` uses it when the binary is older than the sources and `--full` builds first.
- **The `doc` step** is dropped unless a changed file has a runnable doc example (`has_runnable_doc_example`: empty or rust-ish fence info, not `json`/`text`/`ignore`).
- **One heavy job per target directory** in `scripts/dev` (`flock`; `RED_WAIT=1` queues, otherwise exit 75 with a message). Light commands are never blocked.
- Not changed, on the evidence: no cargo job-count or test-thread preset, no sccache, no automatic headless.

## Consequences
- On the same edit, steady state: `preflight` 5.9 s -> 1.3 s; the edit loop `affected --quick --base HEAD` 23.7 s -> `iterate` 7.8 s (8.9 CPU-seconds; 1.6 s with
  `--check-only`); a central `schema.rs` edit 36.7 s -> 9.2 s; a docs-only edit 20.8 s -> 0.3 s (nothing to compile or run). `affected --quick` itself is
  unchanged apart from the dropped `doc` step (20.5 s against 23.7 s; within the run-to-run noise of this box).
- Planning no longer rebuilds the graphics-enabled engine after an ordinary source edit or an index edit; it does when the planner's own logic files change (97 s once).
- The Windows twin `scripts/dev.ps1` gets `iterate` and the smaller planner list, but no lock and no tree-only preflight; not measured on Windows.
- A partial pass is easy to mistake for a verification in a summary, which is why every output line says it is not. Undo: drop `--partial` from the docs; the other
  tiers and the stamp file format are unchanged (old stamps simply stop matching once).
