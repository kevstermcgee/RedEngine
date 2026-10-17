# `groups` + `for_each`: Gentle Carousel from 252 rules to 13 entries, with identical play (2026-10-17)

Phase 2 item 1, first PR (ADR 2026-10-17-rules-repeat-over-groups-by-parse-time-expansion): rule templates expanded when the scene loads, no
protocol change. Measured on the 4-core N97; "before" is main c4860ce (the `prebuilt-e4f91efe9590` binary), "after" this branch's debug build.

## Gentle Carousel, before and after

| | before | after |
|---|---|---|
| rule entries in the file | 252 | **13** (8 plain, one of them `init` with 2 action `for_each`; 4 `for_each` rules; 1 block holding 3 templates) |
| rules after expansion | 252 | 249 (the three dropped are below) |
| variables declared in the file | 30 | 6 (`_s_$cell` declares the 25 tile states) |
| file bytes | 102,527 | **34,413** (-66%) |
| `verify` (lint + 6 `checks.sim` play-throughs) | 0.11 s (3 runs: 0.22 cold, 0.11, 0.11) | 0.11 s (same 7 checks pass) |
| `validate` | 0.01 s | 0.01 s |

The board is the same: 25 tiles, `group tile` (the 24 that are not the start tile) with each tile's enter box, its chalk counter (`good` on the
house picture, `stray` off it) and its neighbours; `cell` (all 25) for the lamps and the state variables; `first` (the three tiles revealed at the
start); `stop` (the moth's four stops). The rules read as the design: init, reveal the first three, the beat, the moth, the tide, then for each tile
`break`, `age`, and a block of `chalk`, `stumble` and `rev` per neighbour, then the five endings.

### Parity, three ways

1. **The expansion is the old rules.** A throwaway test parsed the old file and the new one with the same build and compared the whole `RuleSet`
   (every rule: trigger, volume, condition, actions, variable indices) and the `Debug` text of the whole parsed scene: identical, against the old
   file with three rules removed.
2. **The three removed rules could never fire.** `rev_t40_t30`, `rev_t40_t31` and `rev_t40_t41` reveal a neighbour of the start tile when the
   player enters the start tile, if that neighbour's state is 0. `init_rev_*` sets those three states to 1 at the start, and every other write to them
   sets 2, 3 or 4 (checked over every action in the old file), so the condition never holds after the first tick. The player spawns on the start
   tile, which raises no `enter`. A template could keep them (they sit between t34's and t41's block in the old order, so it would need a split
   block), but a rule that cannot fire is not part of the board.
3. **Play is identical.** `red_engine2 --json sim` on the old file minus those three and on the new file, timing fields stripped: byte-identical
   output, including every scenario's events, variables and **match checksum**. Against the unmodified old file only the checksums differ: per-rule
   state is part of the checksum, and there are three fewer rules.

## How a template reads

```json
{"for_each": "tile", "rules": [
  {"id": "chalk_$tile", "when": {"enter": {"box": "$tile.box"}}, "if": "walking > 0 && _s_$tile == 1",
   "do": [{"set": ["_s_$tile", 2]}, {"show": "lamp_$tile"}, {"add": ["$tile.count", 1]}, {"emit": "chalk"}]},
  {"id": "stumble_$tile", "when": {"enter": {"box": "$tile.box"}}, "if": "walking < 1", "do": [{"emit": "misstep"}]},
  {"id": "rev_$tile_$n", "for_each": "n", "in": "$tile.near", "when": {"enter": {"box": "$tile.box"}}, "if": "walking > 0 && _s_$n == 0",
   "do": [{"activate": "$n"}, {"set": ["_s_$n", 1]}]}
]}
```

A mistake names the template and the member: `rules[7].rules[2] (rev_t00_t01, tile = t00, n = t01).if: unknown variable ...`, which `validate`
puts on the template's line.

## What it does not do (step 2)

Each expanded rule is still a rule, and each per-member variable is still a global variable: a tile's state is `_s_t03`, and a timer is still
`every` or `after`, global and not restartable. Gentle Carousel needs nothing more: its age and break cycle runs on one global `tide` event. Games
with a per-entity clock (`no-per-entity-timers`, 5 runs) still need the second PR: per-entity variables and a restartable timer, which change the
rule state, the checksum and the snapshot.

## Verification

`sim::rule_groups` unit tests (9: plain scenes untouched, per-member rules and variables, blocks and nesting with the longest-binding rule, action
`for_each`, `*` patterns in scene order, `in` as group/list/field, every mistake with its template and member, relabelled checker errors, the
4,096-rule guard); `validate` over the repository; `verify` of Gentle Carousel; the content drift test and the regenerated `schemas/`
(scene, blueprint).
