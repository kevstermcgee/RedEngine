# 0006. `wall`/`fence`/`prefab` expand to plain objects at parse time
Status: accepted

## Context
Authoring 280 raw boxes for a house is unreadable, but adding new object kinds to the renderer,
colliders, ground candidates, bounds, lint and plan every time is expensive and error-prone.

## Decision
Convenience types are *macros*: `src/macros.rs` (`wall` with openings/trim/baseboard, `fence`) and
`src/prefabs.rs` (`prefab`) rewrite themselves into ordinary `box`/`group` JSON inside
`schema::parse`. Everything downstream sees only the expanded objects.

## Consequences
- Adding sugar = write `expand_x`, add to `MACRO_TYPES`, unit-test, document in SPEC. Renderer,
  physics and tools need no change.
- Tools that edit files must edit the *authored* JSON (`ls`/`info` show expanded pieces with `--all`).
- Diagnostics have to be mapped back to the authored id; expanded pieces get derived ids.
