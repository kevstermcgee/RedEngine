# 2026-10-08. Idea Forge runs an agent on a fresh worktree and ships only a game and its feedback
Status: accepted
Summary: scripts/idea_forge.py forges a non-repeating game idea, runs a headless agent to build it, and files structured engine feedback; the agent may not change engine code, and ship refuses any other path.

## Context
The engine improves when a fresh agent builds something real and reports where it lost time (docs/analysis has a dozen such notes, each written by hand after a one-off game). Idea Forge (the 2D game, PR #58) showed the cost of doing it by hand: the useful output was the issue log, the game was incidental, and the log was assembled from memory at the end. The request was to automate the loop: a utility that invents a game idea, has an AI build it with the engine, and has that AI file the feedback in the repository.

## Decision
- `scripts/idea_forge.py` is a standard-library CLI. Ideas come from the 2D game's own vocabulary and walk (`examples/2d/idea-forge/build.py`, one source of truth), so none repeats on a machine.
- A run is one headless `claude -p` agent in a fresh `scripts/dev worktree`, with `RED_TRACE` on, the transcript kept, and `git push`/`gh`/`curl` denied. The brief makes the mechanic the game and requires a scenario that exercises it.
- **The agent may not change engine code.** A missing feature is a finding, not a patch: runs stay comparable, and the only products are `examples/2d/<slug>.game2d.json` and `docs/analysis/idea-forge/<date>-<slug>.md`. `ship` refuses any other changed path and an unfinished feedback file, and never merges.
- Issues are logged at the moment they happen (`note`, with an area, minutes lost and the engine change that would have prevented it), not reconstructed. The feedback file carries a machine-readable findings block; `digest` ranks findings across runs so recurring friction outranks one-off noise.
- Measurements the agent cannot fake (command trace, failures, repair cycles, source reads, tokens, cost) are appended by the CLI from `agent_bench.py summary` and the transcript.

## Consequences
- Each run costs model tokens (`--budget` caps them) and produces a PR to review; nothing publishes or merges by itself.
- Findings are the agent's own account plus measurements; the digest is only as good as the agent's honesty, which is why evidence is a required field and the measurements are separate.
- Ideas are prompts built from a hand-written vocabulary: the idea space grows by editing `vocabulary.json`, not by the CLI.
- Undo: delete `scripts/idea_forge.py`, its test and docs; the game and the feedback files stand alone.
