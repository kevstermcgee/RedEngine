# 2026-10-02. A relay for players behind carrier-grade NAT, with a short join code
Status: accepted
Summary: red_relay forwards opaque bytes between a registered host and a resolving client by a 6-character code, so hosting/joining works even when UPnP cannot, without weakening ADR 0044's end-to-end encryption

## Context
Hosting Killchain for a friend failed: UPnP (ADR 0031) silently cannot open a port when the host's ISP uses
carrier-grade NAT — the router's own "public" address is itself private, so no mapping reaches the real internet,
and the engine's own docs already said so plainly ("nothing can fix that from here"). Separately, the existing
invite mechanism (`net::join_code::JoinCode`) is `HOST:PORT#sha256:<64-hex fingerprint>#key` — accurate, but
genuinely a "long complicated code," not something a casual player wants to read aloud or retype.

Both problems share one fix: something with a real, stable public address that both the host and a joining
player connect *outward* to, the way every "enter a code to play with a friend" game already works. Once that
exists, the human-facing code can shrink to something short, because the relay — not a raw address plus a
fingerprint — becomes the thing both sides already reach.

## Decision
`red_relay` (`src/bin/red_relay.rs`, thin CLI/signal shell) and `net::relay_server::RelayServer` (the actual
socket work, `src/net/relay_server.rs`) are a **blind** forwarder: they never touch QUIC/TLS content, only ever
relaying opaque bytes between two addresses once paired by a code. This leaves ADR 0044's entire security model
unchanged — the relay sees ciphertext, exactly like any NAT box on the path already does.

- **Protocol and bookkeeping** (`net::relay`, no sockets, unit-tested in isolation): `RelayTable` maps a
  6-character human-friendly code (`generate_code`, an alphabet without `0/O/1/I/L`) to a registered host address
  and its optional TLS fingerprint; `RelayMessage` is the tiny control protocol (`Register`, `Registered`,
  `Resolve`, `Resolved`, `CodeNotFound`) riding the same public UDP socket as real traffic.
- **Per-client distinct addressing**: a host can have many joiners at once, and `net::transport` already keys
  peers by the address a connection started from, so each accepted pairing gets its *own* small ephemeral socket
  on the relay used only to talk to that one host — the host sees every joiner as a distinct address, exactly as
  it does today, with no change to `net::server`/`net::quic`.
- **A client's `Resolve` and its real QUIC traffic may use different local ports**: a game client does a quick
  raw-UDP round trip to resolve a code *before* handing off to `net::quic`'s own client, which binds its own
  socket — reusing one exact port would mean reaching into QUIC endpoint internals for a small benefit. Instead a
  resolved code is "pending" for the client's IP for a short window; the first real packet from that IP locks the
  pairing to whatever port it actually arrived on.
- **The fingerprint still travels end-to-end, invisible to the person**: `Register`/`Resolved` optionally carry
  the host's own TLS fingerprint. A player only ever sees/types the 6-character code; `net::relay_server::resolve_code`
  (the client-side half) gets the fingerprint back from the relay and pins it exactly as if it had been pasted —
  ADR 0044's "fail closed, never downgrade" rule is not touched: a relayed connection with no fingerprint (a
  loopback dev-udp host, relayed) is still refused by the client's own existing rule, since the relay's address is
  never loopback.
- **The relay's own address is a hostname, not just a literal IP, resolved fresh on every use**
  (`net::relay_server::resolve_relay`, `std::net::ToSocketAddrs`) — `--relay`/`RE2_RELAY`/`PublicOptions::relay`
  all take a `HOST:PORT` string. A dynamic-DNS name (DuckDNS, say) is the point of this rather than a cosmetic
  nicety: a relay runs as a long-lived service on a residential connection whose IP can itself change, and
  caching one resolved address at startup would go stale exactly when a dynamic-DNS name is doing its job. The
  host's keepalive and every client's resolve each re-resolve independently, never reusing an old lookup.
- **Host-side bridging**: `net::relay_server::HostBridge` is the mirror image on the hosting machine — it
  registers once (repeating as a keepalive), then bridges every distinct address the relay forwards from to the
  real local game server over loopback, one small local socket per remote joiner, for the same per-peer-identity
  reason. Wired into both `red_server --relay HOST:PORT` and the in-process host path `net::host::LocalHost`
  (`PublicOptions::relay`, which Killchain's own HOST button already uses, ADR 0054) — a hosted game can stay
  loopback-only even while relayed, needing no public bind, UPnP or port forward of its own. `red_server --relay`
  without a TLS identity prints a clear warning instead of silently producing unjoinable games, since it would
  otherwise have no fingerprint to offer and ADR 0044's own client-side rule would just refuse every joiner.
- Reusable: the relay is not Killchain-specific — any game project's `red_server`/`LocalHost` hosting can use it.

## Consequences
Hosting and joining now work for players behind CGNAT or a UPnP-less router, which no engine-side UPnP
improvement could ever fix on its own. A host or player only ever needs a 6-character code — no address, no
64-character fingerprint, no port. Running a relay is one more thing to operate (a `red-relay.service` systemd
unit, `deploy/`, mirroring `red-server.service`); it adds one extra network hop and a small bandwidth cost
(trivial at casual player counts) compared to a direct connection, and it is one more process whose uptime a
hosted game now optionally depends on. Not done: NAT hole-punching as a lower-latency alternative tried before
falling back to relaying (deliberately — pure relay is simpler and fully robust at this scale; only worth revisiting
if latency or relay bandwidth actually becomes a problem); a public matchmaking/server-browser service (this is a
private rendezvous for one relay instance, not a discovery service); multi-region relays. To undo: `--relay`/
`PublicOptions::relay` are purely additive opt-ins — direct connect, UPnP and the long join code are completely
unchanged and keep working with no relay running at all.
