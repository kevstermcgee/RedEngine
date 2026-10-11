# 2026-10-11. One validate for every content file
Status: accepted
Summary: validate with no paths, a folder or several files checks every content file through its own checker and prints file:line errors (whole checkout in ~0.2 s, CI's fast tier); schema writes a JSON Schema per format to schemas/ from the typed definitions.

## Context
Phase 3 of the content-formats work (docs/content-formats.md; ADR 2026-10-11-content-formats-are-typed-and-checked-twice is phase 2). `validate` took one
scene or 2D game; the other formats (prefab libraries, blueprints, scores, playscripts, `game.json`, the pack registry, the driver roster, traces) had no
command at all, errors carried a JSON path but no line, and nothing machine-readable described any format to an editor or another tool. The owner asked for
`schema` writing JSON Schema to `schemas/`, `validate [paths]` over every content file with cross-reference checks and file:line errors in under 10 s, and
validate as a fast CI tier before the full suite.

## Decision
- **`validate [paths]`** (`content::validate`): no paths = the checkout's content folders (`examples`, `recipes`, `assets`, `tests/fixtures`, `games`); a
  folder = every `.json` under it; several files = those. Each file's format is detected from its name and top-level keys (`detect`), and it is checked by
  that format's own loader, so the messages are the hand-written ones and the cross-references are the ones the loaders already resolve: prefab names,
  rule/objective/UI variable and zone names, weapons, characters, a blueprint's prefab files, a project's maps. One that no loader checked is added:
  a `checks.sim` scenario's player `spawn` and `spawn_group` must name a spawn the scene has (it used to fail only inside `verify`). Every problem is printed
  as `file:line: message`; the line comes from the message's leading path (an index path, an object id, or `rules[3] (name)`) located in the file's text,
  cut back to the deepest part that exists. Files are checked on all cores. **One file keeps its old report** (`OK: scene is valid`, the 2D summary,
  `error: path: message`), because the JSON envelope's diagnostics, the MCP `validate` tool and their tests read that form.
- **`schema [--out schemas]`** (`content::json_schema`, `schema` feature): one draft 2020-12 schema per format, from the typed definitions via `schemars`,
  with the extension namespace (`x-*`, `x_*`, `_*`, `notes`, `$comment`) added as `patternProperties` to every object that refuses unknown keys. `schemas/`
  is committed; a unit test fails when it is not exactly what the definitions generate.
- **CI**: `scripts/ci.sh content` (in the default list, after the seconds-long checks) and a hosted `content` job that builds only the CLI from the test
  shards' cache and runs it. It runs beside the suite, not in front of it: making the shards wait for it would add its build time (~2-3 min) to every
  run that #97 brought to 4.6 min, for a result that arrives first anyway. Adding `content` to the `test` job's `needs` makes it a gate.

## Consequences
- Measured: the whole checkout is 75 content files (scene 27, 2D game 24, prefab library 11, score 8, pack registry, blueprint, playscript, drivers,
  trace), **0 problems, 0.2 s** (debug build, 4 cores); `tests/content_validate.rs` holds the 10 s budget. Every repository file also passes its generated
  schema under an independent validator (Python `jsonschema`, draft 2020-12), and the schemas refuse an injected unknown key.
- Known limits: a scenario's expected `event` names are not checked (the engine raises events beyond `ENGINE_EVENTS`, such as `interact` and `explosion`,
  and one by a computed name, so a check would refuse valid files); voices are checked inside their 2D games and scores, not as files.
- A new content format needs a typed definition, a place in the drift test, a line in `json_schema::all` and a `detect` rule (CLAUDE.md says so).
- Undo: the multi-file branch of `validate` and the `schema` command are separate from the one-file path; drop them and the `content` stage.
