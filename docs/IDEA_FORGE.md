# Idea Forge: idea -> game -> engine feedback, automatically

`scripts/idea_forge.py` (standard library only) closes the loop the engine's maintainers want: an AI builds a *different* game each run, and what it learned about the engine comes back as structured feedback.

```
idea_forge.py idea [--n N]        the next idea nobody was given yet: YOU (what the hands do), BUT (the law that bends), PUSHBACK, GOAL, optional STORY
idea_forge.py run [--budget USD]  forge an idea, make a worktree (seeded build), run a headless `claude -p` agent in it, score the game, measure the friction
idea_forge.py ship SLUG           commit ONLY the game and its feedback file, push the branch, open a PR on RedEngine (never merges)
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

`run` and `ship` need a POSIX shell (they call `scripts/dev worktree`, `git` and `gh`); `idea`, `brief`, `note`, `feedback`, `digest` and the unit tests also run on Windows.

A run spends model tokens (set `--budget`). It needs `claude` (Claude Code) on `PATH`, `git`, and `gh` for `ship`.

Tests: `scripts/test_idea_forge.py` (run by `tests/idea_forge.rs`) uses a stub `claude` and a fake engine, so the whole loop is exercised without a model.
