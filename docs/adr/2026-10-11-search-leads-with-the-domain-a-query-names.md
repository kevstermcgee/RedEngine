# 2026-10-11. Search leads with the domain a query names
Status: accepted
Summary: search's lead lane holds verified 2D mechanics and 3D recipes, filtered by the domain a query names outright (2d/sprite vs 3d/scene/room), instead of 2D mechanics alone.

## Context
The agent-loop trace (docs/analysis/agent-loop-trace.md, task a, sink 3) built a 3D scene and spent three searches (17 KB) with little to show for them. Since
the 2026-10-06 efficiency pass, `search` opens with a lane of up to two verified **2D** mechanics when two query words single one out; nothing did the same
for the 3D recipes, and the lane ignored a query that says "3D" outright. Measured on the trace's queries and a few controls:
"locked door opened by a key" opened with the 2D `key-door` and found the 3D recipe `gated_garden` 9th of 9; "... in a 3D scene" opened with the 2D
`collect-then-exit`; "3d scene with a gate that opens with a key" opened with two 2D mechanics; even "two rooms joined by a door" opened with `key-door`.

## Decision
- `query_domain` reads the domain a query names outright: 2D (`2d`, `sprite`, `platformer`, `game2d`, `pixel`, `topdown`), a 3D scene (`3d`, `scene`,
  `room`, `floor`, `stairs`, `fps`, `first person`), or neither (both or none).
- The lead lane takes 3D recipes by the same rule as the mechanics: at least two query words that occur in at most two recipes. The lead is two
  mechanics for a 2D query, two recipes for a 3D one, and the best of each (mechanic first) when the query names neither. A recipe that does not make the
  lead stays among the ordinary hits; one that does is not repeated.
- The split never depends on whether a mechanic matched, so the guard `the_mechanics_lane_never_displaces_an_ordinary_result` still holds;
  `the_lead_follows_the_domain_a_query_names` pins the trace's queries.

## Consequences
Rank of the answer the task needed, before -> after (same corpus, `search "<q>"`, first 6 shown):

| query | answer | before | after |
|---|---|---|---|
| locked door opened by a key (trace a #8) | recipe `gated_garden` | 9 (of 9) | **2** (after the 2D `key-door`) |
| show dialogue when the player picks up an item in a 3D scene (trace a #11) | not a 2D mechanic | #1 was the 2D `collect-then-exit` | no mechanic; #1 is the 3D `coin_run` (rules on pickup) |
| 3d scene with a gate that opens with a key | `gated_garden` | 4 | **1** |
| two rooms joined by a door | `rooms_and_door` | 2 | **1** |
| add a door that opens with a key | `key-door`, `gated_garden` | 1, 5 | 1, **2** |
| a 2d game with a key that opens a door | `key-door` | 1 | 1 (no recipe in the lead) |
| door-blocked locked door phase (trace a #18) | (no fragment covers it) | — | unchanged |
| game rules as data coin | `coin_run` | 2 (behind the 2D `story-beat`) | 2 (unchanged) |

- For trace a, the first search now holds the recipe the task copied, which the trace reached through a second listing (`recipe`) and a guess; the output
  size is unchanged (5.5 -> 5.7 KB).
- Not fixed: a mechanic can still lead a query that names no domain on weak words (`story-beat` for "game rules as data coin"), and no document says
  "a 3D scene has no dialogue widget" (trace sink 8): search cannot rank a fragment that does not exist.
- Undo: drop `query_domain` and the recipe lane in `search`.
