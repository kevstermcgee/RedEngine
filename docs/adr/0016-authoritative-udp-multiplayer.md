# 0016. Authoritative UDP multiplayer: server, snapshots, prediction, interpolation
Status: accepted

## Context
Roadmap items 3-5 (ADR 0015): make the simulation authoritative in a headless process, put a real
low-latency transport around it, and prove one server + two independently running graphical clients in the
Test Lab. Constraint from the brief: nothing here may need a window, GPU or audio device on the server side
of the design (update: the crate is now split by a `gfx` feature and `red_server` links none of them, ADR 0017).

## Decision
**Authority.** `sim::match_sim::MatchSim` owns every player and every physics prop and is ticked at 60 Hz. A
client never sends a position, only a `PlayerInput` (wish direction as -1/0/1 per axis, jump/sprint/crouch,
look yaw/pitch); `sim::player::step_player` turns it into movement and is the *same function* single-player
and client prediction run. A player's state is a pure function of the inputs the server has processed, in order
(one per tick, two when a backlog builds, none = the player does not move: no extrapolation). Players shove
props through kinematic cylinders (`PropWorld::set_player_slot`); the props are simulated only on the server.

**Transport.** Plain UDP, hand-rolled little-endian messages (`net::protocol`, no new dependencies), fuzz-tested
decoding with hard limits (packet <= 1400 bytes, <= 8 players, <= 30 props, <= 4 inputs per packet).
Client -> server: `Hello` (protocol version, map hash, character, resume token), `Input` (last 4 inputs
redundantly + snapshot ack + client time), `Bye`. Server -> client: `Welcome`, `Reject` (version / wrong map /
full), `Snapshot`, `Bye`. Hello is retried every 250 ms; an unknown sender's Input gets a `Bye`, which makes a
client whose server restarted rejoin at once.

**Snapshots.** Every 2 ticks (30 Hz) each client gets every player plus **the props that changed since that
client last acknowledged**, using per-client cursors over `sim::change` (`TrackedColumn::iter_changed_since`).
A snapshot that carried *all* changes lets the cursor advance when acked; if more than 30 props changed, a
rotating window is sent and the cursor stays put, so nothing is ever lost, only delayed. A prop that stops moving
publishes its resting pose once and then costs nothing. Prop ids are indexes in `physics::loose_props`, which
server and client compute identically from the same map (the map hash is checked on join).

**Client feel.** Remote players and props are drawn ~100 ms in the past, interpolated between snapshots
(`net::interp`: history per thing, no extrapolation, server-clock estimate that favours the least-delayed
packets). The local player is predicted: inputs apply immediately and are replayed on top of the server's state
when a snapshot acknowledges some of them (`net::predict`); a correction under 1.5 m fades out over ~100 ms,
larger ones snap.

**Sessions.** A client silent for 3 s is dropped; its player is parked for 30 s under its token, so `Hello` with
that token resumes the same player id and position (also from a new port). An unknown token is simply a fresh
join, so clients reconnect automatically after a server restart.

**Tools.** `red_server` (headless; `--spawn-group`, `--demo-kick <object>`, stats line every N s), `red_bot`
(headless scripted client printing JSON), `re2 --connect HOST:PORT` (the graphical client; online it disables
weapons and pick-up), `scripts/multiplayer_demo.ps1` (server + two real windows, screenshots, kills and
restarts things), `scripts/play_multiplayer.ps1`.

## Proof (all run in CI as `cargo test`, plus the demo script for real windows)
- `tests/net_e2e.rs` (real UDP, in-process): two clients join, move and see each other smoothly (monotonic,
  no jumps); prediction matched the server (worst correction < 5 cm); disconnect + reconnect gracefully and by
  timeout, same player id and position; a barrel pushed by one client is seen moving, and ends at the same pose
  on both clients and the server; the same through 15% loss, 40 ms latency and jitter; wrong map / full match
  rejected; garbage datagrams survived.
- `tests/net_processes.rs`: one `red_server` and two `red_bot` **OS processes**, one leaves and rejoins with its token.
- `net::session` test: the graphical client's scene updates (avatars appear and vanish, moved props posed).
- `scripts/multiplayer_demo.ps1`: server + two real `re2` windows: both online, the other player visibly walking
  in each window, a pushed barrel tumbling in the watcher's window, server killed and restarted (both clients
  reconnected by themselves), one client killed without a goodbye (server timed it out, the other saw it go).

## Known limits (deliberately not done)
No interest management (everyone gets every player and changed prop), no encryption or real authentication
(the resume token is only unguessable by accident), no lag compensation (nothing is hit-tested over the network
yet), players do not collide with each other, at most 8 players. RTT is
measured at the client's frame rate (~1 frame of quantisation, so loopback reads 8-15 ms). Prediction and the
server use the platform's libm trig, so a Windows client against a Linux server may correct by millimetres
(hazard 3 in ADR 0014; ADR 0021 switched the simulation maths to `libm`). Weapons and pick-up were not networked when this was written:
they are now (ADR 0022). The server no longer links wgpu/alsa (ADR 0017).

## Consequences
- `MatchSim::checksum()` was the seed of deterministic replay / desync detection: see ADR 0021 (`red_server --record`,
  `red_engine2 replay`).
- Adding an authoritative feature = add it to `MatchSim`, then to `PlayerSnap`/`PropSnap` (bump
  `PROTOCOL_VERSION`), then test it in `net_e2e`.
