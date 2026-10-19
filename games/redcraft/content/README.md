# RedCraft

An endless world of blocks to mine, craft and build with. Every world is new: plains and forests, birch woods, taiga, snowy plains, deserts,
mountains, oceans and rivers, and caves all the way down to the bedrock, with coal, iron, gold and diamonds waiting below. Punch a tree, make
planks and a crafting table, then a pickaxe and a furnace, and build a shelter before the first night: zombies and creepers come out of the dark.

Choose **New World** on the title screen (Survival, or Creative to fly and build with every block), or **Load World** to go back to one.
Worlds are saved every 30 seconds, when you pause and when you quit.

## Controls

| | Keyboard and mouse | Gamepad |
|---|---|---|
| Walk, look | W A S D, the mouse | left stick, right stick |
| Jump (twice quickly to fly in Creative) | Space | A |
| Sneak (you won't walk off an edge) / fly down | Shift | B |
| Sprint | Ctrl (with W) | left stick click |
| Mine, hit | left button (hold) | right trigger |
| Place, use (crafting table, furnace, chest, TNT with flint and steel), eat | right button | left trigger |
| Hotbar | 1-9, mouse wheel | bumpers |
| Inventory and crafting | E | X |
| Throw what is in hand | Q | |
| Debug info (position, biome, light, time, seed) | F3 | |
| Pause: music, sound, render distance, auto-jump, save and quit | Esc | Start |
| Fullscreen | F or F11 | |

In the inventory: left-click takes or puts a stack, right-click takes half or puts one, Shift-click sends a stack across; Shift-click on a
crafting result makes as many as you can carry.

## First recipes

- A log makes 4 planks; 2 planks stacked make 4 sticks; 4 planks in a square make a crafting table.
- At the table: pickaxe (3 on top, 2 sticks down the middle), axe, shovel and sword, in wood, stone, iron and diamond.
- Coal over a stick makes 4 torches. A ring of 8 cobblestone makes a furnace, a ring of 8 planks a chest.
- The furnace smelts iron and gold ore into ingots, sand into glass, cobblestone into stone, and cooks porkchops; coal and wood burn in it.
- You need a stone pickaxe for iron, and an iron pickaxe for gold and diamonds.

`redcraft.json` is the scene; `shaders/blocks.wgsl` paints every block; `audio/` holds the music. The game itself is RedEngine's `games/redcraft`.
