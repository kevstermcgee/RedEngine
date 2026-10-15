# 2026-10-15. The narrative foundation: what was built, measured and proven

The owner's brief (2026-10-14) asked for a lean, AI-first narrative foundation: readable content, strong authoring contracts, static content
compilation, the existing authoritative simulation, a focused milestone with tests and a playable example. Decision record: ADR
2026-10-15-narrative-is-compiled-content-run-by-one-story-system. Contract and authoring guide: `docs/STORY.md` (`describe story`).

## What was built

| layer | where | what |
|---|---|---|
| host extension | `sim::systems` | typed queries (`System::queries/query/reads`, `Cx::query`, `scene:<var>` through `Outside`), atomic command batches (`Cx::issue`, rollback of every touched system, the ledger, the random stream and the events; `System::settled`), `unattended` commands |
| match hooks | `sim::game`, `sim::trace`, `sim::replay` | `MatchGame::intend` / `MatchSim::intend`, recorded (`Entry::Intent`) and replayed |
| shared core | `sim::story::talk` | what moving on and choosing mean; the 2D `dialogue` widget now calls it (`red2d::talk`) |
| runtime | `sim::story::{package,state,runtime}` | the compiled package (check + bind), the story system: conversations, barks, notices, silences, directors, knowledge with provenance, facts with scopes, saves by id, live inspection, the decision log |
| toolchain | `tools::story`, `red_engine2 story` | parse (Ink/Yarn refused by name), bind, type-check, lint (errors, warnings, narrow `allow`, `request`), compile, `refs`, `ledger`, `review`, `walk` |
| verification | `sim::scenario::story`, `story walk` | scenario story steps; bounded exploration through the real match |
| presentation | `app::dialogue` | optional presenter: reveal then advance, wrap-around focus, 1-9, clicks, gamepad, no input leaking into play |
| content | `content::story`, `schemas/story-*.schema.json`, `validate` | typed definitions, schemas, the drift test, `.story` folders checked by `validate` |
| discovery | `describe story`, `docs/STORY.md` in `search`, launchpad story route, glossary | |
| fixture | `games/tidewatch` | a native 3D reference game: the watch system, the story, the client, 21 tests, 3 scenarios, 2 walks |
| exemplars | `examples/story/{the_debt,the_rumour,rain}` | three annotated stories of different tone and structure, compiled by a test |

Fixed on the way, outside narrative: `validate` re-parsed a scene for `checks.sim` without the scene's folder, so a game block's own files
failed to load (Workshop's `workshop_server cli validate` failed too; it passes now).

## Tooling choice (brief)

Ink and Yarn Spinner each bring their own flow VM, expression language, variable store and save format. Integrating either means a second
evaluator beside `rules_expr`, a second state outside the systems host, saves and randomness bridged into the host's, and a runtime in
every shipped game. That is the brief's second authority and general VM. The small frontend lowers into the rules' own expressions and the
host's own commands instead; no third-party dependency was added (`Cargo.lock` gains only the two Tidewatch workspace members).

## Measurements (Intel N97, 4 cores, shared box; wall times are medians of 10 runs unless said)

**Editing one line rebuilds content, not Rust.** Edit `lent_cheer`'s text in `tidewatch.story`, then:

| step | cost |
|---|---|
| `red_engine2 story build games/tidewatch/content/story` (process included) | 5.3 ms wall (1 ms in the compiler); `story check` 6.0 ms |
| `cargo build -p tidewatch_server` afterwards | **0 crates compiled**, 0.19 s (cargo's fingerprint check) |
| the game loads the new package and plays a story scenario (`tidewatch_server cli sim ... --only lend`) | 9.6 ms; all three scenarios 85 ms |
| `story walk` (2 walks, 8 runs, 28k simulated ticks) | 265 ms |

Cold, once: the CLI and the game's program must exist. On main a prebuilt CLI is fetched without compiling (3.9 s, measured 2026-10-13); this
branch has none. The game's program after this branch's engine changes rebuilt in 2 min 53 s (dev, warm third-party dependencies). Neither
recurs per content edit.

**Run-time cost** (`cargo test -p tidewatch_server --release --test cost -- --ignored --nocapture`): the 28.8 KB package loads, checks and
binds in 0.32 ms (p50; p95 0.41 ms). A scripted two-watch night (7,440 ticks, 47 story decisions) ticks the host at p50 0.18 µs with the
watch alone and 0.23 µs with the story (p95 0.21 → 0.26 µs, p99 0.22 → 0.26 µs). The slowest tick was 70.6 µs: a decision tick, which
evaluates every variable and selects among candidates. The 60 Hz budget is 16,667 µs.

**Size** (lines, doc comments included; not a claim against a budget):

| area | lines |
|---|---|
| runtime (`sim::story`: package, state, runtime, pacing, explain, talk) | 2,721 |
| toolchain (`tools::story`, without its tests) | 3,329 |
| CLI, scenario steps, presenter, content typing | 285 + 145 + 337 + 42 |
| host extension (`sim::systems`, net of its tests moving to `systems/tests.rs`) | about 150 |
| other engine edits (hooks, trace, replay, scenario, validate, describe, search, launchpad) | about 250 |
| Tidewatch: server 674, client 304, story 331 lines of `.story` | |
| tests (toolchain, runtime, host, Tidewatch story/play/cost) | 1,721 |

So about 7.3k engine lines excluding tests and the fixture. That is well above the 1–2k the brief mentions, mostly because of the toolchain:
the parser, linter, references and walk are where the authoring intelligence lives. The runtime part a shipped game links is about 2.7k
lines, and it has no new dependency.

**Compactness:** Tidewatch's story is 331 lines of `.story` text (26 conversation lines, 9 choices, 22 barks, a director). The compiled
package is 28.8 KB of JSON that nobody writes by hand.

## The fresh-agent trial

`.gauntlet/tasks/narrative-01.md` (portable task with semantic outcomes; record in `benches/history/agent-tasks.json`). A fresh agent (Sonnet
5.5, n=1) worked on a copy of Tidewatch's content with the prebuilt binaries and no Rust allowed. It had to discover the workflow, add a
reaction, add a consequential scene, rename a fact and repair its dependents, and prove it.

- **Result:** all four items done in 531.8 s, 159,117 tokens and 50 tool calls. No Rust was opened or built. It repaired 6 failed checks: a
  renamed-fact expectation in `checks.sim`, a `fallback` warning answered with a deliberate silence, a script mistake, three wrong
  expectations of its own, and one line rewritten after a probe showed a brick would not carry into the next watch. It ended with 9 of 9
  scenarios passing.
- **Graded independently** by re-running `story check`, `story build --check` and the game's scenarios: confirmed. One sub-case is
  untestable in the map: Bram absent while the lamp is stocked, because the lamp zone lies inside the dock zone. That is a map fact, not an
  agent gap.
- **Friction it reported, and what was done:**
  - Fixed in this branch:
    - `story refs` called declared commands and events undeclared.
    - `story refs` could not see the scene's `checks.sim` or the folder's walk files; it now lists those mentions.
    - There was no way to assert a bound query without a game-specific `show` mapping; `reads.<query>` now exists.
    - The expectation operators and paths were undocumented.
    - `refs` missed a director's command.
  - Left as they are:
    - `scripts/dev start` writes its task record into the checkout's `out/` (gitignored, by design).
    - It recommended a rebuild because this worktree had uncommitted Rust changes (on a clean checkout it would not).
    - Running `tw:odile_helps` between watches gains little, because a watch begins with the brazier topped up. This is a Tidewatch game
      rule, now said in its bindings note.

## Evidence ledger (claim only what ran)

| level | status | evidence |
|---|---|---|
| content-valid | yes | `story check` clean for Tidewatch and the three exemplars (`the_exemplars_compile_cleanly`); `validate` covers packages, bindings and `.story` folders; schemas and the drift test |
| behaviourally verified on tested paths | yes, on those paths | `games/tidewatch/server/tests/story.rs` (19), `tests/story_runtime.rs` (9), `tools::story` unit tests (11), `sim::systems` (7), `talk` (2), `app::dialogue` (3), `games/tidewatch/tests/play.rs` (2); Tidewatch `checks.sim` (3); two walks (8 runs), exhaustive only over the choices at their `*` steps from their starts |
| native presentation inspected | partly | the real client's frames rendered offscreen and looked at (`games/tidewatch/out/*.png`: prompt, typing, choices, bark). Not done: a real window and a real gamepad (no device here), and Windows. |
| editorially reviewed | no | only the author's own pass through `story review`; no independent reader |
| human-playtested | no | nobody has played it |

What the tests prove, by requirement:

- Two distinct voices: the character sheets and the attributed lines.
- A gameplay-backed decision: `two_voices_and_a_decision_that_moves_real_peat`.
- A delayed callback: `refusal_and_delay_are_answers_with_their_own_aftermath`, and the dark first watch scenario.
- Knowledge-dependent dialogue, with a speaker withheld when they lack the information: `knowledge_gates_lines_and_odile_is_withheld_until_she_knows`
  and `what_bram_reveals_the_player_knows_and_what_he_claims_they_only_believe`.
- Save and resume: `a_save_mid_conversation_resumes_exactly_and_bad_saves_change_nothing`, `a_save_from_older_content_loads_by_id...`, and the client test.
- Interruption: `an_urgent_bark_suspends_a_conversation_which_then_resumes` and the runtime interruption test.
- A director that sometimes does nothing on purpose: `the_director_rests_when_tense_and_sometimes_deliberately_does_nothing`.
- Early completion: `early_completion_changes_what_bram_asks`.
- Absent speakers: `absent_speakers_cannot_be_talked_to_and_do_not_bark`.
- Stale choices, repeated input, duplicate effects: `a_stale_choice_is_checked_again...`, `a_repeated_or_late_answer_never_applies_twice`
  and `the_game_refusing_a_choice_s_command_undoes_the_whole_choice`.
- Malformed content: the toolchain tests, and `a_package_that_does_not_fit_the_game_is_refused...`.
- Replay: `a_replay_of_a_recorded_night_reproduces_the_story_exactly`.
- Input leakage: `the_world_does_not_pause_but_the_player_is_held_while_talking`, and the client test.

Nothing here says the writing is good or the game fun. That takes a blind human comparison, which was not run.

## Limits and what is deferred

- Story state is not replicated online; the story runs where its game's authority runs. The snapshot budget is untouched.
- There is no story host for a scene without a game crate.
- `story walk` replays every prefix from the start (the match is not cloned) and does not merge states. Its cost grows with the number of
  paths.
- The type of a `scene:` variable comes from its start value.
- Barks cannot read an event's value: only queries and facts.
- Graph editors, autonomous NPCs, runtime text generation, quests and a world director are deferred, as the brief asked.

Next, in order of evidence: an independent editorial review and a human playtest of Tidewatch; a real-window and real-pad check; a scene-only
story host, if agents keep asking for dialogue in rule-only scenes (the agent-loop trace's task a did).
