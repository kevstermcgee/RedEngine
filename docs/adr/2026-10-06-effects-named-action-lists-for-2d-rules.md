# 2026-10-06. Effects: named action lists for 2D rules
Status: accepted
Summary: 2D games name a list of actions once under effects and apply it from any rule, expanded at parse time so the simulation, hashes and wasm player are unchanged.

## Context
A 2D game that gives the same behaviour several sources (a pickup and an ability both heal; six buildings all charge, spawn, sound and clear the same way) had to copy the action list into every rule. `examples/2d/tiny-station.game2d.json` carried six near-identical `build X` rules and the same pause toggle twice (97 actions written). Copies drift (change the sound in five of six), and a less capable model that adds a seventh source has to find and match all the others. The rules language had prefabs for things and nothing for behaviour.

## Decision
`effects` (a root key) names a list of actions with optional `params` (a list of required names, or name -> default). Any rule or button applies one with `{"apply": "heal", "with": {"amount": 2}}`; `$amount` inside the effect is replaced by the argument (a whole-string `"$cost"` keeps its type, a fragment such as `"-$cost"` is text). Effects may apply effects, 4 deep, no cycles.

It is **parse-time expansion** (`red2d::effects`, called from `parse_actions` in `game.rs`, the same idea as ADR 0006): the `GameDef` the simulation, the CPU renderer and the wasm player see contains only ordinary actions, so there is no new runtime, no change to the replay hash and no change to the web ABI. The consequence for compatibility is that every existing game parses unchanged (`effects` is optional) and tiny-station's seven scenario hashes are identical before and after the rewrite.

Diagnostics are part of the decision. Every error found inside an expanded effect is reported at the use site and the effect line (`rules[1] (grab medkit).do[0] -> effects.heal.do[0].add: no variable `livez` - did you mean `lives`?`); an unknown effect, a missing or unknown parameter, a `$typo` and a cycle each say what exists; an effect nobody applies is an error (`effects.heal: never applied`), so dead copies cannot accumulate. Separately, a failed scenario now ends with the rules that never fired and the ones that did (`Sim::rule_fires`), the first thing to look at when a variable did not move.

## Consequences
Easier: one place to change a behaviour; a fresh model adds a source with one `apply` line; `recipe shared-effect` is the 40-line example and `examples/2d/medic-run.game2d.json` the playable game (item + ability on one `heal`). Harder: a reader must follow an `apply` to see what a rule does (the error paths make that mechanical); effects are not parameterised by expressions beyond string substitution. Undo: remove `effects` and inline the actions; nothing else depends on it. Not done: effects for `scenario` scripts and UI `show` expressions.
