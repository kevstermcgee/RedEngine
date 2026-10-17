# 2026-10-16. A 2D game's clock stands still while its world is paused
Status: accepted
Summary: time, every, after and cooldowns stop while a pausing dialogue or pause_if holds, so a pause never ends in a burst of overdue timers, and a pause menu is rules under pause_if

## Context
Honeybound v2 needed a pause menu a controller can drive (up/down, A to pick, B or Start to go back). The only way a 2D game could stop its world
was an open dialogue with `pause: true`, and an open dialogue takes every press, so no rule could move a cursor. And while a dialogue held the world
still, the game's clock (`Sim::t_ticks`: the `time` variable, `every` and `after` timers, rule cooldowns) kept counting: when the dialogue closed,
every `every` rule had fallen behind and fired once per tick until it caught up, and conditions like `time > star_until` were already past.

## Decision
A game may declare `pause_if` (an expression; `GameDef::pause_if`): while it holds, `Sim::step` treats the world as paused exactly like an open
pausing dialogue (no movement, touching, `every`/`after` timers), but press rules, events and buttons still run, so the menu is ordinary rules.
While the world is paused, by either cause, the game clock stands still: `Sim::step` does not advance `t_ticks`. `tick` (frames since the start)
keeps counting, so a paused screen can still animate or time a message with it.

## Consequences
A pause of any length resumes exactly where it stopped: no timer burst, power-ups and cooldowns keep their remaining time. A game that wanted
wall-clock time during a dialogue must use `tick` now (no example did; all 2D examples verify unchanged). A menu's close rule must come before its
open rule (rules run in order and see each other's changes), guarded by `tick`, or one press both opens and closes it; `describe 2d` says so.
Undo: advance `t_ticks` unconditionally again and drop `pause_if`.
