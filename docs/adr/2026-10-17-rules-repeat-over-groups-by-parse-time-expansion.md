# 2026-10-17. Rules repeat over groups by parse-time expansion
Status: accepted
Summary: `groups` name members and `for_each` repeats a rule, a block of rules or a run of actions per member, with `$name`/`$name.field` placeholders and per-member variables; the scene loader expands them into plain rules, so the runtime, checksum, replays and protocol are unchanged

## Context
Phase 2 item 1 of ADR 2026-10-13-the-next-phase, gap 1 of `docs/RULES_LANGUAGE_GAPS.md`. Rules see global variables, and a rule names one volume and
one set of objects, so anything "per tile" was a copy of every rule per tile. Gentle Carousel's 5x5 board was 252 generated rules (102,527 bytes),
and 13 of 17 Idea Forge runs wrote a Python generator for rules like these (docs/analysis/2026-10-17-rule-generators-kept.md). The gap note's plan has two
steps: first sugar that expands when the scene loads, then real per-entity variables and restartable timers. Only the second changes what runs.

## Decision
A scene may declare `groups`: a list of ids (`*` matches top-level object ids in scene order), or id -> fields. A rule with `for_each` becomes one rule
per member. A block `{"for_each", "in"?, "rules"}` repeats its rules per member, in order, and nests. `{"for_each", "in"?, "do"}` inside a `do` repeats
actions in place. A variable named with `$group` is declared once per member. `$name` is the member's id and `$name.field` one of its fields. A
string that is exactly one placeholder becomes the field's value; inside a longer string, ids and scalars are spliced in. The longest binding name in
scope wins. `in` is a group, a list, or a placeholder that holds one.

`sim::rule_groups::expand` runs in `schema::parse_scene_text` right after prefab expansion and removes every template, so the rest of the engine sees
only plain rules and variables. That is the same choice as `wall`, `fence` and `prefab` (ADR 0006). Errors keep the template's address:
`Expansion::relabel` turns a rule checker's `rules[40]` back into `rules[3].rules[2] (id, tile = t04)`, which `validate` maps to the template's line.
Guards: a template's id must hold its placeholder (expanded ids differ), a pattern must match something, a group must have members, at most 4,096
expanded rules, every unknown name gets a did-you-mean.

Not chosen for this step: per-entity variables as runtime state (`tile.age`), a `timer` action, list-valued triggers or conditional actions. Each
changes the rule state, so the checksum and the snapshot too (the gap note's step 2, with its own decision on replication).

## Consequences
Gentle Carousel is 13 rule entries and 34 KB. Its expansion is the old file's rules exactly, minus three rules that could never fire, which also
gives identical `sim` output and checksums (docs/analysis/2026-10-17-rule-groups-for-each.md). A scene without templates is not touched. The typed
definitions gain `groups`, `RuleEntry`/`RuleBlock` and `ActionEntry`/`ActionBlock`, and `schemas/` follows. A tool that reads a scene's raw JSON instead
of the parsed scene must expand it first, as lint's impulse-speed check now does (`recipe`'s rule list shows the template ids). Undo: delete the module and its
call; scenes that use it would need writing out by hand.
