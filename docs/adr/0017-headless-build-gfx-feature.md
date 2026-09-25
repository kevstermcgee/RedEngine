# 0017. The dedicated server builds without graphics: the `gfx` feature
Status: accepted (implements the plan in ADR 0010; makes ADR 0016's "not yet split" caveat obsolete)

## Context
`red_server` and `red_bot` need no window, GPU or audio, but the crate linked wgpu/winit/rodio/ffmpeg
unconditionally because two "renderer-free" things lived in renderer files: the static collision code in
`viewer.rs` and two geometry helpers (`trs`, `build_stairs_parts`) in `render.rs`. A bare Linux VPS could not
build the server without dev libraries it would never use, and nothing stopped a new `use wgpu` from creeping
into the simulation.

## Decision
One Cargo feature, no workspace split (a split would add a crate boundary for a build-time property a
feature already gives):

- `collide.rs` = static-world collision and ground queries (`Collider2D`, `GroundCandidates`, `resolve_collision`,
  interactables), extracted from `viewer.rs`. `geometry.rs` = `trs` + `build_stairs_parts`. `MAX_LIGHTS` lives in
  `schema`. Nothing headless imports `viewer`/`render` any more.
- Feature `gfx` (**default**) gates `gpu`, `render`, `viewer`, `overlay`, `menu`, `audio`, `video`, `mesh`, `revolver`
  and the `re2` binary (`required-features`), and the optional deps wgpu/winit/rodio/pollster/ffmpeg-sidecar.
- Without `gfx`, `red_engine2` (the analysis CLI: lint/reach/walk/plan/verify/edit/describe/search) still builds;
  `frame`/`tour`/`render`/`storyboard`, `catalog --sheet` and golden-view checks answer with
  `tools::NO_GFX` (a one-line, actionable message) instead. `verify` reports golden views as SKIPPED, not failed.
- Build the server: `cargo build --release --no-default-features --bin red_server --bin red_bot`
  (116 crates instead of 222; no wgpu/naga/winit/rodio/alsa).

## Enforcement (so it cannot rot)
- CI job `headless-linux` installs no audio/GPU libraries and runs `cargo tree` (no graphics crate may appear),
  builds the server, runs clippy and the **whole test suite** (including real-UDP server tests) with `--no-default-features`.
- `tests/headless_boundary.rs` fails locally if a non-gfx source file names a graphics/audio crate, or a gfx-only
  module loses its `#[cfg(feature = "gfx")]` in `lib.rs`.
- `src/net` and `src/sim` deny `clippy::unwrap_used/expect_used/panic` outside tests: nothing reachable from a UDP
  packet can panic the process (`tests/net_abuse.rs` throws garbage, floods and hostile values at the real server).

## Consequences
Adding a module that draws or plays sound: put it in the gfx list in `lib.rs` and `tests/headless_boundary.rs`.
Adding simulation logic: it must compile without `gfx` (the test suite runs both ways). To undo: make `gfx` deps
non-optional again — nothing else depends on the feature.
