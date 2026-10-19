# Honeybound

*Pip and the Heartcomb*: a side-scrolling platformer made for a controller.

On the last warm night of autumn, Baron Thornwick steals the **Heartcomb**, whose warmth keeps the Great Hive alive through winter. He breaks it into
eight shards, gives seven to his lieutenants and grows thorns over the sky so no bee can fly out. **Pip**, the smallest bee of the hive, cannot fly yet,
and that is why Pip is the only one who can go: on foot, across eight worlds, before the first snow freezes the hive.

| World | Stages | Boss (who holds a shard, and a friend from the hive) |
|---|---|---|
| 1 Meadowlands | Meadow Path, Rootways, Canopy, Thornkeep | **Grubmaw**, the larder grub: walks and belly-flops; land on his head |
| 2 Duskwood | Duskwood, Glimmer Caves, Highbridge, Ashen Thornkeep | **Madam Duskwing**, who hoards the light: drops dust, swoops, must rest on the ground |
| 3 Autumn Hollow | 4 stages | **Captain Burr**, the bramble weevil: rolls into a ball of prickles, then is dizzy |
| 4 Frostvale | 4 stages | **Rimeweaver**, the frost spider: icicles from the roof, then drops down and is slow |
| 5 Sunset Dunes | 4 stages | **Sandmaw**, the antlion: a mound that follows Pip, trembles, then bursts out |
| 6 Starlit Meadow | 4 stages | **Gloam**, the lantern thief: sparks from his perches, then sleepy |
| 7 Blossom Ridge | 4 stages | **Queen Vespa** of the thorn wasps: calls wasps, dives stinger first, gets stuck |
| 8 The Ashlands | 4 stages | **Baron Thornwick**: thorn seeds, a horns-down charge into the wall, thorns from the floor |

A bug by the road in each world tells Pip how to beat what is ahead (bump into them). Every boss has a tell before the move that hurts and a moment
after when it can be stomped. The game grows harder world by world (wider pits, more and faster foes, tougher bosses).

## Playing

```
red_engine2 play2d examples/2d/honeybound.game2d.json      # or: re2d examples/2d/honeybound.game2d.json
```

| Controller | Keyboard | |
|---|---|---|
| left stick / d-pad | arrows or WASD | move; down on a hollow stump goes inside |
| A | Space or Z | jump; **hold it while falling to flutter** |
| X (or B) | Shift or X | run; with a firebloom, throw pollen |
| Start | Esc | the pause menu: resume, **save game**, **save this song**, music on/off, fullscreen, quit to title, close game |
| | F or F11 | fullscreen |

The journey is saved at the start of every stage, at every midpoint and from the pause menu (stage, midpoint, lives, honey, Pip's form, shards);
the title offers to continue it. **Save this song** writes the track playing now to your Music folder as `Honeybound/<song title>.wav`; the pause menu
shows the song's name. Royal jelly grows Pip; a firebloom fills Pip with pollen to throw; a sunpetal is ten seconds of sun power; a clover (or a
hundred honeydrops) is a life.

## The soundtrack

Every song is original and grows from Pip's motif (1 3 5 6 5): *Honeybound* (title), *The Hive at Dusk* (the Queen's call), *Meadow Path*,
*Rootways* (underground), *Canopy Breeze* (high up), *Thornkeep* (the keeps), *Duskwood Lanterns*, *Falling Leaves*, *Frost on the Clover*,
*Sunset Dunes*, *Starlit Meadow*, *Blossom Ridge*, *Ashen Wind*, *The Lieutenant* (bosses), *Baron Thornwick* (the last boss), *Sunpetal Dash*,
*A Friend Set Free*, *Home Before the Snow* (the end).

## How it is made

Everything is data in one file, `../honeybound.game2d.json`, written by `build.py` from:

- `canvas.py`: a tiny pixel canvas (ellipses, shading, outlines) and a PNG sheet writer for looking at the art;
- `cast.py`, `bosses.py`, `art.py`: Pip, the hive and the road's bugs, the Baron's thorn-marked creatures, the eight bosses, tiles per world, the
  Bloom Gate, scenery, the parallax backdrops (`sky` + `backdrop` per level), HUD icons and pad buttons, and the game's icon;
- `audio.py`: sound effects as synthesized voices and the soundtrack as titled scores;
- `levels.py` (worlds 1-2, hand-made) and `gen.py` (worlds 3-8 from seeded segments, harder each world), and the boss dens at the end of each keep;
- `story.py`: the prologue, the road's lines, the rescues and the ending;
- `build.py`: prefabs, rules (the flow, the pause menu, saves, the eight boss state machines), the HUD and the scenarios.

```
python3 build.py          # write the game
python3 dev.py solve      # find a run through every stage and mechanic with `red_engine2 solve`, store it in solutions.json, rebuild
python3 dev.py            # build, validate, verify
```

`verify` replays one solved run per stage (bosses included), so it proves every stage can be finished; other scenarios prove the title menu and the
prologue, stomping, growing, pollen, sun power, shells, the honey room, the midpoint, the flutter, losing a life, game over, the pause menu, saving and
continuing, and the road's talk.
