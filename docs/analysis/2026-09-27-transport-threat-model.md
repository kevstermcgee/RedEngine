# Transport threat model and measurements (2026-09-27, ADR 0044)

Measured on one Windows 11 machine over loopback, debug builds (`opt-level` 1, dependencies 2), `examples/test_lab.json`, two players.
Numbers are from single runs: treat differences under ~10% as noise. This is an engineering record, **not an independent security audit**.

## Before: protocol v7 over UDP (ADR 0016, 0028)

| Asset / threat | v7 behaviour |
|---|---|
| Eavesdropper reads traffic (positions, names, lobby, rule state) | Readable: nothing encrypted |
| Eavesdropper learns a short join key | Possible: record one handshake, guess offline against the HMAC proof |
| Off-path attacker forges or injects datagrams | Stopped: per-session 8-byte HMAC tag (after the handshake) |
| On-path attacker on an *open* server | Session key derivable from the visible handshake: tags stop only blind attackers |
| Client reaches an impostor server | Undetectable: no server identity |
| Spoofed-source join flood / amplification | Stopped: stateless address cookie, Challenge smaller than Hello, Hello budget |
| Cryptographic primitives | Hand-written SHA-256/HMAC (vector-tested); secrets from `RandomState` hashing (not a CSPRNG) |
| Resource bounds | Packet ceiling 1400, bounded decoders, per-session token buckets, parked-player cap |

## After: protocol v8, QUIC + TLS 1.3 in production

| Threat | v8 on QUIC | v8 development UDP (loopback, or explicit `--insecure-public-udp`) |
|---|---|---|
| Eavesdropping | Encrypted (TLS 1.3 AEAD) | Readable, as before |
| Offline guessing of the join key | Not possible from a recording: the proof is bound to TLS exporter material | As before |
| Forgery / injection / replay | QUIC packet protection; replayed packets rejected by QUIC | HMAC tags, as before |
| Impostor server | Refused: certificate pinned by SHA-256 or verified to a CA; `ServerIdentity` rejection, no retry, no fallback | Undetectable (why it is loopback-only by default) |
| Relay of a join proof to another server | Useless: exporter differs per connection | Not prevented |
| Spoofed joins / amplification | QUIC Retry (stateless) before any connection state, then Red's cookie | Red's cookie, as before |
| Floods | Connection cap (16), handshake timeout 5 s, inbound queue 1024 (excess dropped, counted), stream budget 64 x 8 KB | As before |
| Primitives / randomness | rustls (ring); `sha2`/`hmac`/`subtle`; `getrandom` for cookie secret, tokens, nonces, `--key auto` | Same maintained primitives |

Remaining limits (not addressed): no client certificates (players are authorised only by the join key); no revocation or rotation
tooling for server identities; a stolen server key lets an attacker impersonate that server until clients change their pin; players
can still see everything the server sends them (no anti-cheat); QUIC metadata (packet sizes, timing, the server address) is visible;
denial of service by volume above the host's link capacity is out of scope; no fuzzing of the QUIC path beyond quinn's own.

## Datagram budgets (`protocol` v8)

The QUIC budget is `Connection::max_datagram_size`, which follows the path MTU. The transport starts at QUIC's guaranteed 1200-byte
UDP payload (`initial_mtu(1200)`: about 1150-byte datagrams) and MTU discovery raises it; on loopback it had reached 1414 bytes by the
time the unit test measured it. Real paths with tunnels or PPPoE may stay near the floor.

| Message | Size | Fits UDP (1400) | Fits QUIC at the ~1150-byte floor |
|---|---|---|---|
| Full snapshot, 8 players + 29 props | 1205 B (+8 tag on UDP) | yes | no: `protocol::snapshot_prop_budget` caps props (8 players leave room for 25 at 1150 B); the rest go in the next snapshot |
| Full rule state (16 vars, 256 hidden, 64 collision) | <= 1392 B (test-enforced) | yes | no: sent on a unidirectional stream (reliable; it is repeated state) |
| Status with a full roster, Welcome, Input, Hello | < 400 B | yes | yes |

## Measurements

`red_engine2 net-test examples/test_lab.json --profile all [--transport quic]`, 6 s per profile, two players. "Wire" is what the lossy
proxy carried (all transport overhead); "messages" is Red's own bytes. Baseline = the `main` build (v7) before this change.

| Link | v7 UDP (baseline): RTT, missed snapshots, B/s per client | v8 dev UDP: RTT, missed, messages / wire B/s | v8 QUIC: RTT, missed, messages / wire B/s |
|---|---|---|---|
| lan | 5-6 ms, 0%, 3958 | 5-6 ms, 0%, 3957 / 3956 | 5 ms, 0%, 3635 / 6372 |
| wifi | 16-18 ms, 1.7-2.6%, 3977 | 15-17 ms, 1.7-2.6%, 3977 / 3976 | 17 ms, 0.8-1.7%, 3654 / 6392 |
| 4g | 51-55 ms, 3.4-5.1%, 3988 | 53-56 ms, 3.4-5.1%, 3987 / 3985 | 48-50 ms, 2.1-2.8%, 3724 / 6784 |
| bad | 107-108 ms, 6.8-7.5%, 4022 | 100-103 ms, 7.5-13.9%, 4033 / 4031 | 100-103 ms, 6.2-8.8%, 3728 / 7224 |
| awful | 183-204 ms, 30.9-32.8%, 4141 | 173-210 ms, 30.6-32.4%, 4127 / 4126 | 170-183 ms, 28.6-33.0%, 4003 / 7981 |

Every profile passes every `net-test` check on both transports (no disconnects, prediction ends on the server's position,
corrections within limits, remote players glide, bandwidth under 24 KB/s). With quinn's default CUBIC controller the QUIC `awful`
run failed (one client missed 48% of snapshots and reconnected: loss-based congestion control shrinks the window under random loss and
QUIC drops datagrams at the sender); BBR fixed it, hence `congestion_controller_factory(BbrConfig)`.

Process level (`red_server` + two `red_bot`, open play, 14 s): QUIC server 16.8 MB working set, 8 threads (two tokio workers plus the
game loop), 0.69 s CPU; development UDP 11.2 MB, 6 threads, 0.64 s CPU. Both clients: 240 snapshots, 0 missed, RTT ~3 ms.

`tests/net_quic.rs` (loopback, debug): two bots finish a coin-and-goal rules game through the `bad` proxy in 4.3-4.5 s with RTT
~100-111 ms and 4-7% measured loss; reconnection after a 1.5 s-plus outage resumes the same player 1.5 s after the network returns;
shutdown reaches the client in ~130 ms (before this change's CONNECTION_CLOSE handling: the 2 s client timeout); 5 connection attempts
against a limit of 2 give 2 connections and 3 refusals; 20,000 unread datagrams leave 1024 queued and 18,976 dropped.

## Migration

- Protocol 7 -> 8: v7 clients and servers are rejected (`Version`). Traces are unaffected.
- Hosting: make an identity (`red_engine2 net-identity --out DIR`) and pass `--tls-cert/--tls-key` (env `RED_TLS_CERT`/`RED_TLS_KEY`);
  give players the printed `--server-fingerprint`. The Docker image expects `/identity`; the systemd unit `/etc/red/identity`. A
  public server without an identity now refuses to start; `--insecure-public-udp` restores the old behaviour for a trusted LAN.
- Clients: `re2 --connect HOST --server-fingerprint sha256:...` (or `RE2_SERVER_FINGERPRINT`), `red_bot --server-fingerprint`; a
  loopback server needs nothing; `--dev-udp` joins a development server elsewhere.
- Library: `Server::bind` is development UDP; production is `Server::with_transport(cfg, sim, Box::new(QuicServer::bind(..)))`.
  `ClientConfig::new` is development UDP; production is `ClientConfig::quic(..)`. `CookieJar::new` and `auth::random_key` now return
  `Result` (the OS RNG can fail), `auth::proof_matches` takes the channel binding.
