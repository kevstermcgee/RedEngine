# 0019. Strict scene fields, a schema version, and one JSON envelope for every command
Status: accepted

## Context
The commonest AI mistake in a JSON scene is a plausible field that does nothing: `"pos"` for `"position"`, `"color"` on an
object instead of `material.color`, `"raduis"`. The parser ignored unknown keys ("a scene can carry its own notes"), so each
typo became an invisible bug that looked like an engine fault. Separately, the CLI printed human text (and, for a few
commands, an ad-hoc `--json`), so an agent or the MCP adapter had to scrape it.

## Decision
- **Unknown keys are errors** with the path and a fix: `crate_1.pos: unknown field — did you mean `position`?`. Every section has an
  allow-list (`src/strict.rs`): root, `meta`, `background`, `ambient`, `post`, `camera`, lights, `material`, poses, every object
  type including the `wall`/`fence` macros (openings, baseboard, gaps), prefab instances, `zones`, `spawns`, `portals`,
  `interest`, and every `checks` group (a misspelled group would otherwise mean a check silently never runs). A number field
  holding a string is an error too. Notes go in the **extension namespace**: keys starting `_`, `x-`, `x_`, plus `notes` and
  `$comment`, are always allowed and ignored. The allow-lists are cross-checked by tests against every example, recipe and
  catalogue asset, and `describe scene` is tested against the root list.
- **`schema_version`** (optional, default 1). A scene naming a newer version is refused with a message. An incompatible format
  change bumps it and documents the migration in SPEC "Strict fields and versioning".
- **One envelope.** The global `--json` flag wraps any command in `{schema, command, ok, exit, data, diagnostics, stderr}`.
  `data` is the command's stdout (parsed JSON when the command has a JSON form, else `{text}`); `diagnostics` are the
  `path: message` problems with a stable code (`unknown-field`, `wrong-type`, `missing-field`, `unknown-name`, ...) and a `fix`
  when the message carries one (`describe diagnostics`). The CLI's prints go through `tools::envelope` so capture needs no
  per-command work; the old per-command `--json` flags were removed in favour of the global one.
- `describe --brief` (about 1 KB) is the cheap first read; deeper topics are opt-in.

## Consequences
Adding a scene field means adding it to its allow-list (a test tells you which). Adding a diagnostic code means adding it to
`envelope::CODES` (a test covers `classify`). Codes come from message wording; if that proves brittle, move them to the error
sites. To undo strictness for one section, delete its `check_keys` call.
