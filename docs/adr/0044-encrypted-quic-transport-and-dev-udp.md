# 0044. An encrypted QUIC transport, with development UDP kept explicit
Status: accepted
Summary: Production multiplayer traffic is QUIC + TLS 1.3 (quinn + rustls) behind a transport boundary, with a pinned server identity, join keys bound to the TLS exporter and no downgrade; loopback keeps development UDP, and a public UDP bind must be asked for explicitly.

## Context
Red's multiplayer (ADR 0016, 0028) sent readable datagrams over UDP: join keys were proven by HMAC and datagrams tagged, but nothing
was encrypted, a short key could be guessed offline from a recorded handshake, clients had no way to know which server they reached,
the SHA-256/HMAC were hand-written, and secrets came from `RandomState` hashing rather than a CSPRNG. Threat model and baseline
measurements: `docs/analysis/2026-09-27-transport-threat-model.md`.

## Decision
- **Transport boundary** (`net::transport`): `ServerTransport` / `ClientTransport` move Red's unchanged binary messages; the server,
  client, replication, prediction and simulation code did not change shape. Peers stay keyed by the address a connection started from.
- **Production backend: QUIC DATAGRAM frames** (RFC 9221) in TLS 1.3 via `quinn` + `rustls` (ring provider), `net::quic`. Chosen over
  DTLS or a Noise layer because it brings address validation (Retry), congestion control, a maintained TLS stack, and a reliable
  stream for the rare oversized message, all in one audited-in-the-wild library, while keeping datagram (not stream) semantics for
  real-time state. TLS 1.3 only, no 0-RTT, ALPN `red/8`, BBR congestion control (CUBIC dropped snapshots at 15% bursty loss).
- **Server identity is separate from permission to join.** Identity: a deployment's certificate + key (`red_engine2 net-identity`,
  `red_server --tls-cert/--tls-key`), verified by clients against a pinned SHA-256 fingerprint or a CA bundle; no "accept anything"
  mode exists. Permission: the join key, proven by HMAC bound to the TLS exporter (`auth::join_proof_bound`), so a proof is
  worthless on another connection.
- **Fail closed, never downgrade.** A client configured for QUIC whose server identity fails stops with `RejectReason::ServerIdentity`
  and never retries or falls back to UDP. `red_server` refuses a non-loopback bind without an identity unless `--insecure-public-udp`;
  clients refuse a non-loopback server without `--server-fingerprint`/`--server-ca` unless `--dev-udp`. Loopback stays development UDP
  so tests, `play-local`, `net-test` and `perf` are unchanged.
- **Maintained primitives only.** `crypto.rs` wraps `sha2`, `hmac`, `subtle`, `getrandom`; cookie secrets, resume tokens, nonces and
  `--key auto` come from the OS CSPRNG.
- **Budgets are explicit.** Protocol v8. On QUIC the datagram budget is `max_datagram_size`, which follows the path MTU: it starts at
  about 1150 bytes (`initial_mtu(1200)`, QUIC's guaranteed floor) and grows with MTU discovery (1414 measured on loopback), versus UDP's
  fixed 1400. The server sizes each snapshot's prop list to it (`snapshot_prop_budget`; props that do not fit go next time, oldest first); a message still too
  large (a full rule state, 1350 bytes) is carried on a short unidirectional stream (at most 8 KB, 64 in flight). Inbound queue 1024,
  16 connections by default (`--max-connections`), handshake timeout 5 s, idle timeout 10 s.

## Consequences
Traffic is confidential and the server authenticated; wire bandwidth is about 1.6-1.9x UDP's (QUIC headers, AEAD tags, ACKs) at the
same message rate, well inside the 24 KB/s budget. Hosting now needs an identity (Docker mounts `/identity`; systemd reads
`/etc/red/identity`); v7 clients cannot join. Not done: client certificates, key rotation tooling, certificate revocation, QUIC
connection migration beyond what quinn does implicitly, 0-RTT, and an independent security review. Undo: the transport trait keeps both
backends; `Server::bind` is still development UDP.
