# 0051. Shooter feedback on the wire (protocol v8)
Status: accepted

## Context
ADR 0050 made the server's combat produce the facts a shooter needs to *feel* like one: how many shots a player fired, which of ours landed, how often we were hurt and from where.
Nothing reached a client. A snapshot carried each player's health, weapon and pose, so an online client could not play another player's gunshot, show a hit marker, point at whoever
was shooting it, announce a kill or count down its own respawn, except by guessing from health going down. The offline client had all of that only because it runs the simulation itself.
Feedback is the difference between a shooter and a box that hurts you; it has to work online, where the game is played.

## Decision
Protocol version **8** adds three things to the snapshot, all *counters* that only grow, never events:
- `PlayerSnap::shots: u8`, per player: how many shots that player has fired (wraps at 256).
- `Snapshot::fx: Feedback`, five bytes addressed to the receiving client: `hits` (our attacks that damaged someone), `hurt` (times we were damaged), `kills` (kills we scored), `bearing`
  (the world direction of whoever damaged us last, a byte's worth of a full turn) and `respawn` (tenths of a second until we return, while dead).
- `FLAG_PROTECTED` in a player's flags: spawn protection is active (so the client can show it and other players can be drawn shimmering).

A client turns counters into events with `net::happenings::Watcher`: it remembers the previous values and reports the differences (`wrapping_sub`), so a lost datagram just means the next
snapshot reports two shots instead of one, and nothing is acknowledged or resent. The first snapshot after joining, resuming or a new round only sets the baseline, a player who leaves
interest range starts over when they come back, and a jump of more than 8 shots in one snapshot is treated as a stale counter, not a firefight. Our own shots are not "heard" (a client plays
those when it pulls the trigger). `NetClient` surfaces the result as `NetEvent::Happened` and `NetSession::take_happened()`; `Bot` counts them so tests can assert on what a client perceived.

Counters rather than events because the wire is unreliable UDP at 30 snapshots a second: a running total is idempotent and self-healing, a fixed 13 bytes a snapshot (5 + 1 per player,
eight players), and needs no per-client queue on the server.

## Consequences
- Every client can hear and see the fight: positional gunshots from other players' weapons, hit markers, kill confirms, a damage direction and a respawn countdown, all from data already in
  the snapshot. `tests/net_bots.rs` proves it over real UDP: bots' shots and damage arrive at an idle client as events.
- Cost: snapshots grow 13 bytes (worst case 1239 -> 1252, budget 1250 -> 1260, still one datagram with its authentication tag), idle bandwidth per client +390 B/s (10 500 -> 11 200 B/s in
  `tests/net_budget.rs`). Raising a budget is a decision, so it lives in the constant, with the reason.
- The protocol version is checked exactly, so v7 clients and servers are refused at the handshake; rebuild both together.
- Limits: the counters say *that* something happened and roughly where, not the exact ray (a shot's tracer is drawn from the shooter's pose and the target's, which is what the
  client knows); `respawn` is only known to the player it concerns.
