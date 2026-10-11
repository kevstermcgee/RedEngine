# 2026-10-11. Content formats are typed and checked twice
Status: accepted
Summary: Every content format has a typed deny_unknown_fields definition in content/ beside its hand-written checker; a CI test fails when the two disagree about any repository file or key.

## Context
The survey (docs/content-formats.md) measured that nearly every content format already refuses an unknown key with its path and a did-you-mean,
through about 25 hand-written checkers over `serde_json::Value`, but none is typed or machine-describable, and a few places really did drop bad keys
silently. Replacing the checkers with derived structs would have regressed their messages (serde gives no did-you-mean) for code that was already
strict, so the owner chose: typed definitions for the schema, the checkers kept for the messages, and a guard between the two.

## Decision
- **`src/content/`** holds one typed definition per format: scene (`scene`, `objects`, `checks`, `prefab`), prefab library, pack registry, 2D game,
  blueprint, playscript, score, voice, `game.json`, kart drivers, trace. Every struct is `#[serde(deny_unknown_fields)]`, every field `Option<T>`
  (required-ness and ranges stay the checkers' job), declared with `strict_struct!`; `type`-discriminated objects are `#[serde(tag = "type")]` enums,
  single-key actions are externally tagged enums. `JsonSchema` is derived behind the new default `schema` feature (`schemars`), so the headless build
  keeps its rule that no MCP/schema crate is linked. `content::typed` strips the extension namespace (as the checkers do) and reports the JSON path to a
  bad field (`serde_path_to_error`).
- **The checkers stay the source of every error message.** Nothing that loads content switched to the typed definitions, except the pack registry,
  which had no checker at all.
- **The drift guard, `tests/content_drift.rs`**: (1) every content file in the repository passes both definitions; (2) an unknown key injected at every
  distinct object path of those files is refused by both or accepted by both (accepted = a map of the author's own names, like `vars`). Any
  disagreement fails CI with the format, the file and the path. Two places where the checker refuses a new name because of a cross-reference, not a key
  set (a prefab instance's `params`, a 2D `apply`'s `with`), are listed with their reason; traces are written by the engine and read leniently, so their
  files must pass both but keys are not probed. `tests/fixtures/content/` adds a scene that uses every section, a playscript with every step and a
  blueprint with every key, because no repository file uses some of them; `new-game` scaffolds of each kind supply `game.json`, blueprints and maps.
- The gaps the survey found are fixed in the checkers: prefab definition and param-spec keys, keyframed tracks, `validate` reading `checks` (shape rules
  and the scripted playthroughs, parsed not played), `catalog` failing on a catalogue error. Building the guard found three more, also fixed: unknown keys
  inside `style.light.sun/ambient/lamp`, inside a text's `backing`, and 2D `map.legend` entries no row uses (never looked at).

## Consequences
- A new key must be added to both definitions; the guard says which one is missing it. A new content format needs a typed definition, a place in the
  drift test and (Phase 3) a schema; CLAUDE.md says so.
- The typed side is looser than the checker on values (an `Any` leaf, every field optional): it pins keys and shapes, not ranges or references. The
  guard probes keys, not values.
- Undo: the guard is one test file; the typed definitions are only read by it (and, in Phase 3, by `schema`).
