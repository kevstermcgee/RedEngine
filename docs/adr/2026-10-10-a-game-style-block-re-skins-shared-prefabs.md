# 2026-10-10. A game style block re-skins shared prefabs
Status: accepted
Summary: A scene's style (palette roles, material presets, light rig, sky, fog, post) resolves at parse time into ordinary fields; catalogue params bind to palette roles; lint warns when a game keeps the default look.

## Context
Games made on the engine look alike. 3D assets are primitives with flat materials (ADR 0008 rules out binary import), the Idea Forge brief tells agents to
reuse the catalogue "before inventing anything", and nothing in the scene format carries a game-wide look: every prefab param default, the blueprint's
walls and floors (`#cfcfd6`, `FLOOR_COLORS`), its lamp (`#fff4e0`) and the recipes' sky (`#8fc7ff` / `#eef6ff`, copied by seven scenes) are the same in
every game. Measured in `assets/*.json`: 163 prefabs, 119 with `$` params, 1143 hard-coded hex colours. 2D games already look distinct because each draws
its own sprites; in 3D the cheapest path is to make the shared assets take the game's colours instead of making new geometry.

## Decision
- A `style` root block: `palette` (role -> `#rrggbb`, up to 16, 6-8 is the useful range), `materials` (named presets with a material's fields), `light`
  (`sun`, `ambient`, `lamp`), `sky` (`top`/`bottom` or `color`), `fog` (`color`, `density`) and `post`. `src/style.rs`.
- **It is resolved at parse time, before prefab expansion, into fields the scene already has**: `"@role"` strings anywhere a colour goes become the
  palette's hex (text sections such as `rules`, `ui`, `text` are left alone); `material: {"preset": ...}` merges a preset under the object's own fields;
  `light`/`sky`/`fog`/`post` fill the scene's `ambient`, sun light, `background`, `fog` and `post` only where the scene has none, and point lights left at a
  default lamp colour take the `lamp` colour. Every tool after the parser (lint, reach, render, the server) sees a plain scene: one model, no new runtime path.
- **Prefab params bind to roles**: a param spec may say `"palette": "wood"`; when the scene's palette names that role, it is the param's default (an
  instance's own value still wins). 86 params in the catalogue are bound to the 12 standard roles (`style::DEFAULT_PALETTE`: primary, secondary, accent,
  wood, metal, fabric, floor, wall, trim, foliage, stone, lamp), chosen by the param's name and description; a test keeps every binding on a standard role.
  The word is `palette`, not `role`: a prefab's `roles` already means its gameplay job (seating, cover).
- Blueprints: with `scene.style`, rooms use `@wall`, `@floor`, `@trim` for the roles the palette names, and the style's light rig and sky replace the
  blueprint's own (its sun keeps its shadow settings and takes the rig's colour).
- `fog` is a new scene field (the shader already fades toward `globals.fog`; only the day/night clock could set it before).
- Lint (games only: a scene with `rules` or `ui`): `style-palette` when there is no palette and at least half of the surfaces are default colours (a game
  that chose its own hex by hand, like Marcel, is not asked for a palette), or when every palette role is a default colour; `style-lamp` when every point
  light is the default lamp colour; `style-sky` when the sky is one of the default gradients and there is no `sky`/`clock`/`procgen`. Warnings, so a
  scene's `checks.lint.max_warnings` decides whether they fail `verify`.

## Consequences
- A shared prefab re-skins per game with no new geometry: the same room of catalogue furniture in the default and a styled palette is in
  docs/analysis/2026-10-10-visual-uniqueness.md.
- The two recipes and Drowsy Marathon now carry their own `style.sky` (their checks allow no warnings). A copy of a recipe keeps the recipe's sky; that is
  not the engine default, so it passes lint, and `style-report` is what measures closeness to anything shared.
- The 1143 hard-coded colours inside prefab objects do not follow a palette; binding more of them is catalogue work, one param at a time.
- Undo: drop the `crate::style::resolve` call in `schema::parse_scene_text` and the `palette` lookups in `prefabs::expand_instance`; the lint check is
  `check_style` in `tools/lint.rs`.
