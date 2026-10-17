# The HUD's 16-variable cap online: the variables it reads go first, and lint names the rest (2026-10-17)

Phase 2 item 0 of ADR 2026-10-13-the-next-phase. Measured on the 4-core N97 at main c4860ce (before) and this branch (after).

## The bug

A networked match (`red_server`, `play-local`, and `playtest`, which runs a host) presents rules to clients in one repeated `ServerMsg::RuleState`. The
server filled its variable list with `rules.vars().into_iter().take(MAX_RULE_VARS)`: the first 16 in declaration order. The online HUD finds a variable by
name (`ui_config::lookup`, `HudConfig::visible_vars`) and reads a missing one as 0. So a counter, an objective condition or an objective placeholder on
the 17th variable or later showed 0 online and nothing said why. Quiet Compass (Idea Forge 2026-10-15, F1, 6 minutes lost) has 20 variables: its playtest
showed `HOUSE CRUMBLES: 0:00` while `sim` counted down. The agent worked around it by moving its counters to the top of `vars`.

**Reproduced before the fix** (`tests/net_rule_state.rs`, `a_hud_counter_on_a_variable_declared_after_the_sixteenth_counts_online`, run on main's code):
20 variables, a counter on `v17`, an objective that reads `v18` and `v19`, a rule that sets them. The real server and client over loopback: the client
got `v00`..`v15` only (`left: (None, None, None, Some(2.0))`), so the HUD drew `CRUMBLE 0 / 9` and the objective `WAIT` instead of `7 / 9` and
`LEAVE (3 LEFT)`. The same test passes after the fix.

## Does sending the HUD's variables first fix it without a protocol change? Yes

Each variable travels with its name (`RuleVar { name, value }`), and every reader on the client looks a variable up by name, so a client finds one wherever
it sits in the list. One reader depends on order: the HUD's plain rows list the received variables in the order they arrive. The server therefore picks
which 16 to send (the HUD's first) and sends them in declaration order, so the rows keep their offline order.

- `ui_config::hud_reads`: the variables the standard client's HUD reads, each once, with the first place that reads it: counters (`var`, `of`), objective
  conditions (`Expr::read_vars`, new) and placeholders, then the first `HUD_ROWS` (8) rows (`hud.show_rules_vars` or `hud.custom_vars`). A hidden HUD
  (`hud.enabled: false`) reads none. Cards and the pause line are not in it: online they do not read the server's variables (see "Found along the way").
- `net::protocol::rule_vars_to_send`: those first, then the others in declaration order up to 16, sorted. `MatchSim` computes the list once per match
  (`MatchSim::sent_rule_vars`); `Server::send_statuses` sends it.
- `HUD_ROWS` replaces the literal 8 in `ui::game::hud_layout` and `ui::rules::panel_layout`, so the read list cannot drift from what the HUD draws.

Wire: unchanged. Protocol stays v16, `RuleState`'s encoding is untouched, the worst case stays 1,392 of 1,400 bytes. A v16 client of the old build
works with a new server and the reverse. The test asserts that the online HUD (drawn from the server's 16) equals the HUD drawn from all 20 variables,
and that the old first-16 list draws the 0.

Cost: the list is computed once per match, not per send; the per-send work is the same (one `vars()` and one `Vec<RuleVar>`).

## What still cannot fit: a lint error

A HUD that reads more than 16 variables, or reads one whose name is over 32 bytes (`MAX_RULE_TEXT`, the wire cuts it), cannot work online. `lint` reports
it as an error, code `net-hud`: one message naming the count, the cap (`net::protocol::MAX_RULE_VARS`), the first six that would not arrive with the
place each is read, and the fix (fewer counters or objective variables, or fewer rows: `hud.show_rules_vars: false` with `hud.custom_vars`, or a `_name`
variable). `describe lint`, `describe ui` and SPEC say it.

Why lint and not a scene-load error (the reject-bad-input ADR's usual place): a load error would stop a game from loading at all, offline too.
Redline (RedEngineGames, declared `networking: offline`) reads 42 variables in its 18-line objective ladder; offline it is fine, online and in its
playtest its objective shows wrong lines. Lint is in every agent loop (`lint`, `verify`'s lint budget, the Idea Forge gates) and leaves loading alone.

| content | scenes | `net-hud` errors |
|---|---|---|
| this repository (every scene `git ls-files` finds) | 37 | 0 |
| RedEngineGames projects, killchain, great-outdoors, the-hollow, mini-games (local checkouts) | all their maps | 1: Redline (reads 42) |

`validate` over the repository: 96 content files, 0 problems (unchanged; the check is not a validate error). Quiet Compass as committed reads 6
variables (positions 0-4 and 14); with this change its workaround was not needed.

## Raising the cap: protocol v17 (accepted by the owner the same day)

ADR 2026-10-17-rule-variables-travel-by-index-so-a-networked-match-can, accepted: a `RuleState` variable is its index among the scene's own variables
(`u16`) and its value, 10 bytes; the client names it from its own scene (`RuleState::named`, `RuleSet::own_var_names`), which it parsed from the same
map the server's hash names. `MAX_RULE_VARS` is 64. The `net-hud` lint fires above 64 and no longer checks name lengths (names do not travel).

| scene | own vars | v16: vars sent / bytes | v17: vars sent / bytes |
|---|---|---|---|
| quiet-compass | 20 | 16 / 239 | 20 / 200 |
| gentle-carousel | 30 | 16 / 232 | 30 / 300 |
| velvet-orchestra | 7 | 7 / 95 | 7 / 70 |
| sunken-marathon | 10 | 10 / 123 | 10 / 100 |
| redline (RedEngineGames) | 227 | 16 / 272 | 64 / 640 |
| worst case (the protocol test) | | 16 / 656 (whole message 1,384 + 8 tag) | 64 / 640 (whole message 1,368 + 8 tag) |

Method: the var section is fixed-width, so it is computed from each scene's variable names (v16: 1 + name bytes up to 32 + 8 per variable; v17: 10).
Every variable is cheaper; a scene with more than 16 now sends more of them, so its message grows (Gentle Carousel: 232 -> 300 bytes, all 30 sent).
Redline's HUD reads 42, under 64: its `net-hud` lint error is gone. A v16 client and a v17 server refuse each other with the version message, as for
every bump.

## Found along the way (not fixed here)

- Online, the pause menu's `ui.pause` line and the audio layers (`audio.layers[].var`) read the client's own `RulesEngine`, which a networked client does
  not step: they show the scene's starting values, not the match's (`client::window`, `client::ambient`).
- `ui.title` is checked for `{var}` placeholders, but the HUD prints the title without filling them.
- Event and outcome names over 32 bytes are cut on the wire; online only the banner text shows it.

## Verification

`scripts/dev iterate`: fmt, clippy, 37 unit tests (lint, describe, ui), `net_rule_state` (2), core 38, red2d 69: green in 14 min. New tests:
`ui_config::tests::the_hud_reads_counters_then_the_objective_then_its_rows_each_once`,
`ui_config::tests::a_hud_that_reads_more_than_the_wire_sends_is_an_error_that_names_the_cap_and_the_fix`,
`net::protocol::tests::the_hud_s_variables_are_sent_first_and_the_rest_fill_up_in_declaration_order`,
`tools::lint::silent_failure_tests::a_hud_reading_more_than_a_networked_match_sends_is_an_error_and_sixteen_late_ones_are_not`, and the regression test above.
