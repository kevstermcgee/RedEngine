# RedCraft

An endless world of blocks to mine, craft and build with, on Red Engine 2. Every world is new: plains, forests, birch woods, taiga, snowy
plains, deserts, mountains, oceans, rivers and caves down to the bedrock, with coal, iron, gold and diamonds below. Punch a tree, make planks
and a crafting table, a pickaxe, a furnace; build a shelter before the first night, when zombies and creepers come out of the dark.

```
cargo run -p redcraft --release -- games/redcraft/content/redcraft.json                 # the title screen: New World, Load World
cargo run -p redcraft --release -- games/redcraft/content/redcraft.json --fullscreen
```

## Controls

| | Keyboard and mouse | Gamepad |
|---|---|---|
| Walk, look | W A S D, the mouse | left stick, right stick |
| Jump (twice quickly to fly in Creative) | Space | A |
| Sneak (you will not walk off an edge) / fly down | Shift | B |
| Sprint | Ctrl (with W) | left stick click |
| Mine, hit | left button (hold) | right trigger |
| Place, use (table, furnace, chest, TNT with flint and steel), eat | right button | left trigger |
| Hotbar | 1-9, the wheel | bumpers |
| Inventory and crafting (the creative list in Creative) | E | X |
| Throw what is in hand (Ctrl: the whole stack) | Q | |
| Pick the aimed block into the hand (Creative) | middle button | |
| Debug lines (position, biome, light, chunks, time, seed) | F3 | |
| Pause menu: music, sound, render distance, auto-jump, fullscreen, save and quit | Esc | Start |
| Fullscreen | F or F11 | |

In the screens: left takes or puts a stack, right takes half or puts one, Shift-click sends a stack across (into the furnace's input or fuel,
into a chest, between the hotbar and the rest), Shift-click on a crafting result makes as many as you can carry.

## Recipes (as the original's)

Log -> 4 planks; 2 planks stacked -> 4 sticks; 2 x 2 planks -> crafting table; coal over a stick -> 4 torches; a ring of 8 cobblestone ->
furnace; a ring of 8 planks -> chest; pickaxe, axe, shovel and sword in wood, stone, iron and diamond; 2 x 2 sand -> sandstone; 2 x 2 stone ->
stone bricks; 9 ingots, diamonds or coal -> a block (and back); sand and coal in a checker -> TNT; iron and flint -> flint and steel.
The furnace smelts iron and gold ore, sand into glass, cobblestone into stone, clay into bricks, logs into coal and porkchops; coal, wood,
sticks and wooden tools burn.

## Worlds

Saved in the game's own folder (`worlds/<name>/`: `level.json` and `chunks.bin`, only the chunks you changed) every 30 seconds, when you pause
and when you quit. `REDCRAFT_WORLD=NAME` (with `REDCRAFT_SEED`, `REDCRAFT_MODE=creative` for a new one) starts straight in a world.

## For developers

- `redcraft shot content/redcraft.json OUT.png --at X,Y,Z [--ground] [--look YAW,PITCH] [--hour H] [--seed N] [--gallery]` draws what a player
  sees there (`REDCRAFT_VIEW` chunks); `--gallery` hangs every block in front of the eye.
- `redcraft map OUT.png --seed N` draws the biomes from above and prints their shares; `redcraft icons OUT.png` every item's picture.
- `redcraft content/redcraft.json --headless --script play.json --dump state.json` plays a script (`press: "key:give:oak_log:3"`, `key:mine`,
  `key:click:inv:0`, ...: `src/mode/script.rs`); the state is under `/redcraft`.
- `cargo test -p redcraft` (the end-to-end runs are `tests/play.rs`). Design: `docs/analysis/2026-10-19-redcraft-plan.md`; what the engine
  needed: `docs/analysis/2026-10-19-redcraft-feedback.md`, ADR 2026-10-19-a-block-game-rides-on-the-mode-and-layer-seams.
