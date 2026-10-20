# 2026-10-20. The daily improver lands its work as pull requests that merge on green
Status: accepted
Summary: scripts/daily_improve.py replaces the nightly's direct push to main: one session per day reads the feedback inbox, works in the engine and RedEngineGames, and each repository's commits become a pull request that merges itself once every hosted check passed, after refusing paths only a maintainer may change

## Context
On 2026-10-20 the owner asked for RedEngine to improve itself with a daily scheduled session that reads feedback from everywhere: AI feedback
from making games, the owner's, the cousin's (now an equal maintainer), and players' from the website and the app. The engine, the app and the
games all improve. The machine already ran `redengine-nightly` at 23:00, an owner-written bash script with its brief in `~/.config/`. It
gated on new feedback, ran one Claude session for at most 3 small fixes, ran `affected --full` locally and pushed straight to main when that
passed and main had not moved; otherwise it pushed `nightly/<date>`.

Measured over its last 10 runs (2026-10-10 to 10-19): 2 landed, 5 had nothing to do, 4 left a branch nobody picked up (2 local CI failures,
1 "main moved", 1 earlier). It read only part of the feedback: PR comments but not PR bodies (Snake #201's engine notes never reached it), no
RedEngineGames app or site work, and players' scores only. Nobody but the owner could see or change its rules.

## Decision
- **The improver lives in the repository** (`scripts/daily_improve.py`, its brief `scripts/daily_improve.md`, `docs/DAILY_IMPROVER.md`), so
  both maintainers read and change what runs. On the machine, `daily_improve.py install` turns `~/.local/bin/redengine-nightly` into a shim that
  resets a dedicated worktree to origin/main and runs the script from it. This is the Idea Forge's model (`RedEngine-factory`). The timer, its
  unit and the settings file (`~/.config/redengine-nightly/env`) stay as they were.
- **It reads one inbox** (`scripts/feedback.py`). Every run first checks the two oldest open keys against main and records
  what is already fixed, because 6 fixes had been recorded against 132 keys.
- **It works in two repositories**: the engine checkout and a RedEngineGames worktree, on `daily/<date>`. The app and the site improve the
  same way the engine does.
- **Every change is a pull request.** The wrapper refuses paths a maintainer must decide (`NEVER`: CI, its own rules, versions and
  dependencies, the reviews data, release lists, games' folders; `maintained_paths`: any game whose playable names a maintainer), runs the
  local checks, pushes, and opens the pull request with the session's report as its body. With merging on (`NIGHTLY_PUSH_MAIN=1`, the owner's
  existing switch), it merges once **every hosted check passed** (`idea_forge.checks_verdict`: no checks never counts as green). It waits up
  to `NIGHTLY_WAIT_MIN`, then the next day's `settle` takes over. A failed, conflicting or draft pull request stays open with its reason, and
  is closed after three days. A refused one waits for a maintainer and is never closed.
- RedEngineGames gained a pull-request check for `site/` and `distribution/` (its `checks.yml`), so its daily pull requests have something
  to be green on.
- Uncommitted work at the end is thrown away. The old script committed it as "leftovers" and let CI decide.

## Consequences
Main moving no longer strands a night's work, and both maintainers see each change and its report as a pull request; closing it stops it.
Landing needs hosted CI (minutes to an hour) rather than the box's local `affected --full`: the local gate is now `affected` (closure scope)
and the hosted suite is the merge gate. A cousin's merged change to the script runs on the owner's machine the next night, as any change to
main already did through the build. Undo: restore `redengine-nightly.bak-<date>` beside the shim, or set `NIGHTLY_PUSH_MAIN=0` to keep the
pull requests and merge none.
