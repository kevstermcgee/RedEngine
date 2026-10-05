# 2026-10-05. Every shader is validated and translated to HLSL in a test
Status: accepted
Summary: tests/shader_validation.rs parses, validates and translates every WGSL module to Direct3D HLSL with naga on any machine, and refuses derivative lookups inside loops, so a Windows-only shader error fails on a laptop instead of in hosted CI.

## Context
The cascaded sun shadows sampled a shadow map (`textureSampleCompare`) inside a loop that `continue`d and returned under per-pixel conditions. wgpu on Linux (Vulkan) accepted it; Direct3D's compiler (FXC) refused the pipeline with "gradient instruction used in a loop with varying iteration". Only the hosted Windows run saw it, a full CI round trip (about half an hour) after the work was "done", and merged it would have broken every Windows player.

We tried naga first, the compiler wgpu itself uses. Its validator (uniformity analysis included) and its HLSL writer both accept the broken shader: naga ignores `break`/`continue` when judging uniformity and treats every local variable as per-pixel, so it can neither flag this case nor tell a uniform loop from a varying one. There is no HLSL compiler on a Linux dev box.

## Decision
`tests/shader_validation.rs` composes every WGSL module as the pipeline builders do (`common.wgsl` first; `DEPTH_TEXTURE_TYPE` filled in), then for each: parses it, validates it with all validation flags, writes HLSL with naga's HLSL backend, and applies one structural rule: **no lookup that needs screen-space derivatives (`textureSample`, `textureSampleCompare`, `textureSampleBias`, `dpdx`/`dpdy`/`fwidth`, directly or through a called function) inside a loop**. Inside a loop use `textureSampleLevel` / `textureSampleCompareLevel`, or take the lookup out. A second test fails when a file in `src/shaders/` is in no checked module. `naga` is a dev-dependency (features `wgsl-in`, `hlsl-out`); it is already in the tree through wgpu, so `Cargo.lock` gains one edge and no crate.

The rule was verified against history: it fails on the shader as it was before the Windows fix (naming `cascade_lit`, `shadow_factor`, `fs_main`) and passes on the fixed one.

## Consequences
A shader that Direct3D's compiler would refuse for loop reasons now fails in `scripts/dev affected` on any machine. The rule is stricter than FXC: a derivative lookup in a loop with the same trip count for every pixel is also refused, because proving that needs a full uniformity analysis; the cost is using the Level variant there. The test cannot find other FXC-only errors (it is HLSL *generation*, not the Microsoft compiler), so Windows stays in the hosted CI matrix. To undo: delete the test and the dev-dependency.
