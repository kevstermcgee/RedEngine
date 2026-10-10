# Idea Forge

A RedEngine 2D game that is also a utility: strike the anvil and it forges a video game idea built around **one strange gameplay mechanic**, with an optional story.

```
PHANTOM LANTERN
YOU   sing notes whose loudness physically shoves everything nearby,
BUT   the floor gives way on your third visit to any tile.
PUSHBACK  Every ten seconds the room swaps its rules with the room next door.
GOAL  Make the loudest sound the level is able to make.
STORY A cartographer mapping a country that dislikes being mapped.
```

| Key | |
|---|---|
| Space / Enter | forge the next idea (reels settle one by one) |
| 1-5 | lock YOU / BUT / PUSHBACK / GOAL / STORY; the next strike changes only the unlocked parts |
| S | story on/off |
| F / V | save the idea (a ring of 8) / browse the saved ones |
| B | back to the previous idea (8 deep) |
| M | music on/off |

Everything persists between sessions (counter, current idea, saved ideas, history, locks). The 10-digit **CODE** in the corner names an idea exactly: `python3 build.py decode CODE` prints it in full.

## Why no idea repeats

An idea is one integer in a mixed-radix space (title adjective x noun x YOU x BUT x PUSHBACK x GOAL x STORY = 24 x 24 x 32 x 32 x 24 x 24 x 24 = 8,153,726,976 ideas). A strike adds a constant step that is coprime to that size, so the walk visits every idea exactly once before any repeat; the starting point comes from the timing of your first strike and the pointer position (the engine has no wall clock, and `play2d` is seeded identically on every launch). Locking a part makes the walk of the *other* parts a remix, so a locked session can revisit a combination.

## Files

- `vocabulary.json`: the phrases (edit this to change what the forge can say; the part sizes may change, the build recomputes everything).
- `build.py`: expands the vocabulary into `../idea-forge.game2d.json` (the game cannot hold strings in variables, so every phrase is its own text widget shown while its digit matches). `python3 build.py check` lints the vocabulary against the 5x7 font, line counts and the full-period walk.
- The generated `../idea-forge.game2d.json` is committed (the engine and the tests read it); rebuild it after editing the vocabulary.

```
red_engine2 verify examples/2d/idea-forge.game2d.json     # six scenarios incl. "twelve strikes never repeat an idea"
red_engine2 play2d examples/2d/idea-forge.game2d.json
```

Development issues found while building it, and what the engine should do about them: `docs/analysis/2026-10-07-idea-forge-feedback.md`.

The same vocabulary feeds the CLI that runs the whole loop (idea, an AI builds the game, engine feedback): `scripts/idea_forge.py`, docs/IDEA_FORGE.md.
