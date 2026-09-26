# Core Asset Library

The library is a small, curated foundation plus a path for games to grow it. Treat asset names as
API identifiers and follow this order before adding anything:

1. **Reuse** — search by intent: `red_engine2 catalog "seat small"` or inspect `--manifest`.
2. **Modify** — place an existing asset with different parameters, scale, material, or make an
   `extends` variant in the game's own asset file.
3. **Generate** — compose primitives as a game-local prefab and record how it was made in `meta`.
4. **Import** — bring in a compatible JSON prefab from another project/pack only after checking its
   source and license. Binary model/texture import is not currently part of the engine (ADR 0008).

Game-local libraries are ordinary JSON files and are discoverable before promotion:

```text
red_engine2 catalog --library ../my-game/assets/stealth.json "hiding cover"
red_engine2 --json catalog --library ../my-game/assets/stealth.json locker_tall
```

Blueprint `prefab_files` embeds those definitions into built maps, so games remain self-contained.
Promote an asset into the narrowest matching shared pack only when it has proved broadly useful.
Promotion means copying the definition, choosing a stable generic name, filling in metadata, and
running `cargo test --release` plus `red_engine2 catalog <name> --sheet out/<name>.png`.

## Definition metadata

The existing `name`, `tags`, `desc`, `mount`, `collide`, `params`, `objects`, and `extends` fields
remain the executable prefab contract. Optional structured metadata lives under `meta`:

```json
{
  "name": "locker_tall",
  "tags": ["storage", "metal"],
  "desc": "A tall ventilated locker; faces +Z.",
  "meta": {
    "aliases": ["school locker"],
    "roles": ["storage", "cover"],
    "styles": ["institutional", "industrial"],
    "status": "experimental",
    "revision": 1,
    "license": "MIT",
    "origin": "modified",
    "provenance": "Adapted from cabinet_tall for a game-local school pack"
  },
  "objects": []
}
```

`status` is `stable`, `experimental`, or `deprecated`. `origin` is `authored`, `generated`,
`modified`, or `imported`. Built-in assets default to stable revision 1, MIT, and authored, so old
definitions stay valid. `catalog --json` returns the normalized record in discovery, placement,
geometry, physics, lineage, lifecycle, parameters, and instantiation groups (while retaining the
older flat keys for compatibility).

[`packs.json`](packs.json) is the machine-readable pack registry and promotion policy. Add a new
pack only when no current pack describes a real cluster of reusable assets.
