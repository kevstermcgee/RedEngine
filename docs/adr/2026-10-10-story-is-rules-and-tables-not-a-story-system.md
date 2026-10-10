# 2026-10-10. Story is rules and tables, not a story system
Status: accepted
Summary: Games carry story with four small primitives (string tables + pick, random/floor/round/min/max/abs, a dialogue widget, recipes) on the existing rules; no character, quest or arc schema.

## Context
AI-made games should be able to carry a compelling story. The run that built Idea Forge (docs/analysis/2026-10-07-idea-forge-feedback.md) named the two
real blockers, and neither is a missing narrative system: **no randomness** (rules had no `random`; the only source was a position range used by `spawn`,
`play2d` replayed seed 1 on every launch, and there was no `floor`/`min`/`max`/`abs`, so an AI built a hidden "die" entity and floored with `x - x % 1`), and
**no string choice** (`text` interpolated numbers only, so showing phrase #7 of 32 took 32 widgets with `show: "v == 7"`: 231 widgets and 72 KB for 10 KB
of words). A third gap followed from the second: a conversation (lines in order, a choice that matters) had to be hand-rolled from widgets and counters.

## Decision
Four primitives, all data on the rules that already exist:
1. **String tables and `pick`** (2D): a root `tables` block (named lists of strings) and a text that is `{"pick": table, "by": expression}`: the entry at the
   expression's whole value, 0-based; past either end shows nothing; an entry may hold `{var}` placeholders. `validate` rejects an unknown table (with a
   did-you-mean) and a **constant** index outside the table. Text widgets take `width` and wrap at word boundaries.
2. **Numbers**: the shared expression language (`sim::rules_expr`, used by 2D and 3D rules) gains `floor round abs sqrt min max` and **`random(a, b)`**, a
   whole number from `a` to `b` inclusive drawn from the simulation's seeded generator through the `World` trait (`draw_int`, xorshift64*). Same seed and
   inputs, same draws, so `verify` scenarios are reproducible. A static evaluation (no simulation) reads `random` as its low end, and a widget's `show` does
   not draw (it would consume numbers every frame). **`play2d` and `re2d` pick a new seed each launch and print `seed N`**; `--seed N` replays that run.
   3D rules draw from `RulesEngine`'s generator, seeded the same for every match so the server, a replay and a scenario agree (`reseed` exists; nothing
   calls it yet).
3. **A `dialogue` widget** (2D): ordered lines with an optional speaker, text or `pick`, `emit` (when the player moves past) and `goto` (a line id or
   `"end"`); up to nine choices per line, each with `emit` and `goto`; the advance action (default `action`) moves on, up/down or 1-9 or a click picks;
   `done` is emitted when it closes. Its state is two **declared variables**, `<id>_line` and `<id>_choice`, so rules read them, `set` replays or skips a
   conversation, and `persist` can save it. While open it owns its keys, and with `pause` (default true) the world stands still (no movement, touching or
   timers; `time` still counts).
4. **Recipes** (`examples/patterns`, so `recipe` and `search` surface them): `random-pick`, `text-table`, `story-beat`, `branching-dialogue`, each a
   complete tiny game whose scenarios prove the mechanic (a fixed-seed draw, a wrapped table page, beats advanced by play, both branches of a choice).

## What was deliberately NOT built, and what would justify it
- **No character, quest, arc, inventory-of-clues or narrative schema** (no `characters: {...}`, no `quests: [...]`, no story graph editor, no
  conditions-on-lines language beyond `goto` and `show`). Story stays rules + tables + dialogue, all data that `validate` checks and scenarios prove.
- **No string variables**: text is chosen from tables by number; a name the player types, or text assembled at runtime, is not possible.
- **No per-match seed in 3D** and no random in 3D UI text (3D `ui` still interpolates numbers only).
- **The signal to build more**: agents repeatedly hand-rolling the same scaffolding across generated games. Concretely, if Idea Forge feedback or the
  generated games show the same structure three or more times (a "who said what" table of speakers, a quest log as N counters and N widgets, the same
  dialogue-state bookkeeping in rules), that pattern earns a primitive, the way tables and `dialogue` were earned here by finding 2.

## Consequences
- Idea Forge (finding 2) can be rewritten as one table per vocabulary and a handful of `pick` widgets instead of 231 widgets; a game no longer needs a
  generator script to unroll its words.
- `random` changes no existing game: nothing called it, and the generator is advanced only by a rule that does.
- A dialogue with `pause` makes press rules silent while it is open; a game that wants a bark that does not stop play sets `"pause": false` (then the
  dialogue still owns the advance and up/down presses).
