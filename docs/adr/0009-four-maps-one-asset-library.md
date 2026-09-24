# 0009. Four maps (house, school, office, store) share one library
Status: accepted

## Context
The prop-hunt game needs varied maps, but a hider disguises as a prop, so the prop set matters as
much as the layouts. Per-map assets multiply work and hurt prop recognisability.

## Decision
Exactly four maps: **house**, **school**, **office**, **convenience store**, all drawing on the same
prop/prefab catalogue ("the same apple in every map is fine"). House is done (`examples/house.json`);
`recipes/` seeds the rest (`classroom_wing` → school, `convenience_store`, `rooms_and_door`).
Each map carries its own `checks`, so `verify` guards it.

## Consequences
- Asset work goes into the shared catalogue with good tags; maps mostly compose.
- A second floor in school/office reuses the multi-floor rules (ADR 0004): separate upper-floor walls,
  a landing for every staircase, a railed stairwell opening.
