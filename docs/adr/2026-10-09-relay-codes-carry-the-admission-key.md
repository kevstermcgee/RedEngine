# 2026-10-09. Relay codes carry the admission key
Status: accepted
Summary: A relayed game is joined with the six-character rendezvous code plus the host's secret join key in one code (H3PQXR-K7Q2-MZ4P-WTXA); the key never reaches the relay, so a guessed or leaked code alone admits nobody

## Context
The graphical HOST button always started its server with a fresh join key (`PublicOptions::key`), and the lobby showed the relay's six-character
code first. The JOIN button resolved that code against the relay and then connected with `join_key = None`, so the client's handshake stopped
at `RejectReason::NeedsKey`: the one path the screen advertised could not work, while the long direct code
(`HOST:PORT#sha256:...#key`) that did carry the key was shown second and truncated by the lobby's label to an ellipsis. Two things were found
on the way and are the same kind of defect (a credential or a lease that silently does not survive the hop):

- the relay read a host's periodic `Register` keepalive, sent from the host's own registered address, as an id-framed joiner packet and
  dropped it, so **every code died five minutes after hosting began** however busy the match was;
- a host that quit left its code resolving for the rest of its lease, handing latecomers a code that led to nobody.

The options for the admission secret were (a) store it at the relay and return it from `Resolve`, (b) let the relay code alone admit, (c) carry
it inside the code the host shares and never show it to the relay. (a) makes the key worth exactly as much as the six characters: anyone who guesses
or overhears a code resolves it and receives the key, and the relay (which already sees every resolve) holds every game's key. (b) is the
same with no key at all: six characters from a 31-letter alphabet (about 30 bits) as the only thing between a stranger and the match.

## Decision
(c). A short code is two parts with different jobs, `net::relay::ShortJoin`:

- the **rendezvous code** (6 characters) is what the relay looks up. It is not a secret and never was: it is spoken aloud, and the relay sees it;
- the **admission key** (`net::relay::generate_join_key`, 12 characters of the same unambiguous alphabet, about 59 random bits) is the game
  server's join key, `ServerConfig::join_key`. A client never sends it: it proves it knows it with an HMAC bound to the TLS exporter of the QUIC
  connection (ADR 0028, 0044). The relay forwards ciphertext and learns neither the key nor the proof.

The host shows `H3PQXR-K7Q2-MZ4P-WTXA` (`LocalHost::share_codes`, which also lists the direct codes after it, each carrying the same key). JOIN is one
library call, `net::join::JoinTarget::parse` then `client_config`, used by the graphical client and by `tests/net_join_flow.rs`: it resolves the
relay part, pins the identity the relay returns (never joining without one, even when the relay is on the loopback), and puts the key part into
`ClientConfig::join_key`. A host that asks for no key shares the bare six characters. `red_server --relay` prints the same combined code; a key a
short code cannot carry (punctuation) is said so on the console and the direct join remains.

Reconnection is explicit and has three tiers. A dropped connection is retried automatically over the same relay pairing for as long as the relay
keeps it (`DEFAULT_PAIR_IDLE_TIMEOUT`, now ten minutes: the claim that locks a pairing is single-use, so a client the relay has forgotten can only JOIN
again) and the host parks the player's place for the resume token. After that the client says "trying to reconnect" and the player joins again with the same code
if it is still live. A code stops being live when its host leaves (`RelayMessage::Unregister`, sent when `HostBridge` is dropped, which also tears down
that host's pairings) or when its lease runs out (`RelayServerOptions::registration_lease`, five minutes without a keepalive; the keepalive now
refreshes it).

## Consequences
- A guessed, overheard or leaked six-character code admits nobody: the match needs the key too, and guessing a key is limited by the server's join
  rate limit to tens of attempts per second against about 2^59 possibilities. A code with the key in it is as strong as the key; share it like a password.
- The relay is still trusted for one thing: telling the joiner which identity to pin (the fingerprint travels through `Resolved`). A malicious relay
  could substitute its own identity and man-in-the-middle a joiner, collecting a key proof it could then try to brute-force offline. Players who do
  not trust the relay should use the direct code, whose fingerprint comes from the host, not from the relay. This is not new; this ADR documents it.
- A host with no key (`Key::None`, a `red_server` without `--key`) is open to anyone holding the six-character code. Hosting from the game never does that.
- The lobby shows a 21-character code instead of 6 (the label shrinks to fit; `ui::killchain` has a test that it is never cut short). Typing it
  tolerates lower case, spaces or dashes between groups, and no separators at all.
- An old relay ignores `Unregister` (unknown tag) and still forgets a code at its lease; an old host's keepalive is still dropped by an old relay.
  Both sides need this build for the fixes, neither breaks with the other.
- Undo: `ShortJoin::parse` accepts the bare six characters, so a relay and clients that ignore the key part continue to work against a host with no key.
