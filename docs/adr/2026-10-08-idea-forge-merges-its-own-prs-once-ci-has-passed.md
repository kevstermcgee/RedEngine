# 2026-10-08. Idea Forge merges its own PRs once CI has passed
Status: accepted
Summary: settle merges only the tool's own PRs, only when every check passed and the changed files are within the PR kind's limits; config --auto-merge off, nightly --no-merge or ship --hold stop it.

## Context
Until now every PR the nightly loop opened waited for a person to say "merge". That made the engine's improvement depend on someone being at the keyboard each morning: fixes sat open, their issues stayed `in-progress`, and the next night's worktrees (cut from `main`) did not include them. The maintainer asked for the opposite default: PRs merge after they pass.

## Decision
- `idea_forge.py settle` merges a PR only when **all** hold: the PR is one this tool opened (the ledger or the improvements record); it is open and GitHub reports no conflict; every CI check has finished and at least one passed and none failed (no checks at all is "pending", never "passed"); and the files GitHub lists for the PR are within its kind's limits (a game PR: its game and its feedback; a feedback-only PR: only the feedback; a fix PR: nothing under `.github/`, no `Cargo.*`, not this tool or earlier feedback). The path check uses GitHub's own file list at merge time, not what the run claimed.
- `nightly` settles at its start (yesterday's finished PRs, so today's worktrees are cut from a `main` that has them) and at its end, waiting up to `--settle-timeout-min` for tonight's checks; unfinished PRs stay tracked.
- A failed check, a conflict, a refused path or a merge error leaves the PR open and records why. The merge itself is a plain merge commit through `gh`, as the repository does, never an admin override and never `--auto` on a branch GitHub has not protected.
- Off switches: `config --auto-merge off` (persistent), `IDEA_FORGE_AUTO_MERGE=0`, `nightly --no-merge` (one run), `ship --hold` (one PR).

## Consequences
- An unattended agent can change `main`. The defences are the ones the maintainer already relies on (hosted CI on Linux, Windows and the headless build), plus the CLI's own gates before the PR exists (preflight and `affected --quick` without the result cache), the path limits, a required regression test for a fix, and the recorded fix that makes a repeat visible as `recurring`. They do not prove a change is wise: a test can pass and the fix still be poor. Reading merged fixes in the morning (`git log`, `ledger`) remains worthwhile, and any can be reverted.
- Two merged games a day grow CI time linearly; `ship --hold` or turning merging off are the brakes.
- `fixes.json` is appended by fix PRs; two fix PRs open at once would conflict on it, and the second is simply left open (conflict) for the next night.
- Undo: `config --auto-merge off`.
