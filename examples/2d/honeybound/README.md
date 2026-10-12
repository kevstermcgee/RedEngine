# Honeybound

*Tobin and the Baron of Thorns*: a side-scrolling platformer in the spirit of the 1985 classics, with its own world and characters.

Baron Thornwick, a crowned stag beetle in a cloak of thorns, has carried off Queen Melli of the Great Hive and locked up her friends in his keeps.
Tobin, a young beekeeper in a straw sun hat and a striped honey jumper, sets off across eight worlds of four stages each (32 stages) to bring them home.

Every world has the classic shape: x-1 out in the open, x-2 underground, x-3 high up (treetops or bridges over nothing), x-4 a keep with the Baron,
where a rescued friend tells Tobin where he went next. The Queen waits in 8-4.

| World | |
|---|---|
| **1 Meadowlands** | Meadow Path, Rootways, Canopy, Thornkeep (hand-made) |
| **2 Duskwood** | Duskwood, Glimmer Caves, Highbridge, Ashen Thornkeep (hand-made) |
| **3 Autumn Hollow** ... **8 The Ashlands** | Autumn Hollow, Frostvale, Sunset Dunes, Starlit Meadow, Blossom Ridge, The Ashlands: generated from seeded segments by `gen.py`, harder each world |
| **Bonus rooms** | honey rooms down the hollow logs of 1-1 and 2-1 |

## Playing

```
red_engine2 play2d examples/2d/honeybound.game2d.json      # or: re2d examples/2d/honeybound.game2d.json
```

Arrows (or A/D) run, **Z** or Space jumps (hold it to jump higher), **X** or Shift held runs faster and, with a firebloom, throws seeds.
**Down** on a hollow log goes inside. **Esc** pauses. The best score is kept between runs.

- **Royal jelly** (from a honeycomb block) makes Tobin big: he can break bricks and survive one hit.
- **Firebloom** gives big Tobin burning seeds that bounce along the ground.
- **Starpetal** (hidden in a brick) makes him invincible for ten seconds.
- **Clover** is an extra life, and so are a hundred **honeydrops**.
- **Foes**: grubbles (stomp them), shellback snails (stomp, then kick the shell), wasplings, spikebugs and snapvines (don't stomp those), cinders and
  ember wheels in the keeps, and the Baron himself: pull the lever behind him, or burn him with seeds.
- Every stage has a timer, a midpoint you restart from after losing a life, and a beacon pole at the end: the higher you touch it, the more it scores.

## How it is made

Everything is data in one file, `../honeybound.game2d.json`, written by `build.py` from:

- `art.py`: every sprite as palette rows (characters by hand, tiles by small functions);
- `audio.py`: sound effects as synthesized voices, eight original music tracks as scores (melodies written bar by bar as scale degrees);
- `levels.py`: worlds 1 and 2, built with a few verbs (`ground`, `gap`, `log`, `stairs`, `foe`, `lift`, `firebar`, `castle`, ...);
- `gen.py`: worlds 3 to 8, segments (blocks, logs, pits, foe groups, pyramids, platforms, lifts, ember wheels, lava) picked by a seeded generator;
- `build.py`: prefabs, the tile legend, the rules (one variable, `mode`, carries the flow: title, card, play, lost, cleared, game over, rescue, end), the HUD,
  dialogue and scenarios.

```
python3 build.py          # write the game
python3 dev.py solve      # find a run through every stage and mechanic with `red_engine2 solve`, store it in solutions.json, rebuild
python3 dev.py            # build, validate, verify
```

`verify` replays one solved run per stage (enemies included), so it proves every stage can be finished; other scenarios prove stomping, growing,
seeds, star power, kicking a shell, the honey room, the midpoint, losing a life, game over and pausing. Change a level and run `dev.py solve` again:
an old run that no longer fits fails `verify`.
