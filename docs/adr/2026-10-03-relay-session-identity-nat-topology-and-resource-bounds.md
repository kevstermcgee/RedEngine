# 2026-10-03. Relay session identity, NAT topology, and resource bounds, hardened
Status: accepted
Summary: Short-lived claim tokens replace IP-only pending association, relay-to-host traffic is reframed to travel over the one socket a host's own registration already opened a NAT mapping for, admission is explicitly bounded, and the relay's trust model is stated rather than assumed.

## Context
A security/correctness review of the relay (ADR 2026-10-02) found its `pending` resolution table keyed by IP
address alone, consumed by whichever real packet arrived first from that IP. A public IP is not a player
identity — two strangers behind the same household, campus, or carrier-grade NAT share one, which is exactly the
population this relay exists to serve. Two confirmed failure modes followed directly: two players resolving the
*same* code from the same IP would have one silently drop the other's traffic (the single pending slot only fits
one), and two players resolving *different* codes from the same IP, interleaved, could have one player's claimed
socket paired to the *other* player's host — a correctness failure, not just a dropped connection.

Separately, `RelayServer::lock_pairing` opened a fresh ephemeral UDP socket to reach each client's host, so a
host's traffic for different joiners arrived from different relay-side source ports. A host's own `Register`
only ever opens a NAT mapping for the one address (this relay's public socket) it sent that `Register` to;
anything stricter than full-cone NAT (port-restricted cone, symmetric — common, and not rare on the mobile and
some consumer networks this relay is meant to help) would silently drop inbound traffic arriving from any other
port. The relay could tell a host "you're online" and tell a client "resolved," while the actual game bytes
between them never arrived — a specific, plausible, silent failure for exactly the harder NAT types this exists
to work around.

Two more gaps surfaced alongside these: `HostBridge::spawn_forwarding` read everything arriving on its `control`
socket as forwardable game data, including the relay's own periodic `Registered` keepalive-ack reply — forwarding
protocol bytes into the real game server as if a player had sent them. And nothing bounded registrations, pending
resolutions, live pairings, or per-source request rate: an attacker (or just a bug) could grow any of these
without limit on what is meant to be a small, personal relay, not public infrastructure.

Last, the relay forwards a host's TLS fingerprint to a resolving client over its own unauthenticated, unencrypted
UDP control channel (`Resolved`). This is the honest trust boundary a short-code rendezvous has always had — the
client already investigated and confirmed its *consequence* is bounded (see Decision) rather than invisible.

## Decision
**Session identity (replaces IP-only pending).** `net::relay::ClaimToken` (8 random bytes) is minted by the relay
on every successful `Resolve` and returned in `Resolved`. A new `Claim { token }` control message, sent once from
the exact local socket about to carry real traffic — `net::quic::QuicClient::connect_claiming` binds that socket,
sends the claim, *then* hands the same socket to quinn, so the claiming address is guaranteed to be the one QUIC
actually uses — is the only way an unpaired address locks a pairing (`RelayServer::handle_packet`'s `Claim` arm).
Pending resolutions are now keyed by token, not IP: two players sharing an address each hold a distinct token, so
neither can claim or disturb the other's, regardless of which room they are joining or how their traffic
interleaves. `net::client::ClientTransportConfig::Quic` gained a `relay_claim: Option<Vec<u8>>` field threading
this through from `re2`'s `try_join` with no change to the direct (non-relay) join path.

**NAT topology (single relay-side socket for all host traffic).** `RelayServer` no longer opens a per-client
socket toward a host at all. Every pairing's host-bound bytes now travel over the relay's one public socket —
the same address the host's own `Register` already has a NAT mapping open for — prefixed with a 4-byte client id
(`[id: u32 LE][opaque bytes]`; `RelayServer::forward_from_host`/the client-paired fast path in `handle_packet`).
`HostBridge` mirrors this: it demultiplexes incoming id-framed datagrams to one local loopback socket per id
(not per remote address, since there is now only one remote address) and re-frames its replies with the same id.
This is additive framing around still-opaque bytes, not inspection of them — ADR 2026-10-02's "the relay never
touches QUIC/TLS content" is unchanged. A side effect: pairings no longer need a dedicated thread each (the
relay's single receive loop handles routing both directions via `pairs`/`by_id`), so idle pairings are now
reclaimed by the housekeeping tick like every other relay state, not by a thread noticing its own silence.
**Verified**: loopback tests confirm every pairing's host-bound traffic uses the one relay address, and a real
`QuicClient`/`QuicServer` handshake plus bidirectional application data pass through end to end
(`a_real_quic_handshake_and_bidirectional_traffic_pass_through_the_relay`). **Not verified**: actual
endpoint-dependent/port-restricted NAT behavior against a real device or a simulated network namespace — this
development environment has neither a spare NAT device nor the privilege to create network namespaces
(`CAP_NET_ADMIN`). The fix is structurally sound (the host's NAT needs exactly one permission, which its own
`Register` traffic already creates) but that specific claim remains unverified in the harder-NAT case it targets;
testing it on a real restrictive-NAT network is the concrete remaining work.

**Trust model, stated explicitly (B3).** The relay is a **trusted, unauthenticated-channel rendezvous service**:
its control channel (`Register`/`Resolve`/`Resolved`/`Claim`) is plain UDP with no transport-level authentication
of its own, so an on-path attacker between a client and the relay (or a compromised relay operator) could in
principle substitute a different fingerprint in a `Resolved` reply. What bounds this, unchanged from ADR 0044: the
fingerprint is only ever a *pin* a client's own QUIC handshake independently verifies by the real cryptographic
handshake with whatever host is actually at the other end — a forged fingerprint makes the client trust the wrong
*identity*, it does not make the client trust an *unverified* connection, and the game traffic itself stays fully
encrypted and authenticated by TLS 1.3 regardless. Confirmed by this review: the engine's own join path
(`re2`'s `try_join` -> `ClientTransportConfig::choose`) already fails closed on a relay-resolved address with no
fingerprint (`server.ip().is_loopback()` is false for any real relay address, so the catch-all arm refuses rather
than silently choosing `DevUdp`) — this was already correct, not a new fix. One real gap found and fixed:
`red_server --relay` combined with no `--tls-cert`/`--tls-key` used to print a warning and start anyway, bridging
a loopback-only, identity-less dev server onto the open internet for anyone willing to connect with the existing,
separate `--dev-udp` opt-in; it now refuses to start at all (`src/bin/red_server.rs`), matching how `--dev-udp`
combined with a server identity is already refused. A genuinely stronger model — TLS-wrapping the control channel
itself, with the relay pinned the same way a game server is — is real future work, explicitly not done here: it
needs the relay to carry its own long-lived TLS identity and clients/hosts to pin it, which is a larger, separate
piece of infrastructure than this pass's scope (bounded correctness fixes, not a new PKI).

**Resource bounds (B4).** `RelayServerOptions` gained four explicit limits, all configurable, defaulting to
small-personal-relay sizes: `max_registrations` (64), `max_pending` (512), `max_pairings` (128), and
`max_requests_per_source_per_tick` (20, a simple per-IP sliding window reset each housekeeping tick). Each is
enforced where the corresponding resource is created (`RelayTable::register`'s own cap for new — not
refreshed — registrations; `handle_packet`'s `Resolve`/`Claim` arms for pending/pairings); at a cap, the relay
answers nothing rather than lying (no `CodeNotFound` for a code that is actually fine, just a busy relay) — the
caller's own request simply times out, an honest if generic failure. The rate limiter prunes its own stale
entries every tick so it cannot itself become the unbounded-memory vector it exists to prevent.
`HostBridge::forward_from_relay` now decodes before treating `control` traffic as forwardable: a `Registered`
keepalive ack is consumed as the control reply it is, never forwarded into the real game server.

## Consequences
Two players sharing a public IP — the exact population a CGNAT-focused relay exists to serve — can now join the
same room or different ones without one silently losing their connection or, worse, being paired to the wrong
host. A host behind a stricter NAT should now actually receive forwarded traffic (structurally argued and
loopback/real-QUIC tested; not yet confirmed against a real restrictive NAT — see above). The relay cannot grow
its memory use without bound from spoofed or abandoned traffic, and a keepalive ack can no longer leak into a
game server's own traffic stream. The trust model is now a written, specific claim instead of an implicit one:
operators and future readers can see exactly what a compromised network path or relay could and could not do.
Nothing here is a breaking change to the wire protocol's *purpose* — `Resolved` gained a `token` field and a new
`Claim` message type, both additive; a client or host built before this change would simply fail to complete a
join (the old flow has no `Claim` step), so a host and its joining clients must be upgraded together, same as any
other protocol version bump (`docs/upgrade-migrations.json`'s `protocol-version-lockstep` migration's own
reasoning applies here too, though this is the relay's own protocol, not the game wire protocol it covers). To
undo: the claim-token flow, the id-framing, the admission caps, and the `red_server --relay` hard refusal are all
localized to `net::relay`/`net::relay_server`/`net::quic`'s two new `QuicClient` constructors/`net::client`'s
`relay_claim` field — reverting means restoring IP-keyed pending, per-client relay sockets, and the softer warning,
none of which touch `net::server`/`net::transport`'s own, unrelated guarantees.
