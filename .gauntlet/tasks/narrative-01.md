# Gauntlet task narrative-01: revise a small, gameplay-connected story

A portable task for comparing engines (or versions of one) on narrative authoring. It names behaviour, not files, commands or formats: an
engine is free to meet it however its own tools allow. Grade it by the semantic outcomes below, checked by running the game (or its
headless equivalent), never by the agent's report.

## Starting game (any engine provides an equivalent)
A small night-watch game: the player carries fuel from a store to a signal fire that burns down over a timed watch; two watches a night. Two
characters: a keeper at the fire (terse, counts things) and a young man on a dock waiting for his father's boat (talkative, asks questions).
The game already has: a conversation in which the young man asks for two units of fuel for the dock lamp (the player may lend them, refuse,
promise to help later, or ask about the keeper); a callback in the keeper's second-watch intro that she says only if she knows about the
lending; a fact recording that the player lent fuel; a resource reserve the keeper can draw on (one game action moves one unit from it).

## The task
1. Add a reaction: when the player puts fuel in the dock lamp themselves, the young man reacts in his own voice, only if he is there.
2. Add a consequential scene: a conversation with the keeper, only between watches after a watch in which the light went out at least once,
   where the player can ask for or decline a unit from her reserve; asking moves real fuel through the game's own action; a later line
   acknowledges which they chose.
3. Rename the story fact for "the player lent fuel" (keep its meaning) and repair everything that depended on it.
4. Prove it: the content compiles and lints clean; automated play-throughs of the new scene and reaction pass through the real game; the
   existing ones still pass.

## Expected semantic outcomes (each checked by running the game)
- O1 Stocking the dock lamp by hand with the young man present produces his reaction once; without him present, nothing is said by him.
- O2 After a watch with darkness, between watches, the keeper's reserve scene is offered; after a clean watch it is not.
- O3 Asking moves exactly one unit from her reserve to the fire (the totals are conserved); declining moves nothing.
- O4 The next watch (or the next talk) says something that differs by the choice made.
- O5 The renamed fact is written by the lending choices and still gates the keeper's knowledge-dependent callback; nothing refers to the old
  name (content, scenarios, walk files).
- O6 No engine or game source was edited; no engine build was needed for any content edit.
- O7 Every pre-existing automated play-through passes.

## What to record
Wall time, tokens if measurable, tool calls, the number of failed checks and what each repair was, which files were read (and whether
engine source was needed), and every point of confusion in the tools or docs.

## Results
- 2026-10-15, Red Engine 2 at branch narrative-foundation (Tidewatch, `games/tidewatch`): fresh agent (Sonnet 5.5, n=1), 531.8 s, 159,117
  tokens, 50 tool calls; O1 partly (the "absent" case is unreachable in that map: the lamp is inside the dock area), O2-O7 met, graded by
  re-running its checks; record in `benches/history/agent-tasks.json` and `docs/analysis/2026-10-15-narrative-foundation.md`.
