# Red Engine 2

JSON-scene 3D engine (Rust + wgpu) for a prop-hunt game. **Goal: you never read engine source.** The
engine describes itself and ships the tools to build/verify maps. Read @AGENTS.md for the workflow.

```bash
cargo build --release                      # once
R=./target/release/red_engine2
$R describe                                # overview + every command + topics
$R search "how do stairs connect floors"   # best doc/asset/lint/source fragments
$R catalog apple                           # assets (props + JSON prefabs); `--sheet out.png` to SEE them
$R recipe                                  # known-good maps; `recipe two_floor_house --new my.json`
$R lint  my.json && $R plan my.json && $R tour my.json out/tour.png   # check, then LOOK
$R verify my.json                          # the scene's own `checks` (lint/reach/walk/objects/golden views)
$R src find <words>                        # only if you must touch Rust: find/show/refs/deps, no file reads
$R describe glossary                       # vocabulary (prop vs prefab, zone, body band, "tire iron"...)
$R describe decisions                      # why it is built this way: docs/adr/ (search --kind adr)
cargo test --release                       # 95+ tests incl. every recipe, the catalogue, docs-vs-code checks
```

Map: `SPEC.md` scene language · `AGENTS.md` workflow + tool reference · `src/` engine (`$R src map`) ·
`assets/*.json` prefab catalogue · `recipes/` example maps · `examples/` demo scenes · `mcp_server.py` MCP wrapper ·
`docs/` glossary + ADRs.

When you change Rust: `//!` on new modules, `///` on pub items (`$R src coverage`), simulation logic as pure
functions (not in `App`), decisions as ADRs — see AGENTS.md "Keeping the codebase cheap for the next AI".
Multiplayer / a headless server is **not built** (ADR 0010).
