//! A blind rendezvous relay: a short code stands in for `HOST:PORT#sha256:<fingerprint>#key` when direct
//! connection is not possible (a home network behind carrier-grade NAT, where no port mapping can ever be
//! reached from outside — `docs/adr/0031-home-hosting-with-upnp.md` already says plainly that nothing fixes
//! that from the home side). A host registers and gets a code; a friend types the code instead of an address.
//!
//! The relay never touches QUIC/TLS content — it only ever forwards opaque bytes between two addresses it has
//! paired. ADR 0044's end-to-end security (encryption, server-identity pinning) is completely unchanged: the
//! relay sees ciphertext, exactly like any NAT box on the path already does. This module is the pure
//! protocol/bookkeeping (code generation, the tiny control-message format, which codes point at which hosts,
//! and when a stale registration expires) — no sockets, so it is unit-tested without any real networking.
//! `red_relay` (`src/bin/red_relay.rs`) does the actual I/O over one well-known public socket. A joining client's
//! `Resolve` and its real traffic almost always arrive from different local ports (the resolve round trip is a
//! tiny raw-UDP exchange *before* handing off to `net::quic`'s own client, which binds its own socket) — and a
//! public IP alone cannot stand in for "the player who resolved this code," since strangers behind the same
//! carrier-grade NAT or campus network share one. [`ClaimToken`] is the fix: `Resolved` hands back a one-time
//! secret, and the client presents it back in a `Claim`, from the exact socket its real traffic will then use,
//! before any of it — the relay locks that socket's address to the pairing only once it sees the matching token,
//! never by guessing from address or timing alone.

use crate::crypto::fill_random;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

/// Characters a code is made of: no `0`/`O`, `1`/`I`/`L` — nothing a person could misread aloud or by hand.
const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
/// How many characters a code has.
pub const CODE_LEN: usize = 6;
/// How long an un-refreshed registration lives before its code is freed for reuse.
pub const REGISTRATION_TIMEOUT: Duration = Duration::from_secs(300);

/// A short code, e.g. `H3PQXR`.
pub type RelayCode = [u8; CODE_LEN];

/// A one-time, per-resolution secret the relay hands a client in `Resolved` and the client must present back
/// verbatim in `Claim`, from the exact socket its real traffic will use. A public IP is not a player identity —
/// two strangers can share one (the same apartment, campus, or carrier-grade NAT this relay exists for) — so
/// nothing before this used IP address as a stand-in for "the player who just resolved this code" is trustworthy.
/// 64 random bits is far more than enough to make guessing one live token infeasible within its short pending
/// window ([`crate::net::relay_server::DEFAULT_PENDING_TIMEOUT`]).
pub type ClaimToken = [u8; 8];

/// A fresh random claim token. `Err` only if the OS CSPRNG itself fails (see `crypto::fill_random`), exactly like
/// [`generate_code`].
pub fn generate_token() -> Result<ClaimToken, String> {
    let mut token = [0u8; 8];
    fill_random(&mut token)?;
    Ok(token)
}

/// The code as text.
pub fn code_to_string(code: &RelayCode) -> String {
    String::from_utf8_lossy(code).into_owned()
}

/// A fresh random code. `Err` only if the OS CSPRNG itself fails (see `crypto::fill_random`).
pub fn generate_code() -> Result<RelayCode, String> {
    let mut raw = [0u8; CODE_LEN];
    fill_random(&mut raw)?;
    let mut code = [0u8; CODE_LEN];
    for (i, b) in raw.iter().enumerate() {
        code[i] = ALPHABET[*b as usize % ALPHABET.len()];
    }
    Ok(code)
}

/// Parses what a person typed or pasted: case-insensitive, ignoring spaces and dashes (some UI might show
/// `H3P-QXR` for readability). `None` means it is not shaped like a code at all — not every wrong guess needs a
/// specific reason, since the caller will ask the relay whether the code is live next.
pub fn parse_code(text: &str) -> Option<RelayCode> {
    let cleaned: String = text.chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    let upper = cleaned.to_ascii_uppercase();
    if upper.len() != CODE_LEN || !upper.bytes().all(|b| ALPHABET.contains(&b)) {
        return None;
    }
    let mut code = [0u8; CODE_LEN];
    code.copy_from_slice(upper.as_bytes());
    Some(code)
}

/// One control-protocol message. Only ever sent by an address not already part of a paired forwarding session
/// (`red_relay`'s main loop never re-parses a paired address's traffic as control messages, so there is no
/// ambiguity with the QUIC bytes a paired session actually forwards).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayMessage {
    /// "Host me": the relay should register the sender's address under a code (sent once, then repeated
    /// periodically as a keepalive for as long as the host is up — the host's own address is whatever the
    /// relay observed the packet come from, the same trick UPnP/STUN-shaped protocols already use). Carries the
    /// host's own TLS fingerprint (`sha256:<64 hex>`, see ADR 0044), if it has one, so a joining client can pin
    /// and verify it without a person ever seeing or typing it — a loopback-only development-UDP host (no
    /// `--tls-cert`) has none, and registers with `None`.
    Register { fingerprint: Option<String> },
    /// The relay's answer to `Register`: this is your code.
    Registered { code: RelayCode },
    /// "Connect me to this code": sent by a joining client.
    Resolve { code: RelayCode },
    /// The relay's answer to a successful `Resolve`: a one-time [`ClaimToken`] to present in `Claim`, sent from
    /// the exact socket real traffic will use, before anything else is forwarded. Carries the registered host's
    /// own fingerprint through, unchanged, for the same reason `Register` does.
    Resolved { fingerprint: Option<String>, token: ClaimToken },
    /// The relay's answer to a `Resolve` naming a code with no live host.
    CodeNotFound,
    /// "This socket is the one claiming the pending resolution for `token`": sent once, from the exact local
    /// socket about to carry real traffic, before any of it. Replaces address-based guessing entirely — see
    /// [`ClaimToken`].
    Claim { token: ClaimToken },
}

const TAG_REGISTER: u8 = 1;
const TAG_REGISTERED: u8 = 2;
const TAG_RESOLVE: u8 = 3;
const TAG_RESOLVED: u8 = 4;
const TAG_CODE_NOT_FOUND: u8 = 5;
const TAG_CLAIM: u8 = 6;

/// A fingerprint string is short (`sha256:` + 64 hex = 71 bytes) but this is still a generous ceiling, not the
/// exact length, so a future identity format does not need a wire change.
const MAX_FINGERPRINT_LEN: usize = 128;

fn encode_fingerprint(out: &mut Vec<u8>, fingerprint: &Option<String>) {
    match fingerprint {
        Some(f) => {
            let bytes = f.as_bytes();
            out.push(bytes.len().min(MAX_FINGERPRINT_LEN) as u8);
            out.extend_from_slice(&bytes[..bytes.len().min(MAX_FINGERPRINT_LEN)]);
        }
        None => out.push(0),
    }
}

fn decode_fingerprint(buf: &[u8]) -> Option<(Option<String>, &[u8])> {
    let (&len, rest) = buf.split_first()?;
    if len == 0 {
        return Some((None, rest));
    }
    let len = len as usize;
    if rest.len() < len {
        return None;
    }
    let (f, rest) = rest.split_at(len);
    Some((Some(String::from_utf8(f.to_vec()).ok()?), rest))
}

impl RelayMessage {
    /// One datagram's worth of bytes.
    pub fn encode(&self) -> Vec<u8> {
        match self {
            RelayMessage::Register { fingerprint } => {
                let mut out = vec![TAG_REGISTER];
                encode_fingerprint(&mut out, fingerprint);
                out
            }
            RelayMessage::Registered { code } => [&[TAG_REGISTERED][..], code].concat(),
            RelayMessage::Resolve { code } => [&[TAG_RESOLVE][..], code].concat(),
            RelayMessage::Resolved { fingerprint, token } => {
                let mut out = vec![TAG_RESOLVED];
                encode_fingerprint(&mut out, fingerprint);
                out.extend_from_slice(token);
                out
            }
            RelayMessage::CodeNotFound => vec![TAG_CODE_NOT_FOUND],
            RelayMessage::Claim { token } => [&[TAG_CLAIM][..], token].concat(),
        }
    }

    /// Parses a datagram as a control message, or `None` if it is not shaped like one (most likely: it is real
    /// forwarded traffic from an already-paired address, which the caller should not even be passing in here).
    pub fn decode(buf: &[u8]) -> Option<RelayMessage> {
        let (&tag, rest) = buf.split_first()?;
        match tag {
            TAG_REGISTER => {
                let (fingerprint, rest) = decode_fingerprint(rest)?;
                rest.is_empty().then_some(RelayMessage::Register { fingerprint })
            }
            TAG_REGISTERED if rest.len() == CODE_LEN => Some(RelayMessage::Registered { code: rest.try_into().ok()? }),
            TAG_RESOLVE if rest.len() == CODE_LEN => Some(RelayMessage::Resolve { code: rest.try_into().ok()? }),
            TAG_RESOLVED => {
                let (fingerprint, rest) = decode_fingerprint(rest)?;
                let token: ClaimToken = rest.try_into().ok()?;
                Some(RelayMessage::Resolved { fingerprint, token })
            }
            TAG_CODE_NOT_FOUND if rest.is_empty() => Some(RelayMessage::CodeNotFound),
            TAG_CLAIM if rest.len() == 8 => Some(RelayMessage::Claim { token: rest.try_into().ok()? }),
            _ => None,
        }
    }
}

struct Registration {
    host: SocketAddr,
    fingerprint: Option<String>,
    created: Instant,
}

/// Which codes point at which hosts. Pairing a resolved client to a forwarding socket is `red_relay`'s own
/// concern (it needs a real socket); this only ever answers "is this code still live, and for whom".
#[derive(Default)]
pub struct RelayTable {
    by_code: HashMap<RelayCode, Registration>,
    by_host: HashMap<SocketAddr, RelayCode>,
}

impl RelayTable {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `host`, reusing its existing code and refreshing its lease (and its fingerprint, in case the
    /// host was restarted with a new identity) if it is already registered (a repeated `Register` is how a host
    /// keeps its code alive — idempotent by design, not an error).
    /// `max` bounds *new* registrations only — a host already registered always succeeds in refreshing its own
    /// lease, even at the cap, since that never grows the table (task step: explicit admission limits, not a
    /// generic rewrite). This is a small, personal-scale relay, not public infrastructure: a full table is an
    /// expected, named condition (`Err`), not unbounded memory growth from spoofed or abandoned registrations.
    pub fn register(&mut self, host: SocketAddr, fingerprint: Option<String>, now: Instant, max: usize) -> Result<RelayCode, String> {
        if let Some(code) = self.by_host.get(&host).copied() {
            if let Some(r) = self.by_code.get_mut(&code) {
                r.created = now;
                r.fingerprint = fingerprint;
            }
            return Ok(code);
        }
        if self.by_code.len() >= max {
            return Err(format!("the relay is at its registration limit ({max}): try again shortly"));
        }
        for _ in 0..20 {
            let code = generate_code()?;
            if let std::collections::hash_map::Entry::Vacant(e) = self.by_code.entry(code) {
                e.insert(Registration { host, fingerprint, created: now });
                self.by_host.insert(host, code);
                return Ok(code);
            }
        }
        Err("could not find a free code (an astronomically unlikely run of collisions): try again".to_string())
    }

    /// The host registered under `code`, and its fingerprint if it has one, if the code is still live.
    pub fn resolve(&self, code: &RelayCode) -> Option<(SocketAddr, Option<String>)> {
        self.by_code.get(code).map(|r| (r.host, r.fingerprint.clone()))
    }

    /// Whether `addr` is a currently live registration's own address (B2/B4: traffic claiming to be a host's
    /// half of the relay<->host framing is only ever trusted from an address that actually registered as one).
    pub fn is_registered_host(&self, addr: SocketAddr) -> bool {
        self.by_host.contains_key(&addr)
    }

    /// Drops `host`'s registration outright (it told us it is leaving, or its forwarding socket died).
    pub fn unregister(&mut self, host: SocketAddr) {
        if let Some(code) = self.by_host.remove(&host) {
            self.by_code.remove(&code);
        }
    }

    /// Forgets every registration whose lease has not been refreshed within [`REGISTRATION_TIMEOUT`], returning
    /// the hosts dropped so `red_relay` can also tear down any live per-client forwarding sockets for them.
    pub fn expire(&mut self, now: Instant) -> Vec<SocketAddr> {
        let stale: Vec<RelayCode> =
            self.by_code.iter().filter(|(_, r)| now.saturating_duration_since(r.created) > REGISTRATION_TIMEOUT).map(|(c, _)| *c).collect();
        stale
            .into_iter()
            .filter_map(|code| {
                let r = self.by_code.remove(&code)?;
                self.by_host.remove(&r.host);
                Some(r.host)
            })
            .collect()
    }

    /// Live registrations right now.
    pub fn len(&self) -> usize {
        self.by_code.len()
    }

    /// Whether there are no live registrations.
    pub fn is_empty(&self) -> bool {
        self.by_code.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(port: u16) -> SocketAddr {
        format!("127.0.0.1:{port}").parse().unwrap()
    }

    #[test]
    fn codes_round_trip_and_ignore_case_spaces_and_dashes() {
        let code = generate_code().unwrap();
        assert_eq!(code.len(), CODE_LEN);
        assert!(code.iter().all(|b| ALPHABET.contains(b)));
        let text = code_to_string(&code);
        assert_eq!(parse_code(&text), Some(code));
        assert_eq!(parse_code(&text.to_ascii_lowercase()), Some(code));
        let dashed = format!(" {}-{} ", &text[..3], &text[3..]);
        assert_eq!(parse_code(&dashed), Some(code));
    }

    #[test]
    fn a_code_rejects_ambiguous_characters_and_wrong_lengths() {
        assert_eq!(parse_code("ABCDEO"), None, "O is not in the alphabet");
        assert_eq!(parse_code("ABCD1Z"), None, "1 is not in the alphabet");
        assert_eq!(parse_code("ABCDE"), None, "too short");
        assert_eq!(parse_code("ABCDEFG"), None, "too long");
    }

    #[test]
    fn messages_round_trip_and_junk_does_not_parse() {
        let code = generate_code().unwrap();
        let token = generate_token().unwrap();
        let fp = Some("sha256:abc123".to_string());
        for m in [
            RelayMessage::Register { fingerprint: None },
            RelayMessage::Register { fingerprint: fp.clone() },
            RelayMessage::Registered { code },
            RelayMessage::Resolve { code },
            RelayMessage::Resolved { fingerprint: None, token },
            RelayMessage::Resolved { fingerprint: fp, token },
            RelayMessage::CodeNotFound,
            RelayMessage::Claim { token },
        ] {
            assert_eq!(RelayMessage::decode(&m.encode()), Some(m));
        }
        assert_eq!(RelayMessage::decode(&[]), None);
        assert_eq!(RelayMessage::decode(&[TAG_REGISTER, 9]), None, "a fingerprint length byte of 9 with no bytes following");
        assert_eq!(RelayMessage::decode(&[TAG_RESOLVE, 1, 2, 3]), None, "Resolve's code is the wrong length");
        assert_eq!(RelayMessage::decode(&[TAG_CLAIM, 1, 2, 3]), None, "Claim's token is the wrong length");
        assert_eq!(RelayMessage::decode(&[200]), None, "not a known tag");
    }

    #[test]
    fn claim_tokens_are_random_and_fixed_length() {
        let a = generate_token().unwrap();
        let b = generate_token().unwrap();
        assert_ne!(a, b, "two tokens colliding would defeat the whole point of using one");
        assert_eq!(a.len(), 8);
    }

    #[test]
    fn registering_the_same_host_twice_reuses_its_code_and_refreshes_the_lease() {
        let mut t = RelayTable::new();
        let now = Instant::now();
        let code = t.register(addr(1), None, now, usize::MAX).unwrap();
        let again = t.register(addr(1), None, now + Duration::from_secs(1), usize::MAX).unwrap();
        assert_eq!(code, again);
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn two_hosts_get_different_codes_and_resolve_to_the_right_one_with_their_fingerprint() {
        let mut t = RelayTable::new();
        let now = Instant::now();
        let a = t.register(addr(1), Some("sha256:aaa".to_string()), now, usize::MAX).unwrap();
        let b = t.register(addr(2), None, now, usize::MAX).unwrap();
        assert_ne!(a, b);
        assert_eq!(t.resolve(&a), Some((addr(1), Some("sha256:aaa".to_string()))));
        assert_eq!(t.resolve(&b), Some((addr(2), None)));
        assert_eq!(t.resolve(&generate_code().unwrap()), None, "a code nobody registered");
        assert!(t.is_registered_host(addr(1)) && t.is_registered_host(addr(2)));
        assert!(!t.is_registered_host(addr(3)), "an address nobody registered");
    }

    #[test]
    fn expiry_drops_only_stale_registrations_and_frees_their_codes() {
        let mut t = RelayTable::new();
        let now = Instant::now();
        let stale = t.register(addr(1), None, now, usize::MAX).unwrap();
        let fresh = t.register(addr(2), None, now + REGISTRATION_TIMEOUT, usize::MAX).unwrap();
        let dropped = t.expire(now + REGISTRATION_TIMEOUT + Duration::from_secs(1));
        assert_eq!(dropped, vec![addr(1)]);
        assert_eq!(t.resolve(&stale), None, "the stale code is gone");
        assert_eq!(t.resolve(&fresh), Some((addr(2), None)), "the fresh one survives");
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn unregister_removes_a_host_on_request() {
        let mut t = RelayTable::new();
        let now = Instant::now();
        let code = t.register(addr(1), None, now, usize::MAX).unwrap();
        t.unregister(addr(1));
        assert_eq!(t.resolve(&code), None);
        assert!(t.is_empty());
    }

    #[test]
    fn a_full_table_refuses_a_new_registration_but_still_refreshes_an_existing_one() {
        // B4: an explicit, named limit instead of unbounded growth from spoofed or abandoned registrations.
        let mut t = RelayTable::new();
        let now = Instant::now();
        t.register(addr(1), None, now, 1).unwrap();
        let err = t.register(addr(2), None, now, 1).unwrap_err();
        assert!(err.contains("limit"), "{err}");
        assert_eq!(t.len(), 1);
        // The host already holding the one slot can still refresh its own lease at the cap.
        assert!(t.register(addr(1), None, now + Duration::from_secs(1), 1).is_ok());
    }
}
