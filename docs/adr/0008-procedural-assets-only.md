# 0008. Meshes and sounds are generated in code, nothing imported
Status: accepted

## Context
The engine is meant to be a small, auditable surface an AI can hold in context and edit without
binary tools. Imported models/textures/samples need external pipelines and can't be diffed or linted.

## Decision
Geometry is built from six primitives (`mesh.rs`, winding unit-tested), props/prefabs are compositions
of them, and audio is synthesized PCM (`audio.rs::synth_hit_clank`). `Audio::new()` returns `Option`
so a missing output device can never take the game down. `SPEC.md` "Known limits" is the contract.

## Consequences
- Everything is diffable JSON or Rust; contact sheets (`catalog --sheet`) can verify assets visually.
- Organic shapes are approximated (lathe/capsule rigs). If that becomes the bottleneck, revisit with
  a *new* ADR rather than quietly adding an importer.
