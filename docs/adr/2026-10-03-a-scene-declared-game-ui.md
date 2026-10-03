# 2026-10-03. A scene-declared game UI
Status: accepted
Summary: A scene's ui block gives the game friendly labels, counters, an objective, a start card and end cards with a restart button, shown by the client and checked by ui-shot and ui-check.

## Context
Moonlight Delivery's author (`docs/analysis/2026-10-02-moonlight-delivery-feedback.md`, item 3) found the stock HUD looked like an engine overlay: it listed raw
variable names (`Moon_stamps`), instructions truncated, and there was nowhere to put an objective, a progress counter, a start screen or a restart button, so a
small game needed UI code the engine could have provided. The pieces existed: a `hud` block that chooses what the standard client draws, a rules panel, an
outcome banner, an audited headless UI kit (`ui-shot`, `ui-check`). What was missing was the game's own words.

## Decision
- A scene may declare `ui` (`ui_config::parse_ui`): `title`; `labels` (variable to friendly name); `counters` (`{var, of?, label?, format?}` drawn as
  `PARCELS: 3 / 6`, `format: clock` writes seconds as `1:23`); an `objective`, one string or a list of `{if?, text}` where the first line whose condition holds is
  shown (the engine's expression language over the scene's variables); a `start` card; and an `end` card per outcome with `default` for the rest. Texts may name
  variables as `{name}`. A card's button defaults to START / PLAY AGAIN, takes a label, or `false` for none.
- Everything is checked when the scene loads, with did-you-mean: variables in labels, counters, `of`, conditions and `{placeholders}`; `end` keys against the
  outcomes the rules can actually produce (`end` actions); unknown keys; texts too long for a screen; an objective line that can never show.
- The pictures are pure `Layout`s (`ui::game::hud_layout`, `card_layout`), so the client paints exactly what `ui-shot` draws and `ui-check` audits.
  `ui-shot game-hud|game-start|game-end --scene S.json --var delivered=3 --outcome victory` draws a scene's own screens; `ui-check --scene S.json` audits its HUD,
  start card and every end card at every window size. The demo game is in the built-in `ui-check`.
- The offline client (`bin/re2/cards.rs`) shows the HUD, holds the simulation at tick 0 until the start button is used, puts the end card up the moment a rule
  ends the match, and restarts by building a fresh `App` from the same scene file in the same window (the way the map switcher travels), skipping the start card.
  Keys: Enter, Space or E; or a click. Escape still opens the pause menu.
- A headless script drives it like a player: `press: start`, `press: restart`. The state dump gains `/card` ({kind, title, text, button}) and `/hud/lines` now
  carries the offline HUD's text, so `expect` can assert the words a player reads.
- `hud` keeps its meaning and composes: `show_rules_vars: false` hides the plain variable rows but counters and the objective stay; `enabled: false` hides all.

## Consequences
- A small game gets a presentable, testable interface from data. `recipe coin_run` shows a `ui` block.
- The cards are offline only. Online, a shared match cannot wait for one player's click and has its own lobby and results screens, so an online client shows the
  friendly HUD (labels, counters, objective) and the plain outcome banner, not the start and end cards. Restart online would be a server decision.
- Presentation only: nothing in `ui` changes gameplay or the match checksum. A restart reloads the scene file, so an edited scene shows on the next play.
- Fonts and colours are the engine's; this is the declarative layer, not a theming system. A game wanting a different look still writes a custom client.
