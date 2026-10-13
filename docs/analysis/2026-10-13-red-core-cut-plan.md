# What the headless server reaches, and the seven edges between it and red_core (2026-10-13)

Phase 1 item 3 of ADR 2026-10-13-the-next-phase ("done: the headless build, clippy and tests compile only `red_core` and its dependencies"); the rule for every
move is ADR 2026-10-13-red-core-is-one-crate-the-engine-and-its-games-share (a module moves only when everything it uses is there; an edge that points
outward is cut first, in its own pull request).

## Method

Main at `352e99c` (red_core steps 1 and 2 merged), with the edge reader of #123 (`tools::module_graph`: grouped and nested imports, `super::`/`self::`,
inline modules, each path resolved in the crate its file is in; comments, strings and test modules are not edges). On any later main, the same commands:

- `red_engine2 src deps --cycles`: the rings of modules that use one another.
- `red_engine2 src deps --core`: what red_core holds and what could move into it now.
- `red_engine2 src deps --reach bin::red_server --into tools,museum,<gated>,app,avatar,kart_camera,killcam,scene_pool,splitscreen,ui,uniforms,shadow`,
  where `<gated>` is every module `src/lib.rs` declares under `#[cfg(feature = "gfx")]` or `#[cfg(feature = "render")]`:
  `awk '/^#\[cfg\(feature = "(gfx|render)"\)\]/{g=1;next} /^#\[/{next} {if(g && $0 ~ /^pub mod /){sub(/^pub mod /,"");sub(/;.*/,"");print} g=0}' src/lib.rs | paste -sd,`.
  The root is the headless server binary, so "the server's tree" is what it compiles, not a list chosen by hand. The `--into` list is everything that must stay
  out of a headless core: what draws, the engine's tools, the museum (one game, Frameless), and the presentation modules that are not gated today.

## Results

| question | answer |
|---|---|
| red_core today | 29 modules, 7,634 lines |
| the largest ring | 56 modules, 36,867 lines: `schema`, the simulation (`sim::*`), `physics`, `collide`, `player`, `props`, `hit`, `net::protocol`, and the scene's blocks (`ambience`, `atmosphere`, `daycycle`, `hud_config`, `ui_config`, `style`, `museum`, `sfx`, `mixer`); a ring moves whole or after a cut |
| what `bin::red_server` reaches | 102 modules, 51,635 lines |
| edges from that tree into the `--into` set | **7** |
| what it reaches with those 7 cut | 95 modules, 50,779 lines: 26 of red_core's 29, the binary itself, and **68 to move (43,173 lines)**; with `uniforms` and `killcam` (below) 70 |

The rest of the engine (the client, the renderer, the tools, `content::validate`) reaches the server's tree, never the other way, except through these edges:

| edge | what it is | action |
|---|---|---|
| `bin::red_server -> tools::portmap` | `--upnp` keeps the router mapping alive | move `tools::portmap` into red_core as `net::portmap` (it uses only `net::upnp`); `tools::portmap` stays a re-export |
| `macros -> tools::font` | the sign macro asks the 5x7 font which characters it can draw | the glyph lookup and size move to `red_core::font_glyphs`, which already holds the glyphs; `tools::font` keeps the drawing |
| `atmosphere -> shader_check` | with `render`, a game's sky and art shaders are compiled (naga) when the scene loads | `shader_check` moves into red_core behind an optional `shader-check` feature (naga only, no GPU) that the engine's `render` turns on: the same check in the same builds |
| `characters -> uniforms`, `costumes -> uniforms` | team colours | none: `uniforms` uses nothing and moves with them (the `--into` list was too wide) |
| `net::client -> killcam` | the client records the last seconds of snapshots | none: `killcam` uses only `net::interp`, `net::protocol` and `sim::clock` and moves with them |
| `schema -> museum` | the scene holds a parsed `museum` block (`Arc<MuseumSpec>`) | the one design decision: a scene block defined by a crate above the core. The seam ADR 2026-10-13-the-next-phase item 5 needs to move Frameless out; it gets its own ADR |

The "to move" count is the reach without the crossings, less red_core's modules and `bin::*` (a script over `src deps --reach` and `src map`; the
module names of `crates/red_core/src` are their paths). After cut 1 (`tools::portmap` -> `net::portmap`, this pull request) the same command lists 6 edges.

## What this changes in the plan

Item 3 is four cuts and one seam, then the move of 70 modules (about 43,500 lines) in a few structural pull requests, each with a parity run: the ring of 56
moves in one, because it cannot be split without more cuts. The moved tree needs crates the core does not have yet (the QUIC transport's `quinn`, `rustls`,
`tokio`); they are headless and join red_core's dependencies with the move. Then `red_server` builds from red_core and its dependencies alone, which is the
item's measure, together with the `iterate` time of a one-line edit in `sim/`.

The earlier estimate in the red_core ADR (the core's modules reach 171 modules) and the 69-module ring I reported on 2026-10-13 came from edge readers that
missed grouped imports and `super::`; the numbers above replace them.
