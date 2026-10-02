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
//! `red_relay` (`src/bin/red_relay.rs`) does the actual I/O: one well-known public socket clients and hosts talk
//! to, and one fresh small socket per paired client used only to talk to that client's host, so the host can
//! still tell multiple joiners apart by address exactly as it does today.

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
    /// The relay's answer to a successful `Resolve`: start sending your real traffic now, it will be forwarded.
    /// Carries the registered host's own fingerprint through, unchanged, for the same reason `Register` does.
    Resolved { fingerprint: Option<String> },
    /// The relay's answer to a `Resolve` naming a code with no live host.
    CodeNotFound,
}

const TAG_REGISTER: u8 = 1;
const TAG_REGISTERED: u8 = 2;
const TAG_RESOLVE: u8 = 3;
const TAG_RESOLVED: u8 = 4;
const TAG_CODE_NOT_FOUND: u8 = 5;

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
            RelayMessage::Resolved { fingerprint } => {
                let mut out = vec![TAG_RESOLVED];
                encode_fingerprint(&mut out, fingerprint);
                out
            }
            RelayMessage::CodeNotFound => vec![TAG_CODE_NOT_FOUND],
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
                rest.is_empty().then_some(RelayMessage::Resolved { fingerprint })
            }
            TAG_CODE_NOT_FOUND if rest.is_empty() => Some(RelayMessage::CodeNotFound),
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
    pub fn register(&mut self, host: SocketAddr, fingerprint: Option<String>, now: Instant) -> Result<RelayCode, String> {
        if let Some(code) = self.by_host.get(&host).copied() {
            if let Some(r) = self.by_code.get_mut(&code) {
                r.created = now;
                r.fingerprint = fingerprint;
            }
            return Ok(code);
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
        let fp = Some("sha256:abc123".to_string());
        for m in [
            RelayMessage::Register { fingerprint: None },
            RelayMessage::Register { fingerprint: fp.clone() },
            RelayMessage::Registered { code },
            RelayMessage::Resolve { code },
            RelayMessage::Resolved { fingerprint: None },
            RelayMessage::Resolved { fingerprint: fp },
            RelayMessage::CodeNotFound,
        ] {
            assert_eq!(RelayMessage::decode(&m.encode()), Some(m));
        }
        assert_eq!(RelayMessage::decode(&[]), None);
        assert_eq!(RelayMessage::decode(&[TAG_REGISTER, 9]), None, "a fingerprint length byte of 9 with no bytes following");
        assert_eq!(RelayMessage::decode(&[TAG_RESOLVE, 1, 2, 3]), None, "Resolve's code is the wrong length");
        assert_eq!(RelayMessage::decode(&[200]), None, "not a known tag");
    }

    #[test]
    fn registering_the_same_host_twice_reuses_its_code_and_refreshes_the_lease() {
        let mut t = RelayTable::new();
        let now = Instant::now();
        let code = t.register(addr(1), None, now).unwrap();
        let again = t.register(addr(1), None, now + Duration::from_secs(1)).unwrap();
        assert_eq!(code, again);
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn two_hosts_get_different_codes_and_resolve_to_the_right_one_with_their_fingerprint() {
        let mut t = RelayTable::new();
        let now = Instant::now();
        let a = t.register(addr(1), Some("sha256:aaa".to_string()), now).unwrap();
        let b = t.register(addr(2), None, now).unwrap();
        assert_ne!(a, b);
        assert_eq!(t.resolve(&a), Some((addr(1), Some("sha256:aaa".to_string()))));
        assert_eq!(t.resolve(&b), Some((addr(2), None)));
        assert_eq!(t.resolve(&generate_code().unwrap()), None, "a code nobody registered");
    }

    #[test]
    fn expiry_drops_only_stale_registrations_and_frees_their_codes() {
        let mut t = RelayTable::new();
        let now = Instant::now();
        let stale = t.register(addr(1), None, now).unwrap();
        let fresh = t.register(addr(2), None, now + REGISTRATION_TIMEOUT).unwrap();
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
        let code = t.register(addr(1), None, now).unwrap();
        t.unregister(addr(1));
        assert_eq!(t.resolve(&code), None);
        assert!(t.is_empty());
    }
}
