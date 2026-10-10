# 2026-10-10. Windows CI on pull requests is build and Direct3D
Status: accepted
Summary: On a pull request the Windows job builds and links every binary and compiles the renderer's pipelines through Direct3D; the full Windows suite runs on pushes to main, because Windows is authoritative only for Direct3D and the Python tools.

## Context
The Windows leg of the `test` job ran the whole suite on every engine pull request and set the pull request's wall-clock: median 19.0 min over 43 runs with
the full matrix (Linux 9.3, headless 8.1; docs/analysis/2026-10-10-ci-wall-clock.md). In run 37962855231 it spent 348 s compiling the tests, 278 s running the
parallel group (`split_render` alone 143 s and `procgen_render` 46 s, both on a software adapter), 180 s on the real-time network suites one test at a time,
and 141 s on the external client. Almost all of that repeats, more slowly, what the Linux job proves: the simulation, the network suites and the tools behave
the same on both platforms, and nothing in this repository's history failed only on Windows except two classes of fault:
- **Direct3D**: wgpu compiles WGSL to HLSL and hands it to Microsoft's compiler (FXC/DXC), which refused a shadow lookup that Vulkan accepted
  (`tests/shader_validation.rs` documents it). Only a real Direct3D device on Windows runs that compiler.
- **The Python tools**: paths, quoting, subprocesses and process trees (`idea_forge.py`, `launchpad.py`, `proc_supervisor.py`), which the `python-tools`
  job already covers on Windows whenever they change (ADR 2026-10-09-ci-runs-the-python-tools-on-windows-when-they-change).
`shader_validation` itself runs naga, which behaves the same on every OS: on Windows it proves nothing Linux does not. What reaches Microsoft's compiler is
a test that creates a device: `shadow_render` builds the `Renderer` (the world pipelines of `gpu.rs`, post and the ocean pass: 8 of the 11 pipeline
creation sites) on the hosted runner's WARP adapter in about 2 s.

## Decision
- On a pull request the Windows job runs `scripts/ci.sh direct3d`: `cargo test --test shader_validation --test shadow_render`. Building integration tests
  makes cargo build and link every binary (they need `CARGO_BIN_EXE_*`), so this is also "everything builds and links on Windows", with one engine compile
  (a separate `cargo build --bins` first compiles it again: dev-dependencies change the feature set). `RED_DIRECT3D_REQUIRED=1` fails the stage if
  `shadow_render` found no adapter and skipped, so a skip is never counted as a Direct3D pass.
- On a push to main the Windows job runs the full suite, the same stages as before (`lockfiles clippy tests benches`, `external-client`, `killchain`).
- The job is no longer a leg of the Linux `test` matrix but its own job, `windows (build + Direct3D)` / `windows (full suite)`.

## Consequences
- A pull request no longer waits for the Windows suite; the numbers are in the analysis note.
- A Windows-only failure outside Direct3D and the Python tools (a `cfg(windows)` path in `src/clipboard.rs`, `src/tools/pe.rs`, `package.rs`, the server's
  socket options) is found by the push to main, not the pull request. So are Direct3D faults in the three pipelines `shadow_render` does not build
  (`fx.rs`, `overlay.rs`, `split_gpu.rs`; `split_render` covers the last on main). If that ever costs a broken main, add the test that would have caught it
  to `stage_direct3d`, not the whole suite back.
- Clippy does not run on Windows for a pull request: lints are platform-independent except in `cfg(windows)` code, which main catches.
- Undo: in `.github/workflows/ci.yml`, drop the `if: github.event_name ...` conditions on the Windows job's steps.
