# 2026-10-11. The launchpad routes by what a task names
Status: accepted
Summary: start routes a scene in a checkout to the recipe -> lint -> verify loop, recognises 'new 2D game', picks engine-change owners by file names and commands before summary words, and names directories from the idea's words.

## Context
The agent-loop trace (docs/analysis/agent-loop-trace.md, sink 1) ran three tasks from `scripts/dev start` (ADR 2026-10-06-an-ai-launchpad-and-one-executable-resolver).
All three were routed badly:
- **a. "New 3D scene: a two-room cabin ..."** went to `game-change` ("room"), whose plan assumes a game project: NEXT was `scripts/dev red check`, a command that
  exists only in a project's `scripts/red`. The agent built the CLI for ten minutes to reach `error: unrecognized subcommand 'check'`, then found its way by three
  searches (17 KB), the right recipe ranked 9th of 9.
- **b. "New 2D game: ..."** was `unrouted`: the cue `new game` does not match "new 2D game". The rerun proposed the directory `second-collect-dodge` (from
  "90-second").
- **c. "Engine change: add a lint code ..."** got owners by summary words: scene_format, idea_forge, karts. NEXT was `context scene_format`, 12 KB about
  the wrong feature; lint lives in `map_analysis`.

## Decision
- **A scene in the engine checkout** (`game-change` with no game project) gets its own plan, `plan_scene`: NEXT is `recipe` (after the build when there is no
  executable), then `recipe NAME --new out/scene.json`; checks are `lint`, `plan` (look) and `verify` on that file; a `.json` the task names that exists is
  linted first instead. Pointers come from the task's topic words (`SCENE_TOPICS`: style -> `describe style`, ui/dialogue/story -> `describe ui`,
  key/lock/rules -> `describe rules` and `recipe gated_garden`, props -> `catalog`); the text view shows five pointers instead of three. A project's
  `scripts/red check` is never named outside a project (test).
- **Cues:** `new <up to three words> game` and `2d game`/`3d game` mean game-create; `scene` counts towards game-change like `map`.
- **Owners:** a task word that is the file name of a file a feature owns (`lint` -> `src/tools/lint.rs`) scores 4, a `red_engine2 <word>` command the
  feature lists scores 2, before the 1 point a summary word gets; words that say what to do (`change`, `add`, `fix`, `engine`, ...) never match file names.
- **Directory names** skip tokens with a digit (`90-second`, `2d`): they say how, not what.
- No new command, workflow or file: four rules in `scripts/launchpad.py`, guarded by `scripts/test_launchpad.py` (`TraceRoutes`, the trace's three tasks verbatim).

## Consequences
The trace's tasks, before and after (`start` output; tokens = bytes / 4):

| task | before | after |
|---|---|---|
| a | `game-change`; NEXT `scripts/dev red check` (does not exist: a failed command after the build); 3 searches, 17 KB, to find `gated_garden` and the topics | NEXT `describe --brief` (build) then `recipe`; pointers `describe style`, `describe ui`, `describe rules`, `recipe gated_garden`: the four calls the trace needed, one of which it found 9th in a search (5.5 KB saved) |
| b | `unrouted` (1,115 B) + rerun with `--workflow` (2,068 B), directory `second-collect-dodge` | `game-create` first time (2,754 B, one call), directory `collect-dodge-unique` |
| c | owners scene_format/idea_forge/karts; NEXT `context scene_format` (11,971 B wasted, ~3,000 tokens), then `context lint` | owners map_analysis, scene_format; NEXT `context map_analysis` (the 10.6 KB packet the trace needed) |

- `start` output grows by ~550 bytes on the scene route (the pointers); it saves a failed command, a reroute and a wrong 12 KB packet.
- Routing stays word-based: a task that names nothing still needs `--workflow`. Undo: revert the four rules; the tests name what each one fixed.
