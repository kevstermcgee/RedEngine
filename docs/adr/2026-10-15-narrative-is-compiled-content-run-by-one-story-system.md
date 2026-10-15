# 2026-10-15. Narrative is compiled content run by one story system
Status: accepted
Summary: Stories are compact .story text compiled by a content build into an indexed package that the engine's Story system runs inside a game's Systems host under the match's one authority: typed queries in, validated command batches out, saves and checksums through the host; no second simulation, VM or rules engine.

## Context
The owner's brief (2026-10-14) asks for a lean, AI-first narrative foundation: characters, contextual dialogue, consequential scenes and small
emergent patterns that behave correctly in the real game, cheap for a fresh agent to author and revise, with the intelligence in the format,
the linter, the exemplars and the guide, and a bounded runtime. ADR 2026-10-10-story-is-rules-and-tables-not-a-story-system had built four
primitives (string tables, `random`, a 2D `dialogue` widget, recipes) and said what would justify more. What existed at main 8581197:

- The 2D widget's progression (goto, emits, `done`) was written inline with its key handling (`red2d::sim::dialogue_input`); 3D had no
  dialogue at all ("a 3D scene has no dialogue widget", agent-loop trace a #11).
- The game-systems host (ADR 2026-10-14-game-systems-compose-under-one-match-authority) already gave a game one authority, versioned saves
  with migration and rollback, the match checksum, `sim --inspect` and scenario `game` expectations, but a system could not *read* another
  (no queries), and nothing applied several systems' commands as one transaction.
- Scenarios could walk and press, not talk; traces recorded inputs, not decisions; `validate` re-parsed a game block's scene without the
  scene's folder (Workshop's own `validate` failed for that reason).

Considered for the authoring frontend: **Ink** and **Yarn Spinner**. Both are mature and well documented, and both bring their own runtime: a
flow VM, an expression language, a variable store and a save format. Integrating either means a second evaluator next to `rules_expr`, a
second state outside the host (its variables would mirror gameplay or call out to it), bridging its saves into the host's versioned ones and
its randomness into the host's stream, and a dependency in every shipped game: the brief's "no second authority, no general VM, no mirrored
variables". Their syntax is also the easy part to borrow. So: a small Red frontend that lowers into the rules' own expressions and the
host's own commands; Ink and Yarn syntax is refused by name (`story check`), never half-understood.

## Decision
- **Three layers.** *Authoring*: `.story` text (character sheets, facts, infos, setups, conversations of nodes and lines, barks, notices,
  directors; `docs/STORY.md` is the contract). *Toolchain* (`red_engine2 story check|build|refs|ledger|review|walk`, `tools::story`): parse
  with `file:line` on everything, bind every name to the game's bindings file (generated from its systems, freshness-tested) and the scene's
  variables, type-check (a condition is a yes/no; an int fact takes a whole number), lint, and write an immutable indexed package
  (`<story>.story.json`). Editing dialogue is a content build (milliseconds), never a Rust build. *Runtime* (`sim::story`, red_core): the
  package's `check` and `bind` run again at load, the same functions the toolchain ran, so they cannot disagree; expressions are compiled by
  `rules_expr`, the one evaluator.
- **One authority.** The story is a `System` (`<story>:story`) a game composes into its `Systems`; there is no second `MatchGame`. The host
  gains, with defaults that change nothing for existing games: typed **queries** (`System::queries/query/reads`, `Cx::query`, pure; `scene:<var>`
  answered by the match through `Outside`, `Systems::tick_with`), and **batches** (`Cx::issue`: commands to other systems applied after the
  issuer's call, in order, whole or not at all, with every touched system, the ledger, the random stream and the events put back on the first
  refusal; `System::settled` tells the issuer). A director may run only commands the game marks `unattended`.
- **Clients send intentions** (`MatchGame::intend`, `MatchSim::intend`, recorded in traces as `Entry::Intent` and replayed): `story:talk`,
  `advance`, `choose`, `leave`, `skip`, `new_run`, each naming the revision it was shown. A stale or repeated answer is refused; a choice is
  checked again when picked; its commands apply first as one batch, the story's own effects only if they did, else its `else` route.
- **One meaning of moving on.** `sim::story::talk::answer` is the progression core; the 2D widget now calls it (identical behaviour, its
  content and tests unchanged), and the story runtime adds conditions, effects and knowledge around it.
- **Contextual lines are rule tables over live state.** Candidates are filtered by pure conditions, ordered by priority, specificity, least
  heard, written order; a real tie draws once from the host's seeded stream when committed. Bark, speaker (by priority) and category
  cooldowns; a queue of three that goes stale after 4 s; higher priority interrupts (seen, not heard); `silence` is a chosen nothing; a
  `notice` records what witnesses see without speaking. A director evaluates every so often, never during a conversation, within recovery,
  budget and groups; `rest if` and `chance` make it do nothing on purpose; exhaustion is reported. Every decision is in the story's log with
  why (`sim --inspect story`).
- **Knowledge is explicit.** Infos may have a world truth; a line `reveals` (its speaker must be able to know it: checked) or `claims` (belief
  only); a reveal of something untrue is only believed; `learn`/`believe` effects; provenance kept per holder. Facts are narrative-only
  state with a scope (round, run, profile); gameplay is never a fact, it is a query.
- **Saves by id** through the host's versioned save: a newer content version or another story is refused, an older one loads by id with what
  no longer exists reported.
- **Verification**: scenario story steps (`{"story": "talk|advance|until_choice|choose|leave|skip"}`), and `story walk`, which replays every
  choice prefix through the real match within bounds and reports coverage, held-back choices with why, and per-branch consequences.
- **Presentation is optional** (`app::dialogue::Presenter`: reveal then advance, wrap-around focus, 1-9, clicks, gamepad, no input leaking
  into play), drawn into the game's own HUD with the audited UI kit. **Tidewatch** (`games/tidewatch`) is the reference: a native 3D game.

## What is deliberately not built
No graph editor, autonomous NPCs, runtime text generation, quest or inventory systems, string variables, localization tables, online
replication of story state (a story runs where its game's authority runs; the snapshot budget is untouched), story state migrations beyond
by-id loading, a scene-only (no game crate) story host, or a world director. A story's namespace must differ from its game systems'.

## Consequences
ADR 2026-10-10-story-is-rules-and-tables-not-a-story-system is superseded for games that have a game crate; its 2D primitives (tables,
`pick`, `random`, the widget) stand. Measured costs, limits and the evidence ledger are in
`docs/analysis/2026-10-15-narrative-foundation.md`. Undo: every engine hook defaults to nothing; the story is a system a game may omit.
