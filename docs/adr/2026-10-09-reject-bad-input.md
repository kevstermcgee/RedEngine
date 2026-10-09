# 2026-10-09. Bad input is rejected with the fix, never silently corrected
Status: accepted
Summary: A number out of its range, a value of the wrong type or a count over its limit in a scene, a rule, a scenario, a blueprint or a macro is a validation error in the id.field: message style that names the fix; the parsers no longer clamp or default it.

## Context
Authoring is done by AI agents that cannot see the effect of a value, so a parser that quietly corrects it costs them the most. Idea Forge runs found three: a negative `pad` on a rule volume was treated as 0
(changing -0.55 to -0.8 gave a byte-identical simulation and five minutes of searching), a scenario `hold: {forward: true}` ran for nine seconds with the player standing still (only `forward: 1` moved), and
errors that did not say the fix. Reading the parsers for the same pattern found it everywhere a number was `.max()`-ed, `.clamp()`-ed or `unwrap_or`-ed: material `opacity: 1.5` became 1, a stairs `steps: 200`
became 64, a wall `post_spacing: 0.1` became 0.5, a blueprint `count: 40` of spawns became 16, `"collide": "no"` left collision on, and a text where a number belonged became the default.

## Decision
Present-but-wrong is an error, absent is the default. In `schema.rs` (material, camera, stairs, lights, humanoids, jump pads, planes, post, meta), `sim/rules.rs` (volume `pad` and `height`, field `rate`,
impulse `speed`, rule `once`/`who`, ids that must be strings), `sim/scenario.rs` (`hold` axes, buttons, angles and weapon choice, expectation `tol`), `tools/blueprint.rs` (counts, numbers, flags) and `macros.rs`
(`wall`/`fence` options) every such value is checked where it is read and reported as `id.field: must be ... (got <value>); <the fix>`, the style the parsers already used (`secs`, `ranged`, `describe_value`).
The helpers are `ranged_fix`, `at_least`, `bool_field` (scene), `name_of`, `id_of` (rules), `hold_axis`/`hold_flag`/`tolerance` (scenarios), `number`, `flag`, `count_of` (blueprints) and `check_macro`.
One value is still adjusted, on purpose and in the documentation: a `roughness` below 0.04 draws as 0.04, because a perfect mirror has no highlight to shade (SPEC.md, materials).

## Consequences
A scene, rule or blueprint with a value that used to be corrected now fails `validate` and says what to write. None of the 90 JSON files in this repository (examples, recipes, assets, fixtures) had one; a
game project outside it may, and gets a message with the fix instead of a changed game. The check is per field, so a new field needs the same treatment: use the helpers above, never `.max()`/`.clamp()`/`unwrap_or` on
a value the author wrote. Undo: none needed; relaxing a rule is deleting its check.
