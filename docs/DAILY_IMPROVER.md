# The daily improver

Once a day (23:00 on the owner's machine, the `redengine-nightly` systemd timer) an AI session reads everything people and AIs said about
RedEngine, the RedEngine Games app and its website, fixes what it can verify, and lands each repository's work as a pull request that merges
itself once its hosted CI passes. Both maintainers see every change as a pull request and can stop any of them by closing it.

```text
python3 scripts/daily_improve.py prompt        the brief tonight's session would get (the inbox and the rules), read it first
python3 scripts/daily_improve.py run --dry-run the same, from the run's own path; changes nothing
python3 scripts/daily_improve.py settle        merge earlier daily pull requests whose checks passed, close failed ones after 3 days
python3 scripts/daily_improve.py install       point this machine's redengine-nightly at this script on origin/main
```

## A day

1. **Settle** the earlier days' pull requests (`daily/<date>` branches in both repositories). A pull request is merged when every hosted
   check passed. It is left open, with the reason, when its checks failed, it conflicts with main, or it is a draft; after three days such a
   pull request is closed (the branch stays in its history). One that touches a refused path is left for a maintainer and never closed.
2. **Read the inbox**: `scripts/feedback.py digest` since the last run ([`docs/feedback/README.md`](feedback/README.md) says what it
   reads and how much each source is trusted). The run is skipped only when nothing is new and nothing is open.
3. **One session** (`claude -p` with `scripts/daily_improve.md` as its brief, bypass permissions, a dollar cap, a deadline enforced from
   outside, and no tools that push, merge or write to GitHub) works on `daily/<date>` in two checkouts: this engine and a RedEngineGames
   worktree beside it. It first checks the two oldest open keys against main and records any that are already fixed in
   `docs/analysis/idea-forge/fixes.json`, so the inbox stays honest. Then it fixes at most three items, one commit each, each with tests and
   with its key's `fixes.json` entry (or `Fixes owner/repo#N`), and writes the report. Uncommitted work is thrown away.
4. **Land** each repository that has commits:
   - Refused paths: CI, the improver's own rules, versions and dependencies, the player reviews data, a maintained game (one whose playable
     names a `"maintainer"`), and in RedEngineGames the app's version, the release lists and the games' folders.
   - Local checks: `scripts/dev affected` in the engine; in RedEngineGames, the site and distribution tests and the app's core tests, for
     what changed.
   - Push, then open a pull request whose body is the session's report.
   - When merging is on (`NIGHTLY_PUSH_MAIN=1`), nothing was refused and the local checks passed, the improver waits (`NIGHTLY_WAIT_MIN`,
     default 90) for hosted CI and merges once every check passed. Otherwise the pull request stays open: a **draft** when it needs a
     maintainer.
5. **The report** is `~/nightly-reports/<date>.md` (and `latest.md`) on the machine, the body of each pull request, and its transcript
   `<date>.claude.jsonl`.

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
| `NIGHTLY_WAIT_MIN` | 90 | how long to wait for hosted CI before leaving the pull request to tomorrow's settle |
| `NIGHTLY_MODEL` | (default) | the session's model |

Build output goes to the USB drive (`/data/build/nightly`, `/data/build/nightly-launcher`). The timer's unit skips the day when the drive is
missing. Stop the improver with `systemctl --user disable --now redengine-nightly.timer`; `daily_improve.py install` kept the previous script
beside the new one as `redengine-nightly.bak-<date>`.

The session runs with bypass permissions on the machine, as the old nightly did. Its disallowed tools stop mistakes, not a determined
program. The protection is the wrapper's: it alone pushes, it refuses paths, it merges only on green hosted CI, and every change is a
pull request.
