# 2026-10-08. Idea Forge makes two games a day, one 2D and one 3D, in a random order
Status: accepted
Summary: idea_forge.py daily keeps a per-day ledger of one 2D and one 3D game in an order shuffled once per day; cron runs it twice a day; each run is shipped as a PR, archived and its worktree removed.

## Context
The Idea Forge loop (ADR 2026-10-08-idea-forge-runs-an-agent-on-a-fresh-worktree) produced one 2D game per manual run. The request: make it a daily routine of two games, one 2D and one 3D, in any order. Two things had to be decided: what a 3D game is in a repository that has only single-scene 3D examples (Lantern Walk), and how an unattended job stays safe and cheap.

## Decision
- A 3D game is a first-person RedEngine scene `examples/3d/<slug>/<slug>.json` with whatever sits beside it (audio scores). It is built from `recipe`/`catalog`/blueprints and rules as data, and proven with `lint`, `reach`, `checks.sim` and `verify`, the same way Lantern Walk is. A 2D game stays one `examples/2d/<slug>.game2d.json`. The run's allowed paths follow: a 3D game owns its folder, a 2D game one file, plus the feedback file; `ship` refuses anything else.
- `daily` records one slot per kind per day. The order is shuffled once per day and then fixed ("any order", never always the same). `--next` runs one game; cron calls it twice a day, so a missed run is caught up by the next. A slot that produced nothing that builds is retried once with a fresh idea, then left for a human; a built game with unfinished feedback is held for review.
- Unattended safety: a pid lock (one daily run at a time), a USD cap per game, PRs only (never a merge, never a publish), the agent cannot push or call `gh`/`curl`, and nothing is scheduled until `schedule --install` is run.
- Disk: each run's worktree holds a seeded 1+ GB build, so after the PR is opened the run's brief, notes, trace and transcript are archived under the state directory and the worktree is removed (the branch stays: it is pushed).
- CI: `tests/games3d.rs` verifies every `examples/3d/<slug>/<slug>.json` and requires a scripted playthrough, as `tests/games2d.rs` does for 2D, so a merged game cannot turn CI red later.

## Consequences
- Up to two PRs and two model sessions a day (cap: twice the budget). Merging all of them grows CI time linearly: merge the feedback worth keeping and close the rest.
- A 3D game can only use what rules-as-data can express: that limit is itself the most useful feedback ("Idea fit").
- The schedule is cron and POSIX; other schedulers can use the printed lines.
- Undo: `schedule --uninstall`; delete `daily`/`schedule` from the script. Single runs (`run --kind`) are unaffected.
