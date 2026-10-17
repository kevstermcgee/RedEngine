# 2026-10-17. Rule variables travel by index so a networked match can send more than sixteen
Status: proposed
Summary: Proposed, not built: protocol v17 sends a rule variable as its index in the scene's vars and its value (9 bytes, not up to 41), so the cap rises from 16 to 64 in the same packet and a name no longer travels

## Context
A networked match presents its rule state in one repeated `ServerMsg::RuleState` (ADR 0022, SPEC "rules online"). Each variable travels as its name
(up to `MAX_RULE_TEXT` = 32 bytes, length-prefixed) and its value (8 bytes): up to 41 bytes, at most `MAX_RULE_VARS` = 16 of them. The server used to send
the first 16 in declaration order, so a HUD counter on a later variable showed 0 online (Quiet Compass, Idea Forge 2026-10-15, F1). Since this PR the
server sends the variables the HUD reads first (`net::protocol::rule_vars_to_send`, `ui_config::hud_reads`) and `lint` reports a HUD that reads more than
16 (`net-hud`), with no protocol change. That covers every scene in this repository; one published game does not fit: Redline (RedEngineGames) reads 42
variables in its objective ladder (it is declared `networking: offline`, but the playtest runs a host, so an agent watching its playtest sees wrong lines).

Raising the constant alone does not work. The worst-case rule state is 1,384 bytes (16 named variables 656, 256 hidden ids 512, 64 collision ids 128, two
texts 66, the rest 22) plus the 8-byte tag: 1,392 of the 1,400-byte packet (`MAX_PACKET`, `protocol::tests::a_status_and_a_welcome_fit...`). A 17th named
variable does not fit.

The names are redundant on the wire: a client joins only with the server's map hash (`Hello::map_hash`), so both sides parsed the same `vars` in the same
order.

## Decision
Proposed, for the owner to accept or reject; nothing here is built.

Protocol v17 (`PROTOCOL_VERSION` 16 -> 17; client and server must match, as for every bump) changes only `RuleState.vars`: each entry is `u8` index into the
scene's own variables (declaration order, built-ins left out, the order of `RulesEngine::vars`) and the value (`u64` bits), 9 bytes. `MAX_RULE_VARS`
becomes 64: 64 x 9 = 576 bytes, under today's 656, so the worst case shrinks to 1,304 + 8 bytes. A client turns indices back into names from its own scene
(`RuleVar` keeps a name for the HUD code, filled on decode from the scene), so `ui`, `hud`, the state dump and the replay need no change. The server keeps
sending what the HUD reads first; `lint`'s `net-hud` then fires above 64. A name longer than 32 bytes stops mattering, since names no longer travel.

Not chosen: sending every variable (a game with hundreds, like Redline's 227, does not fit any packet; the HUD reads few of them); splitting the rule state
over several packets (it is resent whole and must be atomic for a late joiner, ADR 0022); deltas (the state is small and resent; deltas add a baseline to
lose).

## Consequences
A scene could show up to 64 variables online, which covers Redline's 42. Old clients cannot join new servers (the version check refuses them with the
existing message), as with v16. The `net_budget` and packet-size tests keep the bound; `tests/net_rule_state.rs` keeps checking that what the online HUD
draws equals what a client holding every variable draws. Undo: revert to names, v18.
