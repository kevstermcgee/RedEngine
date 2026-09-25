# 0028. Authenticated datagrams and join keys (protocol v3)
Status: accepted

## Context
Protocol v2 identified a client by its source address alone and accepted a `Hello` from anyone. On a public UDP port that means:
a spoofed or off-path packet can inject an `Input`, a `Bye` or a `Snapshot` into a session; a flood of spoofed `Hello`s makes the
server hold state (and a bigger reply than the request is an amplifier); a leaked resume token is a player takeover; and there is
no way to keep strangers out of a private game. A comparison with BlueEngine (which sends its join key in the clear and admits
"no cryptographic authentication") showed the same gap there. What we can do with no dependencies is *authentication*; we cannot
do confidentiality without a key-exchange library, and we do not claim to.

## Decision
`src/crypto.rs` (SHA-256, HMAC-SHA256, constant-time compare, checked against the published vectors) and `src/net/auth.rs`:
1. **Address cookie.** A `Hello` with no valid cookie gets a small `Challenge` (a cookie = `HMAC(server secret, address || client
   nonce || epoch)`), so an unproven address costs the server no state and gets a reply *smaller* than its request.
2. **Join key.** `red_server --key K` (or `--key auto`, 128 random bits) makes the client prove it knows `K`: the second `Hello`
   carries `HMAC(K, nonce || cookie || map hash || version)`, never `K`. A client with a key refuses an open server
   (`ServerIsOpen`: it may be an impostor); a keyless client learns from the `Challenge` that a key is needed (`NeedsKey`).
3. **Session key + tags.** Both sides derive `HMAC(K, nonce || cookie)`; every datagram after the handshake (`Input`, `Lobby`,
   `Bye`, `Welcome`, `Snapshot`, `Status`) ends with an 8-byte truncated HMAC over the direction and the datagram. The server and
   client verify the tag before decoding: a forged, corrupted, replayed-into-another-session or reflected packet is dropped unread.
4. `Reject`, `Challenge` and `NoSession` are untagged (there is no key yet, or no session). A client only believes them while
   handshaking, and believes `NoSession` only when a live session has already gone quiet, so a forger cannot restart a healthy one.
5. Parked (recently disconnected) players hold their player id only softly, so a crowd of drop-outs cannot lock newcomers out.

## Consequences
Cost: 8 bytes per datagram and one extra round trip at join (a worst snapshot is 1205 + 8 bytes, tested against the 1400 limit).
`tests/net_auth.rs` proves forgery, replay, bit-flip, wrong-key and impersonation cases in both directions; `tests/net_abuse.rs`
proves a 300-source join flood creates no session.
**Not provided:** payload confidentiality (traffic is readable), and protection of a *short* join key from an eavesdropper who records
the handshake and guesses offline (use `--key auto`). On an **open** server the session key is derivable by anyone who sees the
handshake, so tags then only stop blind attackers, which is most of them. Transport encryption would need a TLS/QUIC library and is a
separate decision (the reference Feta build uses QUIC for that reason).
