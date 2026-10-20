# Snake

Classic Snake with unlockable portals and backgrounds. `../snake.game2d.json` is generated: edit `build.js`, then run

```sh
node examples/2d/snake/build.js examples/2d/snake.game2d.json
```

## Unlocks (saved between runs)

| Unlock | Requirement |
|---|---|
| Side Portals (left/right wrap) | Reach length 25 in one game |
| Sky Portals (top/bottom wrap) | 15 side-portal trips in one game |
| Classic background | Starter |
| Garden background | High score 150 |
| Night Sky background | High score 450 |
| Checker background | High score 1000 |

Each background is a full theme: board tiles, snake and apple. Switching destroys `tag:bg`, spawns the theme's tiles and `become`s every snake part and the apple into that theme's prefabs. Background unlocks are also rebuilt from the saved high score on `loaded`.

Controls: arrows move, Space starts/restarts, P or Escape pauses, U opens the unlocks directory (1-6 or click to use), M toggles music.
