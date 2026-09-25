# 0029. The match flow: lobby, ready-up, countdown, rounds, results, rematch
Status: accepted

## Context
Until now a joined client was in the world at once, forever. A game needs a front end: connect with a key, a lobby with character
choice and ready-up, a countdown, timed rounds, a results screen and a rematch. The comparison with BlueEngine's multiplayer
template showed its screens are drawn but not wired (Connect just switches screen; Ready jumps straight to Playing), and its
rematch is a screen, not a protocol. The flow has to be authoritative (so every client agrees), testable without a window, and
tolerant of lost datagrams.

## Decision
- **`sim::flow`: a pure state machine** (`Waiting -> Countdown -> Playing -> Results -> Waiting`), stepped once per server tick with
  `FlowInput {connected, ready, in_round, rules_outcome, best_score}` and answering with at most one `FlowEvent` the server acts on.
  Every rule is unit-tested with no network. It is opt-in: a scene's top-level `"match"` block (min_players, countdown_secs,
  round_secs, results_secs, score_to_win, join_in_progress, ready_check; strict keys) or `red_server --lobby`. Without one the
  server is in **open play**, exactly as before.
- **The server owns the world's life.** The world is rebuilt from a factory at each countdown (a rematch starts from the authored
  map), players are placed at spawns in their stable player id, and the countdown freezes them there. A round ends on the rules'
  `end`, the clock, `score_to_win`, or everyone leaving. A late joiner plays at once or watches until the next round
  (`join_in_progress`). Each finished round goes to a hook with its winner, scores and recorded trace (`--record` writes
  `TRACE.roundN.json`).
- **Protocol v3 messages are state, not events.** The client repeats `Lobby {ready, character}` at 5 Hz (and on change), the server
  repeats `Status {phase, timer, roster, result}` at 5 Hz (and on change), and repeats a round's `Welcome` until the client's
  `round_ack` shows it applied. A lost datagram costs a fifth of a second, never a state. Pings in the roster are client-reported
  (cosmetic). A rematch is everyone pressing Ready again during the results.
- **`ui::online`: the screens** (connect form with a masked key, lobby, HUD with ping / timer / scoreboard / countdown /
  "watching" banner, results with a rematch button) are `Layout`s on the ADR 0026 kit, audited at nine window sizes, rendered with
  `ui-shot`. The `re2` client feeds them from the `NetClient`, freezes the local body outside a round, and shows them.

## Consequences
`tests/net_flow.rs` runs the whole loop over real UDP (join, ready, countdown, round, time up, results, lobby, rematch, late joiner,
aborted countdown, keyed server). `scripts/lobby_demo.ps1` drives a real graphical client through it and screenshots each stage.
Not built: a spectator camera for late joiners, kick/ban, teams, map voting, and per-player rule state in `Status` (rules stay
server-side; a game shows its own score through `vars` once those are replicated).
