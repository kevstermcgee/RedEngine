# Idea Forge: idea -> game -> engine feedback, automatically

`scripts/idea_forge.py` (standard library only) closes the loop the engine's maintainers want: an AI builds a *different* game each run, and what it learned about the engine comes back as structured feedback.

```
idea_forge.py idea [--n N]        the next idea nobody was given yet: YOU (what the hands do), BUT (the law that bends), PUSHBACK, GOAL, optional STORY
idea_forge.py run --kind 2d|3d   forge an idea, make a worktree (seeded build), run a headless `claude -p` agent in it, score the game, measure the friction
idea_forge.py daily [--next]      today's two games: one 2D and one 3D, in a random order each day; build what is pending, ship each as a PR
idea_forge.py schedule [--install]  the cron lines that run `daily --next` at 09:30 and 17:30 (printed; installed only with --install)
idea_forge.py ship SLUG           commit ONLY the game and its feedback file, push the branch, open a PR on RedEngine (never merges), archive the run, remove the worktree
idea_forge.py digest [--write]    every run's findings grouped and ranked: the engine backlog
idea_forge.py ledger | brief | note | feedback    (the agent's side: see below)
```

## One run

1. **Idea.** The vocabulary is `examples/2d/idea-forge/vocabulary.json`, the same file the Idea Forge game uses (8,153,726,976 combinations). Each machine walks it from a random start with a step coprime to the space, so no idea is handed out twice (state in `$IDEA_FORGE_HOME`, else `~/.local/state/idea-forge`).
2. **Build.** `run` makes `../RedEngine-idea-<slug>` on branch `idea-<slug>` (`scripts/dev worktree`, so the dependencies are already compiled; run it from a checkout that has built), writes the brief (`idea_forge.py brief`) and starts `claude -p` there with `RED_TRACE` set, every command logged, the transcript kept, and `git push`/`gh`/`curl` denied. The brief makes the mechanic the game, demands a scenario that exercises it, and forbids engine changes: **a missing feature is a finding, not a patch**.
3. **Track issues.** The agent calls `idea_forge.py note "<what happened>" --area A --cost-min N --fix "<engine change>"` the moment it hits friction. `--area` is one of discovery, docs, cli, diagnostics, format, runtime, verify, publish, perf, tooling, idea-fit, worked.
4. **Feedback.** `feedback --init` drafts `docs/analysis/idea-forge/<date>-<slug>.md` from the notes; the agent ranks and completes it. It contains a fenced ```` ```json findings ```` block (`id, area, title, severity 1-3, cost_min, evidence, workaround, proposal`) plus *Idea fit* (which parts of the card the engine could not express: the feature backlog) and *What worked*. `feedback --check` rejects TODOs, empty evidence, a finding with no proposed engine change. After the run the CLI appends the automatic measurements (the `scripts/agent_bench.py summary` friction report over the trace and transcript, turns, tools, cost).
5. **Ship.** `ship SLUG` refuses unless the changed paths are exactly `examples/2d/<slug>.game2d.json` and the feedback file, and the feedback passes `--check`. It opens a PR; merging and publishing stay human.
6. **Improve the engine.** `digest` ranks findings across runs (severity, minutes lost, how many runs hit it), so recurring friction rises and one-off noise does not.

`run` and `ship` need a POSIX shell (they call `scripts/dev worktree`, `git` and `gh`); `idea`, `brief`, `note`, `feedback`, `digest` and the unit tests also run on Windows; `daily` and `schedule` need a POSIX shell and cron.

## Two games a day

`daily` keeps a ledger per day: one **2D** game (`examples/2d/<slug>.game2d.json`, `describe 2d`) and one **3D** game (a first-person scene `examples/3d/<slug>/<slug>.json`: `rules`, `zones`, `checks.sim`, `verify`), each from its own fresh idea. The order is shuffled once per day and then fixed, so neither dimension is always first. `daily` runs whatever is still pending; `daily --next` runs one game, which is what the schedule calls twice a day, so a missed run is caught up by the next one. A slot whose agent produced nothing that builds is retried once with a new idea, then left for a human; a built game whose feedback is unfinished waits for review with its worktree kept. One daily run at a time (a pid lock).

`schedule` prints two cron lines (`--times 09:30,17:30`, `--budget`, `--model`) that bake in the current `PATH` and `IDEA_FORGE_HOME`; `--install` writes them to your crontab (idempotent, marked `# idea-forge`), `--uninstall` removes only those. Nothing is scheduled until you install it. Each game is capped at `--budget` USD (default 15), so a day costs at most twice that; the log is `$IDEA_FORGE_HOME/daily.log`. Run it from a dedicated checkout that has built once: each run's worktree is seeded from that checkout's `target/` (about 1.3 GB, 4 s) and removed after its PR is opened, with the brief, notes, command trace and transcript kept in `$IDEA_FORGE_HOME/runs/`.

CI protects what gets merged: `tests/games2d.rs` verifies every `examples/2d/*.game2d.json`, `tests/games3d.rs` every `examples/3d/<slug>/<slug>.json` (and requires a scripted playthrough). Two merged games a day is real CI load: merge the feedback you want, close the PRs you do not.

A run spends model tokens (set `--budget`). It needs `claude` (Claude Code) on `PATH`, `git`, and `gh` for `ship`.

Tests: `scripts/test_idea_forge.py` (run by `tests/idea_forge.rs`) uses a stub `claude` and a fake engine, so the whole loop is exercised without a model.
