# 2026-10-13. A scene block can be defined above the core
Status: accepted
Summary: The scene parser keeps a registry of blocks that crates above the core define (key, parser, launch seed); the museum is the first, each binary that parses a scene registers the engine's, and an unregistered block is an unknown field, never ignored

## Context
`schema` parses every block of a scene, and `Scene` holds the result. One of those blocks belongs to one game: `museum` (Frameless, ADR
2026-10-11-an-endless-museum-is-a-scene-block-the-engine-builds), held as `Scene::museum: Option<Arc<MuseumSpec>>`. Measured in
docs/analysis/2026-10-13-red-core-cut-plan.md, `schema -> museum` is the one edge from the headless server's tree into a game that cannot be cut by
moving code: `schema` must move into red_core (ADR 2026-10-13-the-next-phase item 3), and the museum must not (item 5 moves Frameless to its own crate).
The block's parse needs the scene's asset directory and reports its errors with the scene's (`validate` lists them with the rest); its `seed` can be
`"launch"`, which `Scene::fix_launch_seeds` resolves with the others. Four places read it: the renderer, the viewer, `re2`'s museum client and the
scene's own launch seed.

Alternatives weighed: keeping the parsed block in the core under a feature (a game's format inside the core, and item 5 would undo it); keeping the
block as raw JSON and parsing it where it is read (its errors would move from load time to the first visit, and `validate` would stop seeing them);
a type parameter on `Scene` (every user of `Scene` changes); registration at link time (`inventory`/`ctor`: no such hook on wasm32, where the engine
also runs, and an unreferenced object file can be dropped by the linker).

## Decision
- The core's scene parser keeps a registry of **scene blocks** (`schema::BlockDef`): a key, a parse function (the scene root and the asset
  directory -> the value or the block's errors) and a launch-seed hook. `schema::define_blocks(&[...])` adds to it (a key already defined keeps its
  first definition). At load, each defined block is parsed where `museum` was, so its errors keep their place among the scene's; the value is kept in
  `Scene::blocks` (`schema::SceneBlocks`) as a `schema::BlockValue` (`Arc<dyn Any + Send + Sync>`) and read back by type:
  `scene.blocks.get::<MuseumSpec>()`, or `museum::spec_of(&scene)`.
- The defined keys join `strict::ROOT_KEYS` for the unknown-field check (`schema::root_keys`). **A block nobody registered is an unknown field**, with the usual
  did-you-mean: never silently ignored. (Registered keys come after the built-in ones; `museum` moves from the middle of the list to the end, and no
  key within two edits of it, where suggestions are made, sits after it, so no message changes.)
- The engine defines its blocks in one place (`scene_blocks::ENGINE`, today only the museum's) and registers them: every binary that parses a scene
  (`red_engine2`, `re2`, `red_server`, `red_bot`) in `main`, and
  the engine's own scene loading for everything else (tools, tests, the renderer) through `scene_blocks::parse_scene`/`parse_scene_in`, which register
  and then call `schema`'s. Code that will move into red_core calls `schema` directly; in a process it runs in, a binary registered first.
- The museum's readers ask `museum::spec_of(&scene)` (`museum::set_spec` gives a scene one); the block's parser, its errors and its value are the
  ones of today. `museum::BLOCK` is its definition.
- This is the seam a game crate uses (item 5): Frameless's binary registers `museum` itself, and the engine's list loses it.

## Consequences
`schema` no longer names a game, so it can move into red_core with the rest of the server's tree. A game can add a scene block without changing the
engine. The cost: a process must register before it parses (an unregistered block fails loudly as an unknown field, which is what a test or a new
binary that forgot sees), and a block's value is reached by type instead of a field. `describe scene` and the JSON Schema still list `museum`: they are
the engine's documentation of its own blocks (`content::scene`), unchanged. Undo: put `museum: Option<Arc<MuseumSpec>>` back on `Scene`, parse it in
`schema` again, drop the registry and the registration calls.
