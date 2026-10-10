# 2026-10-08. Idea Forge closes the loop: feedback always arrives, a keyed backlog, and a nightly engine-improvement agent
Status: accepted
Summary: nightly runs the day's games one at a time, delivers every run's feedback (auto-completed when needed), ranks findings by key in a backlog, and runs one agent that fixes the top issue with a test; the CLI re-runs the gates and opens a PR, never a merge.

## Context
Feedback was only useful if it reached the engine and something acted on it. Two gaps: a run whose game did not build (or whose agent timed out or left TODOs) shipped nothing, so the most informative feedback was lost; and the digest was a report no one acted on, so nightly runs produced PRs but not engine improvements. The request: work one game at a time from about 6am, and make sure the feedback finds its way to the engine so the nightly updates improve it.

## Decision
- **Delivery.** `produce` always delivers feedback. A sound agent file ships with the game; an unfinished one is completed by the CLI (`autofill_feedback`: notes and measurements are real, missing prose is marked, a banner says so) and shipped; a game that did not build ships feedback only. Every feedback file is also copied to `$IDEA_FORGE_HOME/feedback/` at once.
- **Keys and a backlog.** Findings carry a stable kebab-case `key`; the same key across runs is one issue. `build_backlog` ranks by severity, runs, minutes and derives a status from `docs/analysis/idea-forge/fixes.json` on `main` (addressed; recurring if reported after the fix), from the open/closed state of fix PRs (in-progress; rejected). Game briefs list the known issues so agents reuse keys and add evidence.
- **Improvement agent.** `improve` runs one agent on a fresh worktree to fix ONE issue with a regression test and a `fixes.json` entry. The agent's word is not trusted: the CLI re-runs `preflight` and `affected --quick`, refuses a change that touches `.github/`, `Cargo.*`, `scripts/idea_forge.py`/its test or earlier feedback, or has no test or no fix record, and only then opens a PR. It never merges.
- **One agent at a time, one entry.** `nightly` takes one lock and runs game 1, game 2, then the improvement; `schedule` installs a single 06:00 cron entry.

## Consequences
- Engine changes are written by an unattended agent. The PR gate (CI plus the maintainer's merge) is the only thing standing between a bad fix and `main`, which is why the CLI adds its own gates, size-free but strict path limits, and a required test.
- The loop improves the engine only as fast as fix PRs are merged; a PR left open takes its key out of the queue (`in-progress`) so the agent moves to the next issue instead of stacking duplicates, and a closed one is not retried.
- Feedback quality depends on the agents; auto-completed files are marked as such so a reader knows which prose is missing, and measured numbers sit beside the claimed ones.
- Undo: `schedule --uninstall`; the improvement step can be skipped with `nightly --skip-improve`.
