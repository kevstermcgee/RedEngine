# Visual uniqueness for 3D games: measured before and after (2026-10-10)

Task B: make AI-made 3D games stop looking alike. Built: a game `style` block with palette roles that shared prefabs bind to (B1), lint warnings for the
default look (B2), `style-report` and `catalog --scene --local` (B3), the Idea Forge 3D brief floor and score items (B4), `voxels` in 3D scenes (B5).
Designed, not built: procedural materials and shape ops (B6, ADR 2026-10-10-procedural-json-materials-and-shape-ops).

## Why they looked alike (measured)

- The catalogue: 163 prefabs; 119 take `$` params; 1143 colours are hard-coded hex. Nothing tied a param to a game-wide colour, so every game that placed a
  chair got the same chair.
- The defaults every game inherits: the blueprint's walls (`#cfcfd6`, `#b8b8c0`), floors (six fixed greys) and lamp (`#fff4e0`, intensity 10); the recipes'
  sky (`#8fc7ff` / `#eef6ff`), copied by 7 scenes; `#fff4e0` / `#fff0d0` / white point lights in most maps (tally of recipes, examples and generated 3D games).
- The generated 3D games (`style-report`, before this change): Gentle Carousel: 84 placed, 1 shared, 83 hand-built boxes, no prefab of its own, no palette;
  Drowsy Marathon: 20 placed, all hand-built boxes, no palette, the recipe sky. They are alike less because of the catalogue than because they are flat
  boxes in default colours under a default sky: exactly what `style-palette` / `style-sky` and the brief's floor now catch.

## The same room, default look vs a style (shared prefabs only, no new geometry)

A room of 12 catalogue prefabs (two chairs, side table, dresser, two lamps, monstera, pillow, curtains, locker, vase, rug), read as a game:

| | default look | with `style` (11 roles, light rig, sky, fog) |
|---|---|---|
| shared instances re-skinned by the palette | 0 of 12 | 11 of 12 (the rug's params are not bound) |
| surfaces in a default colour | 68 of 84 (81%) | 20 of 84 (24%) |
| palette distance from the defaults | 0 | 152 (20 reads as a different colour) |
| lint | `style-palette`, `style-lamp`, `style-sky` | none |

The rendered pair was checked by eye: walls, floor, wood, fabric, stone, foliage and the lamp's light all take the palette, and the sky, sun and fog
change the mood; the remaining 24% are the hard-coded hex inside prefabs (drawer fronts, the rug).

## What each piece does

- **Palette roles** (12 standard roles, `describe style`): 86 catalogue params now bind to a role (wood 15, primary 19, stone 16, trim 11, fabric 9,
  accent 9, foliage 4, metal 3), chosen by name and description; a test keeps every binding on a standard role with a hex default. Food, toys and the
  contents of paintings stay unbound on purpose: an apple is red in every game.
- **Lint**: games only (a scene with `rules` or `ui`). `style-palette` without a palette fires only when at least half of the surfaces are default
  colours, so a game that picked its own hex by hand (Marcel) is not asked for a palette; `lantern_walk` (12 of 13 default) is. The two recipes and Drowsy
  Marathon, whose checks allow no warnings, got a sky of their own.
- **`style-report`**: shared vs own vs hand-built, re-skinned count, palette distance, default-coloured surfaces, voxel models, sound files, the
  defaults lint says were kept; `--sheet` (and `catalog --scene GAME --local --sheet`) draws the game's own prefabs in its own palette and presets.
- **Idea Forge**: the 3D brief's "before inventing anything" is replaced by a floor: a palette of 6-8 roles of the game's own colours with light and
  sky, at least 3 prefabs of its own, its own sounds, and a look at them with `catalog --local --sheet` and `style-report`. The score records four items
  (palette of its own, own prefabs, own sound files, no default look kept) from `style-report --json`; they are quality facts, not the build gate.
- **`voxels`**: pixel art pressed into 3D (rows + palette, `@role` colours), each colour merged into the fewest boxes, usable inside a game's own prefab:
  a mushroom (5 x 6 cells, depth 4), a character (8 x 10) and a voxel tree prefab placed twice rendered as intended.

## Not done, and why

- The 1143 hard-coded colours inside prefab objects do not follow a palette. Binding more is catalogue work, one param at a time; `style-report`'s
  "surfaces in a default colour" is the number to watch.
- No colour grading in `post` (saturation, tint, vignette): it would differentiate games further but is a shader change; the light rig, sky and fog
  already change the mood without one.
- Procedural materials and lathe/extrude/array-along-path are designed with a cost estimate (ADR 2026-10-10-procedural-json-materials-and-shape-ops),
  not built.
- Not measured yet: whether Idea Forge's next 3D games actually meet the floor. That needs the nightly runs after this merges; the score items will
  show it per game in the ledger.

## Next

Run the next week of Idea Forge 3D games through `style-report` and compare with Gentle Carousel and Drowsy Marathon (above). If they meet the palette
floor but stay flat boxes, build `array_path` and the four patterns (B6's order).

## Method

`red_engine2 style-report <game>` and `lint` on each scene; the showcase room is two generated scenes (default and styled) with `"rules": []` added so the
game-only checks apply; `frame` renders of both, inspected. Catalogue counts: a script over `assets/*.json`. The binding pass is reproducible from the
rules in this note (param name and description keywords; frames by prefab).
