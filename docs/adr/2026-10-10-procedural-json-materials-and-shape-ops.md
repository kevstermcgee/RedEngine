# 2026-10-10. Procedural JSON materials and shape ops (design)
Status: proposed
Summary: Designed, not built: per-object procedural patterns (stripes, checker, noise, gradient, sprite decal) in the existing object uniform, and lathe/extrude/array-along-path shape ops, with schema and cost.

## Context
After ADR 2026-10-10-a-game-style-block-re-skins-shared-prefabs a game can recolour the shared assets and draw its own props as voxels, but every surface is
still one flat colour on a box, cylinder, sphere, capsule, cone or plane. Two things would widen what a game can look like without binary assets (ADR 0008):
surfaces with a pattern, and shapes that are not primitives (a vase, a column, an arch, a rail along a curve). This record designs both so the cost is known
before anyone builds them. Facts it rests on: `mesh::Vertex` is position, normal, colour, sway (no UVs); `gpu::ObjectUniform` is 176 bytes (model and normal
matrices, base colour, material, emissive) and is bound with a dynamic offset aligned to 256 bytes, so **80 bytes per object are free**; only `Box`
collides (ADR 0003 / `PrimKind`); every shader change is validated by `tests/shader_validation.rs` and, for Direct3D, by Windows CI's `direct3d` stage.

## Decision (the design)

**1. Procedural materials: `material.pattern`.**
```json
"material": {"color": "@wall", "roughness": 0.8,
             "pattern": {"kind": "stripes", "color": "@trim", "scale": 0.25, "angle": 90, "mix": 1.0}}
```
- `kind`: `stripes` (bands `scale` m wide, at `angle` degrees in the face plane), `checker` (cells `scale` m), `gradient` (from `color` at the object's
  bottom to the base colour at its top; `angle` tilts it), `noise` (value noise of feature size `scale`, `mix` = contrast), `decal` (below).
- Evaluated in the fragment shader in **object space**: the vertex shader passes the untransformed position as a new varying, so a pattern sticks to the
  object as it moves and needs no UVs. Faces pick the plane by the dominant normal axis (triplanar without blending: one branch per fragment).
- Uniform: 32 of the 80 free bytes: `pattern: [kind, scale, angle_rad, mix]` and `pattern_color: [r, g, b, 0]`. No new binding, no change to the vertex
  format, no change for objects without a pattern (`kind = 0` returns the base colour before any work).
- `decal`: `{"kind": "decal", "rows": [...], "palette": {...}, "size": [w, h], "face": "+z"}` (the voxel sprite format) on one face. The pixels do not fit a
  uniform; they go in one storage buffer of packed palette indices for the frame (<= 64 x 64 x 4 bits = 2 KB a decal) with an offset in the 80 bytes. One new
  binding in the object bind group, so it is the step most likely to meet a Direct3D or WebGPU limit; it is optional and ships last.
- Prefab params bind to it like any colour (`"pattern": {"color": "$stripe"}` with `"stripe": {"palette": "accent"}`); `style.materials` presets may carry a
  pattern, which is how a game gets "all wood is planked" in one line.

**2. Shape ops: three new object types.**
```json
{"id": "vase", "type": "lathe", "profile": [[0,0],[0.12,0],[0.16,0.1],[0.09,0.3],[0.11,0.42]], "segments": 24, "material": {"color": "@stone"}}
{"id": "arch", "type": "extrude", "outline": [[-1,0],[-1,2],[0,2.6],[1,2],[1,0],[0.7,0],[0.7,1.9],[0,2.3],[-0.7,1.9],[-0.7,0]], "depth": 0.3}
{"id": "rail", "type": "array_path", "template": {"type": "box", "size": [0.1, 1, 0.1]}, "path": [[0,0,0],[2,0,1],[4,0,0]], "spacing": 0.5, "align": true}
```
- `lathe`: a profile of `[radius, y]` points revolved around +Y in `segments` steps (smooth normals; a closed bottom when the first radius is 0).
- `extrude`: a 2D outline in the XY plane (any simple polygon; ear-clipping triangulation, holes not in v1) pushed `depth` along Z, capped.
- Both become a new `PrimKind::Mesh` carrying generated vertices; `GpuMesh::upload` already takes raw vertices, `half_extent` uses the generated bounds,
  and the triangle budget is enforced at parse time (4000 a mesh, like the 2D game's model budget). They do **not** collide (only boxes do): a collider
  is authored as boxes beside them, which the brief and `describe objects` must say plainly; `lint` treats them like any walk-through object.
- `array_path`: a parse-time macro exactly like `array` (copies of a template, renamed `<id>.0..n`), placed every `spacing` m along a polyline (or a
  Catmull-Rom curve with `"smooth": true`), yawed to the tangent when `align`. No renderer change at all, and the copies collide if the template does.

## Cost estimate
| piece | where | size | risk |
|---|---|---|---|
| `pattern` stripes/checker/gradient/noise | `schema::Material`, `gpu::ObjectUniform` (+32 B of the free 80), `scene.wgsl` (~60 lines), shader layout tests | ~350 lines incl. tests and docs | low: no new binding; Direct3D via `direct3d` |
| `decal` | + one storage buffer and bind group entry, packing at scene load | ~300 lines | medium: a new binding on every backend |
| `lathe` + `extrude` | `PrimKind::Mesh`, mesh generation (`geometry`), bounds, budget, render upload, `describe objects` | ~500 lines | low-medium: collision expectations must be clear |
| `array_path` | `macros.rs`, like `array` | ~150 lines | low |
Order: `array_path` (no risk, immediate use), then the four patterns, then `lathe`/`extrude`, then `decal` if the earlier pieces show demand.

## Consequences
- Nothing here is built yet; `style` and `voxels` are. Build a piece when `style-report` on generated games shows the need: games re-skinned but still all
  flat boxes (patterns), or games hand-stacking cylinders into vases and arches (shape ops).
- The pattern design keeps "a scene is plain JSON, materials are data": no texture files, nothing to import, and the same scene renders the same
  everywhere the shader runs.
