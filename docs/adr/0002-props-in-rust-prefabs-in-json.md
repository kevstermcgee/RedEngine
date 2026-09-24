# 0002. Props (custom collision) are Rust; everything else is JSON prefabs
Status: accepted

## Context
The asset library must grow fast (apples to vending machines) and be extendable by AIs without
recompiling. But a few objects need behaviour a JSON group can't express: a single union collider,
"walk-through" small items, origin-at-base guarantees enforced by tests.

## Decision
- **Prefab** (`assets/*.json`): parametric groups of ordinary objects (`$param`, `"=expr"`, `extends`).
  This is the default way to add an asset; no Rust, checked by tests (expands, tagged, floor prefabs
  start at y=0, unique names). Scene-local `"prefabs"` cover one-offs.
- **Prop** (`src/props.rs`): only when custom collision is needed. One collider per prop's overall
  footprint (union AABB of its parts) — per-part colliders make gap-riddled, unintuitive collision.
- Both share conventions: origin = middle of the base, front = local +Z.

## Consequences
- ~100 prefabs vs 39 props; the ratio should keep tilting toward prefabs.
- `props::tests::props_rest_on_the_floor` guards the origin rule; a prop with an odd origin is a bug.
- A prefab that needs collision beyond `box` parts (or `collide:false`) is the signal to promote it.
