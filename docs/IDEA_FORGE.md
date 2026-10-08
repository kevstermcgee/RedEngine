# Idea Forge: idea -> game -> engine feedback, automatically

`scripts/idea_forge.py` (standard library only) closes the loop the engine's maintainers want: an AI builds a *different* game each run, and what it learned about the engine comes back as structured feedback.

```
idea_forge.py idea [--n N]        the next idea nobody was given yet: YOU (what the hands do), BUT (the law that bends), PUSHBACK, GOAL, optional STORY
idea_forge.py run --kind 2d|3d   forge an idea, make a worktree (seeded build), run a headless `claude -p` agent in it, score the game, measure the friction
idea_forge.py nightly             what cron runs (06:00): today's two games ONE AT A TIME (one 2D, one 3D, random order), then ONE engine improvement from the backlog they fed
idea_forge.py daily [--next]      just the games; idea_forge.py improve   just the engine improvement (a PR; never merged)
idea_forge.py backlog             every finding of every run, grouped by key and ranked, with what was done about each
idea_forge.py schedule [--install]  the cron entry for `nightly` (printed; installed only with --install)
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

## The nightly loop: games, feedback, engine improvement

`nightly` is one cron entry (default 06:00) holding one lock, so **exactly one agent works at a time**: game 1, game 2, then the improvement.

1. **Two games** (`daily`): one **2D** (`examples/2d/<slug>.game2d.json`) and one **3D** (a first-person scene `examples/3d/<slug>/<slug>.json`), each from its own fresh idea, the order shuffled once per day. A slot whose agent built nothing is retried once with a new idea.
2. **Feedback always arrives.** Whatever happens, the run's feedback reaches the repository: the agent's own file when it is sound; otherwise the CLI completes it (the notes and the measurements are real, prose the agent never wrote is marked "not completed") and ships it **with the game when the game built and without it when it did not**. A timeout, an exhausted budget or a crash still produces a feedback PR, because the failed runs are the most useful ones. A copy of every feedback file is also kept on the machine (`$IDEA_FORGE_HOME/feedback/`), so the backlog never waits for a merge.
3. **The backlog** (`backlog`): every finding has a stable kebab-case **key** (`no-random-expression`, `cold-worktree-build-and-preflight-rebuild`). Findings with the same key from different runs are one issue, ranked by severity, then how many runs hit it, then minutes lost. Statuses: `open`, `in-progress` (a fix PR is open), `addressed` (a fix is merged: it is in `docs/analysis/idea-forge/fixes.json` on `main`), `recurring` (reported again in a run dated after its fix: a regression), `rejected` (its fix PR was closed). Each game agent's brief lists the known open issues so it adds evidence under the same key instead of rediscovering them.
4. **One engine improvement** (`improve`): an agent in a fresh worktree reads the top of the backlog, picks the most valuable issue it can fix completely and safely, fixes it, adds a regression test, records the fix in `fixes.json`, and logs its own friction. **The CLI then runs `scripts/dev preflight` and `scripts/dev affected --quick` itself** and opens a PR only if they pass, and only if the change touches nothing off limits (`.github/`, `Cargo.*`, this tool, earlier feedback), includes a test, and records the fix. Merging the PR marks the issue addressed; if a later run reports it again it comes back as `recurring`. Nothing is merged for you.

`schedule` prints the cron line (`--times 06:00`, `--budget` per game, `--improve-budget`); `--install` writes it to your crontab (idempotent, marked `# idea-forge`), `--uninstall` removes it. Nothing is scheduled until you install it. A night costs at most two game budgets (default $15 each) plus the improvement (default $25); the log is `$IDEA_FORGE_HOME/nightly.log`. Run it from a dedicated checkout that has built once: each run's worktree is seeded from that checkout's `target/` (about 1.6 GB, 4 s) and removed after its PR is opened, with the brief, notes, command trace and transcript kept in `$IDEA_FORGE_HOME/runs/`.

CI protects what gets merged: `tests/games2d.rs` verifies every `examples/2d/*.game2d.json`, `tests/games3d.rs` every `examples/3d/<slug>/<slug>.json` (and requires a scripted playthrough). The engine only improves overnight if you merge the fix PRs; merging two game PRs a day is real CI load, so merge the feedback and fixes worth keeping and close the rest.

A run spends model tokens (set `--budget`). It needs `claude` (Claude Code) on `PATH`, `git`, and `gh` for `ship`.

Tests: `scripts/test_idea_forge.py` (run by `tests/idea_forge.rs`) uses a stub `claude` and a fake engine, so the whole loop is exercised without a model.
