# Frameless (the crate)

An endless museum on Red Engine 2, in a crate of its own since ADR 2026-10-14-a-game-ships-as-its-own-binary-the-client-is-a-library: the
`museum` scene block, the visit, its world layer and its game mode in the engine's first-person client are Frameless's, not the engine's. The game
itself (its scene, collection, shaders and scores) is in `content/`, which is what RedEngineGames publishes; `content/README.md` describes the
museum and ships with it.

```bash
cargo run -p frameless -- content/frameless.json                      # visit (every option of re2: --fullscreen, --headless --script, ...)
RE2_MUSEUM_FRESH=1 cargo run -p frameless -- content/frameless.json   # a new museum instead of the saved visit
cargo run -p frameless -- museum content/frameless.json               # the first room: its pieces, plaques, where its key is
cargo run -p frameless -- museum content/frameless.json --room 0,-1 --inside 1/2 --shot out/deep.png   # the world inside a painting inside a painting
cargo run -p frameless -- museum content/frameless.json --styles out/styles.png --t 6                  # every painting style on one wall
cargo run -p frameless -- museum content/frameless.json --map out/map.png --explore 3                  # the pause menu's plan of the museum
cargo test -p frameless                                                # the museum's tests, the collection's drift check, a headless visit
```

(Run from `games/frameless`; from the repository root, the scene is `games/frameless/content/frameless.json`.)

- `src/museum/`: the block (`spec`), the layout, the exhibits and their plaques, sculptures, the visit (`state`), its sounds and its map.
- `src/museum_gpu.rs`: the museum's world layer (`MuseumLayer`, live; `StillMuseum`, a picture's own visit).
- `src/mode.rs`: `MuseumMode`, the museum's game mode in the client (labels, pause menu, map, footsteps, saves).
- `src/tool.rs`: `frameless museum`; `src/content.rs`: the collection's typed definition.
- The engine's `validate` leaves `content/` to this crate (its block is Frameless's): `cargo test -p frameless` validates it with the block registered.
