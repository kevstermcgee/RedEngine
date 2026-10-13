# 2026-10-13. red_core is one crate the engine and its games share
Status: accepted
Summary: A headless red_core crate replaces the 2D crate's #[path] sharing; a module moves into it only when everything it uses is there, and the engine re-exports it at its old path

## Context
The 2D crate (`crates/red2d`) compiled ten of the engine's source files a second time through `#[path]` (the synthesizer and its analysis, the voice and
score formats, the rules' expression language, strict fields and did-you-mean, the font glyphs): their 58 inline tests ran twice, `dsp::Voice` existed as two
distinct types, and the 2D crate needed shims (`prefabs::suggest`, a test-only `sfx`) to satisfy the files' `crate::` paths. Phase 1 item 3 of ADR
2026-10-13-the-next-phase asks for a headless `red_core` that the engine, the 2D crate, `re2` and game crates depend on, so that the headless server's tree
builds and tests alone and a simulation edit stops relinking the whole engine. Measured at 80fbb71 with `src deps`: the core's own modules (schema, content, sim,
physics, net, collide, player) reach 171 modules once every outgoing edge is followed, because `schema` is a hub (it parses the museum block, audio specs,
the sky, the UI configuration) and `net::session` builds presentation (`ui::race`, `avatar`, `kart_camera`). So the core cannot move in one step.

## Decision
- `crates/red_core` is a workspace member and a default member; `red_engine2` and `red2d` depend on it. It mirrors the engine's paths (`red_core::dsp`,
  `red_core::sim::rules_expr`) and the engine re-exports each moved module at its old path (`pub use red_core::dsp;` in `src/lib.rs`, `pub use
  red_core::sim::rules_expr;` in `src/sim/mod.rs`), so `red_engine2::dsp::Voice` and every game crate keep compiling unchanged.
- **The rule for every later move:** a module moves into `red_core` only when everything it uses is already there. An edge that points outward (from a core
  module to presentation, tools or one game) is cut first, in its own pull request, as a behaviour-neutral change; the move is then a structural pull request with
  a parity run, never mixed with a behaviour change.
- The first move is the ten shared files: `red2d` drops every `#[path]` and both of its shims; the files call `crate::suggest::suggest` (the function
  `prefabs::suggest` re-exported); `audio_analysis::fft` became `pub` (the engine's `listen` uses it); the one test that needs the engine's sound catalogue moved
  to `src/sfx.rs`.
- Every dependency the crates share comes from the root's `[workspace.dependencies]` (`x = { workspace = true }`): one version and one feature set, so
  `cargo test -p red_core` compiles `rapier3d` with `enhanced-determinism` exactly as the engine does. The core's `sim` and `net` deny `unwrap`/`panic`
  outside tests like the engine's (nothing reachable from a packet may panic). A core test never reads an engine file (a map, a game's data): such a test
  is about that content and moves to the engine's suites.
- `red_core` never draws or plays sound: `preflight`'s headless check covers `crates/red_core/src` with nothing gated. The tools know it: `src map|show|deps`
  index it under the engine's module names (its `lib.rs` is `red_core`), `context` lists its files, and `affected` plans `red_core`'s clippy and tests for a change
  there, plus the 2D crate and the features that own the file.

## Consequences
One copy and one type for each shared module, the 58 tests compile and run once, and the 2D crate's shims are gone. The remaining outward edges of the core,
measured here, are the work list for the next moves: `net::host` and `bin/red_server` -> `tools::portmap`; `net::session` -> `ui::race`, `avatar`,
`kart_camera`, `characters`, `scene_pool`; `net::client` -> `killcam`; `content::validate` -> `tools::blueprint`, `tools::game`, `museum::spec`; `sim::scenario`
-> `tools::simrun` (tests); `schema` -> `museum`, `ambience`, `atmosphere`, `ui_config`, `hud_config`, `style`, `daycycle`, `procgen`. Utilities the core needs
anyway (`strict`, `geometry`, `expanse`, `hit`, `props`, `terrain`, `track`, `weapons`, `arsenal`, `crypto`, `color`, `easing`, `macros`, `prefabs`) move
before their users. Undo: move the files back under `src/`, restore the `#[path]` lines in `crates/red2d`, and drop the dependency.
