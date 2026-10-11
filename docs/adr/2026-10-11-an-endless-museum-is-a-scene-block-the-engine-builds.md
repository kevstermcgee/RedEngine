# 2026-10-11. An endless museum is a scene block the engine builds around the player
Status: accepted
Summary: A `museum` block names a collection (themes, words, styles, masterpieces as data) and a painted-surface shader; the engine builds an endless grid of rooms with doors, art with plaques, a key hidden in each room (in a sculpture, a painting, or a painting inside a painting), and the worlds inside paintings.

## Context
Frameless (examples/frameless) is an endless art museum: rooms behind rooms for ever, each hung with paintings, tapestries, sculptures and
installations that have a name, an artist and a story, each room keeping a key in one of its pieces, and every painting a world one can step into
(with paintings inside it, a few worlds deep). None of it could be authored as a scene: a scene is a fixed list of objects uploaded once, and an
endless building has to be made as the player walks. The release pipeline packages only `re2`/`re2d` plus data (ADR 2026-10-11-a-game-draws-its-own-sky-with-a-checked-wgsl-function),
so a crate of the game's own could not ship either. The procedural world (`procgen`) showed the shape that works: pure functions of a seed, built
around the player, drawn by a layer of its own.

## Decision
- A scene's `museum` block (`museum::spec::parse_museum`): `collection` (a JSON file: art styles and sculpture materials by name in the shader's
  order, word lists, artist templates, media, origin-story templates, sixteen-or-so emotional themes with colours, palettes, favoured styles,
  sculpture families and music, and hand-written masterpieces), `art` (the game's painted-surface WGSL, ADR 2026-10-11-a-game-paints-surfaces-with-its-own-checked-wgsl-and),
  `seed` (a number or `"launch"`), `start_theme`. The collection is a content format of its own: typed in `content::museum`, schema
  `schemas/collection.schema.json`, detected by `validate`, held to its checker by `tests/content_drift.rs`.
- Everything else is the engine's, as pure functions of the seed (`museum::layout`, `museum::exhibit`): an endless grid of rooms (a binary-tree maze
  joins every room, a third of the other walls get doors too), wings of about three by three rooms sharing a theme, rooms of five sizes and heights
  with pillars, a partition or a skylight; pieces placed so no door is blocked; plaques written from the collection's templates (a deck per room so no
  story repeats in it, artists who recur within a wing, quoted titles, years as a label says them and as a story does); one key per room, hidden in a
  sculpture (touch it) or in a painting one to three worlds deep (`KeyPlan`); the worlds inside paintings (`Pocket`: the painting's style all around,
  mirror water, painted ground, dunes or stepping stones, more paintings on easels, pillars or in the air, the key, the way back). Eighteen sculpture
  families (`museum::sculpt`) move by `Motion`s that answer the viewer: a figure turns its head to watch, a bloom opens as one comes near, a heart beats
  faster, shards tremble.
- `museum::state::Museum` is a visit: the rooms around the player (built a room a frame), doors that swing open once either room's key is found,
  what the player looks at, the flight into a painting (the camera glides into the canvas, the picture floods the screen, the world opens around),
  progress (keys, rooms, paintings entered, deepest) saved as JSON. `museum::build` turns rooms, doorways and worlds into `Model`s (meshes, looks,
  colliders, lamps) that `museum_gpu::MuseumLayer` draws in both renderers; the live client (`re2`, `museum.rs`) feeds the visit the player, lights
  the scene with the nearest lamps, steers the music journey to the wing (ADR 2026-10-11-a-game-steers-its-music-journey), plays the museum's own
  synthesized sounds, and draws labels, titles, its pause menu and the map in the player's serif (ADR 2026-10-11-text-that-is-read-is-set-in-the-player-s-own-system).
- `settings.steps`: footsteps can be turned off (the museum's are faint and ring a little in the room).
- Tools: `red_engine2 museum SCENE` prints a room (plaques, key, note), draws a piece, the world inside any painting (`--inside N/I/J`), the plan
  (`--map`) or every style on one wall (`--styles`); scripts name `museum:N`, `museum:frame:N`, `museum:way-back`, `museum:key`, `museum:door:SIDE`
  for `approach`/`look_at`; `RE2_SEED` repeats a launch, `RE2_MUSEUM_FRESH` starts a new visit.

## Consequences
- A museum is one scene, one collection, two shaders and some scores; a second museum (a sculpture garden, a library of impossible books) is a new
  collection and shader, not new engine code, as long as it fits rooms-and-doors.
- The generator is engine code: changing how rooms are laid out changes every museum, and the seed-to-room mapping is part of a saved visit (a saved
  progress file names rooms by grid position). Tests pin the properties that matter (connectivity, doors agree from both sides, nothing blocks a
  doorway, a key at the end of every route, masterpieces hang once).
- Undo: remove the `museum` block from `schema`/`strict`, the `museum` module, `museum_gpu`, the client's `museum.rs` and the `museum` command.
