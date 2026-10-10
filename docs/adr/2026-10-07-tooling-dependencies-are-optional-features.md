# 2026-10-07. Tooling dependencies are optional features
Status: accepted
Summary: MCP (rmcp, schemars) and MP4 export (ffmpeg-sidecar) become the `mcp` and `video` cargo features (mcp stays in the default build, video leaves it); the minimal server drops 26 crates.

## Context
`rmcp`, `schemars` and `base64` were unconditional dependencies although only the `red_engine2 mcp` authoring adapter (src/cli/mcp.rs) used them, so the dedicated server and bot builds (`--no-default-features`) compiled an MCP SDK. `ffmpeg-sidecar` rode along in `gfx` although only `render` (MP4 export) used it; every normal graphics build paid for it.

## Decision
- Feature `mcp` = rmcp + schemars + base64, in `default`. The adapter and its test (`[[test]] mcp_server`, `required-features`) compile only with it; `red_engine2 mcp` in a build without it says which feature to enable. The native in-process MCP implementation is unchanged.
- Feature `video` = `render` + ffmpeg-sidecar, NOT in `default` or `gfx`. `src/video.rs` and `render_video` are `cfg(feature = "video")`; `render` without it returns `tools::NO_VIDEO`. PNG frames, tours, storyboards, ui-shot and every verification path are untouched. CI stage `video` lints the feature.
- The headless-dependency guards (`ci.sh` tree stage, `package` forbidden strings, preflight banned crates) now also name rmcp/schemars (and softbuffer, added by the native 2D player).

## Consequences
Measured with `cargo tree -p red_engine2 -e normal --prefix none | sort -u | wc -l`: minimal build (`--no-default-features`) 168 -> 142 crates (rmcp, schemars, futures-*, darling, chrono, uuid, tokio-util and others gone); `red_server` pulls none of rmcp/schemars/ffmpeg/wgpu/winit. The default build 279 -> 267 crates (ffmpeg-sidecar and its tree); `--features video` is back to 279. Anyone who needs MP4 builds with `--features video`. Undo: add `video` back to `gfx`'s list.
