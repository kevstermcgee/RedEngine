# 0037. Core assets are a growing API, not an exhaustive inventory
Status: accepted

## Context
Agents need reliable visual and gameplay vocabulary, but predicting every future asset creates a
large, low-quality library. Game-specific assets are where useful abstractions are actually found.
The existing catalogue could list built-ins, but it did not expose provenance/lifecycle metadata or
discover project-local packs before they were copied into the engine.

## Decision
Use **Reuse -> Modify -> Generate -> Import**. The curated core stays split into focused JSON packs
registered by `assets/packs.json`; asset names are stable API identifiers. `catalog --json` exposes a
versioned normalized record with discovery, placement, geometry, physics, lineage, lifecycle,
parameters and instantiation while retaining the old flat keys. Optional prefab `meta` records aliases, roles, styles, status/revision,
license, origin and provenance. `catalog --library` makes game-local packs equally discoverable.
Promote only proven, general assets into the narrowest shared pack after validation and visual review.

## Consequences
Games can invent specialized vocabulary without bloating or forking the engine, while useful pieces
have an explicit path back to core. Existing prefab files remain valid through metadata defaults.
Changing a stable identifier or its meaning now requires a variant/deprecation or a new ADR.
