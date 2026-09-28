# 0053. Lag compensation: a shot lands where it was aimed on screen
Status: accepted

## Context
A client draws the other players slightly in the past (ADR 0016: 100 ms of interpolation so motion is smooth across 30 snapshots a second), and the shot then takes half a round trip to
reach the server. The server judged that shot against where everyone is *now*. Against anyone moving, the gap is metres: a bot strafing at 6 m/s is 0.6 m from where the screen shows it
at 100 ms, wider than the body, so aiming at what you see misses. In a shooter that is the difference between "my aim is bad" and "the game is broken".

## Decision
The server judges a shot against the world its shooter saw. Each tick `MatchSim` remembers where every player stood (`HISTORY_TICKS`, 16 ticks). Each player carries a `view_lag`: how many
ticks behind the present their screen is, which the server derives from what the client reports (its round-trip time, capped at 100 ms) plus the interpolation delay
(`net::interp::view_lag_ticks`: 6 ticks on a loopback, 8 at 40 ms, at most 12). A bullet or a bat swing is traced against the other players' positions `view_lag` ticks ago
(`MatchSim::probe_lagged`); fixed geometry and props are judged as they are now. Bots see the present (lag 0).

The lag is simulation input like any other: setting it (`MatchSim::set_view_lag`) is recorded in the trace as a `ViewLag` entry, only when it changes, so a recorded match replays bit for
bit and a fixture without the entry still replays as before. No protocol change: the server already knows the round trip.

## Consequences
- Aim at what you see and you hit it, on a loopback and on a connection with latency; `tests/combat_rules.rs` proves the four cases (aimed at the late position with the lag known: a hit;
  the same aim judged in the present: a miss; aimed at the present with the lag: a miss; aimed at the present without it: a hit) and that a replay judges the shot the same way.
- The usual trade of lag compensation: the shooter is favoured, and a target can be hit a moment after it has stepped behind cover on its own screen. The cap (about 200 ms) bounds that,
  and a client cannot ask for more by claiming a long round trip (the server clamps what it believes).
- Cost: 16 small frames of positions, no allocation per tick (`alloc_budget` unchanged).
- Limits: the hit box is the player's cylinder at the rewound position (height and crouch as they are now); a player who respawned inside the window is judged at their old position.
