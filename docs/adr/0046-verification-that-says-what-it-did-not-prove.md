# 0046. Verification that says what it did not prove
Status: accepted

## Context
Two measured holes in the maintenance loop. `scripts/ci.sh` ran with `set -e`: when the parallel test group failed, the serial group
(real-time network suites, `server_env`, `ai_tasks`) never ran and nothing said so. With one injected failure in each group the run
reported one failing target out of two, and a real session paid a second full CI cycle to find the others. `affected` judged a step
by its exit code: a library filter that matched no test passed, and three filters in `docs/features.json` (`sim::interact`,
`sim::replay`, `tools::plan`) plus one misleading one (`player`) selected nothing, so a change there was "verified" by other modules' tests.

## Decision
- `scripts/ci.sh` runs every requested stage and both test groups, logs each stage to `out/logs/ci-<stage>.log`, and ends with one
  summary: PASS / FAIL [category] / NOT RUN per stage, each failed test with its location, message and a `repro:` command that runs it
  alone (`scripts/ci_summary.awk`), exit 1 on any failure. `CI_FAIL_FAST=1` keeps the old stop-early behaviour, and says what it skipped.
- `affected` gives every step a verdict, not an exit code: `FAIL [empty-selection]` when a suite ran no tests or a claimed library
  filter matched none (a filter only *guessed* from a changed path is a note instead), a failure category (`tool-missing`, `compile`,
  `format`, `lint`, `test`, `empty-selection`), failed tests with location and repro line, and the steps it did not run.
- `features --check` rejects a `lib:` filter whose module has no `#[test]` (static, catches it before a run); the four bad filters are
  removed. `docs/KNOWLEDGE.md` gains `VERIFY-001`.

## Consequences
One CI run surfaces every failure (measured: 2 of 2 injected, 612-byte summary, vs 1 of 2 in 39 KB); it takes as long as the whole
requested suite (278 s vs 171 s here, the difference being the serial group that used to be skipped). No check was removed or narrowed.
Not done: timeouts per step inside `affected` (`scripts/dev` wraps commands in one), doc-test filter checks, build-profile changes.
