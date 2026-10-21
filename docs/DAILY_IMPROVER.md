# The daily improver

Once a day (23:00 on the owner's machine, the `redengine-nightly` systemd timer) an AI session reads everything people and AIs said about
RedEngine, the RedEngine Games app and its website, fixes what it can verify, and lands each repository's work as a pull request that merges
itself once its hosted CI passes. Both maintainers see every change as a pull request and can stop any of them by closing it.

```text
python3 scripts/daily_improve.py prompt        the brief tonight's session would get (the inbox and the rules), read it first
python3 scripts/daily_improve.py run --dry-run the same, from the run's own path; changes nothing
python3 scripts/daily_improve.py settle        merge green daily pull requests, retry failed checks and update branches behind main
python3 scripts/daily_improve.py install       point this machine's redengine-nightly at this script on origin/main
```

## A day

1. **Settle** earlier daily pull requests in both repositories. Merge only the inspected head when hosted checks have finished with at
   least one success and no failures, and GitHub reports it mergeable. Missing checks and unknown mergeability never count as green.
   A branch behind main is updated with an expected-head guard and must pass CI again. Failed workflow jobs get one retry per head,
   remembered across restarts. Failed work is preserved for repair instead of being closed after three days.
   Only the repository owner's daily branches targeting main qualify; forks and other authors do not. Every changed file page and renamed
   source is checked. A missing ownership manifest or incomplete file list stops the operation.
   RedEngineGames filters its PR workflows by path. If no check appears after five minutes (for example a docs-only change), dispatch
   its existing read-only `checks.yml` workflow on the daily branch, once per head. Dispatching is never a green result: wait for its
   actual completed checks. Release/deployment workflows are never dispatched or retried by this fallback.
2. **Repair** at most one failed or conflicting PR per night in a separate temporary worktree. The session gets a $5 cap and 30-minute
   deadline by default, can read failed workflow logs, merges main, and commits a minimal fix. It cannot change CI, dependencies, its own
   rules or maintained games. Both histories must survive, refused paths are checked again, and a lease prevents overwriting another
   writer. Hosted CI must pass on the repair before it merges. There are at most two repair sessions per PR; exhausted attempts remain
   open with their logs for a maintainer. Deliberately drafted PRs and refused paths remain under maintainer control.
3. **Read the inbox**: `scripts/feedback.py digest` since the last run ([`docs/feedback/README.md`](feedback/README.md) says what it
   reads and how much each source is trusted). The run is skipped only when nothing is new and nothing is open.
4. **One session** (`claude -p` with `scripts/daily_improve.md` as its brief, bypass permissions, a dollar cap, a deadline enforced from
   outside, and no tools that push, merge or write to GitHub) works on a unique `daily/<date>-<time>` branch in two checkouts: this engine and a RedEngineGames
   worktree beside it. It first checks the two oldest open keys against main and records any that are already fixed in
   `docs/analysis/idea-forge/fixes.json`, so the inbox stays honest. Then it fixes at most three items, one commit each, each with tests and
   with its key's `fixes.json` entry (or `Fixes owner/repo#N`), and writes the report. Uncommitted work is thrown away.
5. **Land** each repository that has commits:
   - Refused paths: CI, the improver's own rules, versions and dependencies, the player reviews data, a maintained game (one whose playable
     names a `"maintainer"`), and in RedEngineGames the app's version, the release lists and the games' folders.
   - Local checks: `scripts/dev affected` in the engine; in RedEngineGames, the site and distribution tests and the app's core tests, for
     what changed.
   - Push, then open a pull request whose body is the session's report.
   - When merging is on (`NIGHTLY_PUSH_MAIN=1`) and nothing was refused, open a ready PR even if local checks failed. Record that failure
     in the body; hosted CI is authoritative. After handing off both repositories, the improver waits up to `NIGHTLY_WAIT_MIN` (default 90,
     one shared budget) and the separate settle timer checks
     every 15 minutes after that. Refused changes stay drafts. Older runner-created drafts whose body records a local failure are promoted
     only when hosted CI is green. Closing a PR while it waits cancels the merge.
   - Merges specify `--match-head-commit`, so checks on an earlier head cannot authorize a newer commit. New daily branches are pushed
     without force; rerunning a day cannot replace its previous branch.
6. **The report** is `~/nightly-reports/<date>.md` (and `latest.md`) on the machine, the body of each pull request, and its transcript
   `<date>.claude.jsonl`.

The shim and direct commands share one nonblocking lock. The shim acquires it before fetching or resetting a checkout, so a settle tick
cannot erase an active session's changes. Bootstrap failures stop the command with a failing status. Local checks use the process-tree
supervisor and one overall deadline, so timing out also stops build descendants. The inbox cursor advances only after a completed session
has handed off its changes. `run --dry-run` reads the brief without changing reports or state; invoke it directly, since the shim refreshes
its managed checkout before every command.

The old nightly pushed straight to main. When main had moved, or its local CI failed, it left a `nightly/<date>` branch that nobody picked
up: 4 of its last 10 runs ended that way. A pull request merges whenever main moves underneath it, shows its CI and its report to both
maintainers, and is closed or merged instead of being forgotten.

## What the maintainers do

Nothing, on a normal day: read the pull request when it arrives, or close it to stop it from merging. To steer tomorrow's run, leave
feedback. It all reaches the inbox:
- an issue in either repository;
- a note in `docs/feedback/` (`scripts/feedback.py add`);
- a signed note from the app;
- an "Engine notes" section in a pull request.

A finding about one game goes to that game's maintainer in the report. The improver never edits a maintained game.

## Settings

`~/.config/redengine-nightly/env` (the process environment overrides it):

| Setting | Default | What it does |
|---|---|---|
| `NIGHTLY_MAX_USD` | 25 | the session's dollar cap (`claude --max-budget-usd`) |
| `NIGHTLY_CLAUDE_TIMEOUT` | 3h | the session's deadline, enforced from outside (`scripts/proc_supervisor.py`) |
| `NIGHTLY_CI_TIMEOUT` | 100m | the local checks' deadline |
| `NIGHTLY_MIN_FREE_GB` | 20 | below this much free disk the day is skipped |
| `NIGHTLY_PUSH_MAIN` | 1 | 1: merge a daily pull request once every check passed; 0: leave every one for a maintainer |
| `NIGHTLY_WAIT_MIN` | 90 | shared wait for hosted CI before handing off to the 15-minute settle timer |
| `NIGHTLY_MODEL` | (default) | the session's model |
| `NIGHTLY_REPAIR_MAX_USD` | 5 | dollar cap for the night's one repair session, separate from the improvement session |
| `NIGHTLY_REPAIR_TIMEOUT` | 30m | repair session's process-tree deadline |

Build output goes to the USB drive (`/data/build/nightly`, `/data/build/nightly-launcher`). The timer's unit skips the day when the drive is
missing. The persistent nightly timer catches a missed run after downtime; its service deadline is nine hours to accommodate the capped
session, repair, two local checks and hosted wait. `install` also enables `redengine-nightly-settle.timer` (every 15 minutes, no model or
builds). Its output is in `journalctl --user -u redengine-nightly-settle.service`. To stop both jobs:
`systemctl --user disable --now redengine-nightly.timer redengine-nightly-settle.timer`.
`daily_improve.py install` keeps the previous shim as `redengine-nightly.bak-<date>`.

The session runs with bypass permissions on the machine, as the old nightly did. Its disallowed tools stop mistakes, not a determined
program. The protection is the wrapper's: it alone pushes, it refuses paths, it merges only on green hosted CI, and every change is a
pull request.
