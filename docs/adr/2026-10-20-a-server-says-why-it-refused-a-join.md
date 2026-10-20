# 2026-10-20. A server says why it refused a join
Status: accepted
Summary: The server counts every refused Hello per reason (version, map, missing key, wrong key, full, other), shows the counters in its `stats:` line and stop line, and writes one `join refused (...)` line per reason per minute with the client's protocol version, map hash and a masked address; a missing key is inferred from a challenge nobody answered, because the server never sees that refusal.

## Context
Two people could not play Killchain together for weeks. The network was fine; the failures were software and operations, and the worst of them was invisible. On
2026-10-20 the dedicated server was still protocol 15 while the clients were 16 and 17, so it answered every join with `Reject(Version)`. Its journal read
"0 joins, 0 bad packets": `on_hello` bumped no counter and wrote no line for a refused `Hello` (only a wrong key was counted, and nothing logged it). The client
screen says "a different version" without saying which side is old, so the owner had nothing to look at on either end.

## Decision
Refusals are counted and said, with no change to the wire (the replies are the protocol's existing ones; a v4 client and the deployed protocol-17 relay and server
interoperate, `tests/net_refusals.rs`).
- `ServerStats` counts each reason: `rejected_version`, `rejected_wrong_map`, `bad_keys` (which existed), `rejected_needs_key`, `rejected_full`, `rejected_other`.
  `Server::refuse` is the one place a `Reject` is sent from `on_hello`; `Refusal::of` maps a `RejectReason` with a total `match`, so a new reason cannot be added
  without deciding what it counts as.
- The periodic `stats:` line keeps every field it had, in order, and appends `| joins N | rejected v1 m0 k0 b0 f0 o0 | throttled N`; the stop line carries the same
  counters. `red_engine2 servers` and `red-health` read the line by field name, so appended fields change nothing for them (a test parses the new line).
- Each reason writes **one line per minute** (`net::refusals`): `join refused (version): the client speaks protocol v16 and this server v17, so the client is the old
  side and must update; map unknown ...; peer 73.157.x.x`. It names the reason, the client's protocol version, its map hash and the peer masked to two octets. It
  never carries a key, a proof or a name. The refusals between two lines are counted into the next ("N more like it").
- **A map hash is only readable for the server's own protocol version.** `ClientMsg::decode` keeps just the version of another version's Hello (its layout is
  unknown), so a version refusal says "map unknown" rather than a made-up hash.
- **`NeedsKey` is inferred.** The server never sends that reply: a client challenged for a key it does not have gives up on its own (`RejectReason::NeedsKey` is
  "decided by the client"), which is exactly what a short code without its key did (the first of the weeks' failures). On a keyed server the first `Hello` is
  answered with a `Challenge`, and the server remembers up to 32 such handshakes until the second `Hello` arrives; one that nobody answered within 8 s counts as a
  `NeedsKey` (`k` in the line) and is logged as "gave up at the key challenge (usually a code without the key part)". The honest limit: a client that vanished for
  any other reason in those 8 s is counted the same way.
- `deploy/red-selfcheck.py` (every five minutes, `docs/HOSTING.md`) fails when these counters grow with no join in between, and compares the protocol the deployed
  server was built with (its `REVISION` file, read from git) with the newest Killchain release's.

## Consequences
The next version drift, key mistake or wrong map shows in the journal in the first minute, naming the side that must update, instead of in a week of silence. The
cost is one pending-challenge list of at most 32 entries and a few counters per tick-second sweep. An operator who greps `join refused` gets the story; one who
greps `stats:` gets the totals. To undo: remove `net/refusals.rs`, the `refuse` helper and the appended fields; nothing else depends on them but the self-check,
which tolerates their absence ("this build predates the reject counters"). A reason could later travel to the client (so its screen says which side is old), which is
a wire change and was out of scope here.
