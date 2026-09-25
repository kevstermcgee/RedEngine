# 0034. `net-test` and bad-network resilience
Status: accepted

## Context
Network code is easy to make pass on loopback and hard to make good on Wi-Fi. BlueEngine's `net-test` runs 120 ticks of a prediction
buffer through a latency/loss model *inside one process*, without a socket, a server or an interpolator, and prints counters. Red's
existing lossy test used independent 15% loss on two clients and asserted a few end states; nobody looked at what a *remote player looks
like* on screen during a burst of loss.

## Decision
- `net::netsim`: a seeded UDP proxy with **bursty loss** (a two-state Gilbert-Elliott model: real links drop runs of datagrams, which
  is what defeats "survives an isolated loss"), one-way delay, jitter (so datagrams reorder), duplication, and per-direction counters.
  Profiles `lan`, `wifi`, `4g`, `bad` (5% loss in bursts of 3, 100 ms round trip, 1% duplicates), `awful`.
- `red_engine2 net-test scene.json --profile bad|all --players N`: a real server and real clients (handshake, tags, prediction,
  interpolation, reconnect) behind the proxy, judged on what a player notices: nobody disconnected, prediction ends on the server's
  position, corrections stay small, **other players glide instead of teleporting**, bandwidth stays modest, and the link really was
  lossy. JSON with `--json`; `tests/net_sim.rs` runs it in CI.
- **It found a real defect.** On `bad`, a remote player froze for a burst of lost snapshots and then jumped 0.5 m in one frame, because
  interpolation held still when data stopped. Fix in `net::interp`: players are carried on along their last heading at the server's
  reported speed for at most `MAX_EXTRAPOLATE` (0.25 s), never through a teleport or from a standstill; and a catch-up after a longer
  outage is limited to `CATCH_UP_SPEED` (10 m/s) so it glides. Props still hold (a prop that came to rest must not overshoot).

## Consequences
Every link profile passes with 2 and 8 players. The verdict thresholds are deliberate: worst drawn step 0.25 m per 10 ms, final
prediction error 0.10 m. Network randomness is seeded, wall-clock timing is not, so thresholds have margin. Not simulated: bandwidth
caps, asymmetric links, NAT rebinding mid-session (a client whose source port changes is treated as a new join and resumes by token),
and lag compensation for hitscan (still open).
