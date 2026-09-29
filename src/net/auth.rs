//! Who may join and who sent this datagram (ADR 0028, ADR 0044).
//!
//! **Permission to join** is the join key (`red_server --key`), on both transports: the client proves it knows the key with an HMAC and
//! never sends it. On QUIC the proof also covers TLS exporter keying material ([`join_proof_bound`]), so it is only valid on the
//! connection it was made for. **Server identity** is a separate matter: the QUIC transport verifies the server's certificate
//! (`net::quic`); knowing the join key says nothing about which server you reached.
//!
//! On the **development UDP transport** only (authenticated, not encrypted):
//! 1. **Address cookie.** A `Hello` with no valid cookie is answered with a small `Challenge` carrying one. The cookie is
//!    `HMAC(server secret, source address || client nonce || epoch)`, so the server keeps *no state* for someone who has only said
//!    Hello, and a spoofed source address gets a reply smaller than its request (no amplification) that it cannot answer.
//! 2. **Session key + tags.** Both sides derive `HMAC(key, client nonce || cookie)` and append an 8-byte tag (truncated HMAC of the
//!    direction and the whole datagram) to every datagram after the handshake.
//!
//! Development UDP does not hide traffic, and a short join key can be guessed offline by someone who recorded its handshake. QUIC removes
//! both problems (the handshake is encrypted; the proof is bound to a key exchange the eavesdropper does not have). Primitives are the
//! maintained ones in `crate::crypto`; secrets come from the OS CSPRNG.

use crate::crypto::{ct_eq, hmac_sha256, hmac_sha256_parts, Digest};
use std::net::SocketAddr;

/// Bytes of tag appended to every authenticated datagram.
pub const TAG_LEN: usize = 8;
/// Bytes of a join proof.
pub const PROOF_LEN: usize = 16;
/// How long (seconds) one cookie epoch lasts; a cookie is valid for its own and the next epoch.
pub const COOKIE_EPOCH_SECS: u64 = 10;

/// Which way a datagram travels; part of every tag so a captured client packet cannot be replayed as a server packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Client to server.
    ToServer = 1,
    /// Server to client.
    ToClient = 2,
}

/// The key for one session (one join).
#[derive(Clone, PartialEq, Eq)]
pub struct SessionKey(Digest);

impl std::fmt::Debug for SessionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionKey(..)") // never print key material into a log
    }
}

impl SessionKey {
    /// The key both sides derive once they hold `client_nonce` and the server's `cookie`.
    pub fn derive(join_key: &[u8], client_nonce: u64, cookie: u64) -> SessionKey {
        let mut m = Vec::with_capacity(32);
        m.extend_from_slice(b"red-session-v8");
        m.extend_from_slice(&client_nonce.to_le_bytes());
        m.extend_from_slice(&cookie.to_le_bytes());
        SessionKey(hmac_sha256(join_key, &m))
    }

    fn tag(&self, dir: Direction, packet: &[u8]) -> [u8; TAG_LEN] {
        let mut m = Vec::with_capacity(packet.len() + 1);
        m.push(dir as u8);
        m.extend_from_slice(packet);
        let full = hmac_sha256(&self.0, &m);
        let mut t = [0u8; TAG_LEN];
        t.copy_from_slice(&full[..TAG_LEN]);
        t
    }

    /// Appends the tag for `packet` (which must be the whole datagram so far).
    pub fn sign(&self, dir: Direction, packet: &mut Vec<u8>) {
        let t = self.tag(dir, packet);
        packet.extend_from_slice(&t);
    }

    /// Checks and strips the tag: `Some(body)` when it is genuine, `None` for a forged, truncated or wrong-direction datagram.
    pub fn verify<'a>(&self, dir: Direction, packet: &'a [u8]) -> Option<&'a [u8]> {
        let body_len = packet.len().checked_sub(TAG_LEN)?;
        let (body, tag) = packet.split_at(body_len);
        ct_eq(&self.tag(dir, body), tag).then_some(body)
    }
}

/// The proof a client sends on development UDP to show it knows `join_key` without sending it.
pub fn join_proof(join_key: &[u8], client_nonce: u64, cookie: u64, map_hash: u32, version: u16) -> [u8; PROOF_LEN] {
    truncate(hmac_sha256_parts(
        join_key,
        &[b"red-join-v8", &client_nonce.to_le_bytes(), &cookie.to_le_bytes(), &map_hash.to_le_bytes(), &version.to_le_bytes()],
    ))
}

/// The proof on QUIC: as [`join_proof`], plus the connection's TLS exporter keying material (`binding`), so a proof captured or
/// relayed from one connection is worthless on another.
pub fn join_proof_bound(join_key: &[u8], binding: &[u8; 32], client_nonce: u64, cookie: u64, map_hash: u32, version: u16) -> [u8; PROOF_LEN] {
    truncate(hmac_sha256_parts(
        join_key,
        &[b"red-join-v8-quic", binding, &client_nonce.to_le_bytes(), &cookie.to_le_bytes(), &map_hash.to_le_bytes(), &version.to_le_bytes()],
    ))
}

fn truncate(full: Digest) -> [u8; PROOF_LEN] {
    let mut p = [0u8; PROOF_LEN];
    p.copy_from_slice(&full[..PROOF_LEN]);
    p
}

/// Whether `given` is the right proof (`binding` = the QUIC connection's exporter, `None` on development UDP).
#[allow(clippy::too_many_arguments)]
pub fn proof_matches(
    join_key: &[u8],
    binding: Option<&[u8; 32]>,
    client_nonce: u64,
    cookie: u64,
    map_hash: u32,
    version: u16,
    given: &[u8; PROOF_LEN],
) -> bool {
    let want = match binding {
        Some(b) => join_proof_bound(join_key, b, client_nonce, cookie, map_hash, version),
        None => join_proof(join_key, client_nonce, cookie, map_hash, version),
    };
    ct_eq(&want, given)
}

/// Makes and checks the stateless address cookies.
pub struct CookieJar {
    secret: [u8; 32],
}

impl CookieJar {
    /// A jar with a fresh 256-bit secret from the OS CSPRNG. `Err` when the OS cannot provide one (the server then refuses to start).
    pub fn new() -> Result<CookieJar, String> {
        let mut secret = [0u8; 32];
        crate::crypto::fill_random(&mut secret)?;
        Ok(CookieJar { secret })
    }

    /// A jar with a given secret (tests).
    pub fn with_secret(secret: [u8; 32]) -> CookieJar {
        CookieJar { secret }
    }

    fn make_for_epoch(&self, addr: SocketAddr, client_nonce: u64, epoch: u64) -> u64 {
        let mut m = Vec::with_capacity(40);
        match addr.ip() {
            std::net::IpAddr::V4(v) => m.extend_from_slice(&v.octets()),
            std::net::IpAddr::V6(v) => m.extend_from_slice(&v.octets()),
        }
        m.extend_from_slice(&addr.port().to_le_bytes());
        m.extend_from_slice(&client_nonce.to_le_bytes());
        m.extend_from_slice(&epoch.to_le_bytes());
        let d = hmac_sha256(&self.secret, &m);
        u64::from_le_bytes([d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]]).max(1)
    }

    /// The cookie for this client at `now_secs` (seconds since the server started). Never `0` (`0` means "no cookie yet").
    pub fn make(&self, addr: SocketAddr, client_nonce: u64, now_secs: u64) -> u64 {
        self.make_for_epoch(addr, client_nonce, now_secs / COOKIE_EPOCH_SECS)
    }

    /// Whether `cookie` was issued to this address and nonce recently (this epoch or the one before).
    pub fn valid(&self, addr: SocketAddr, client_nonce: u64, cookie: u64, now_secs: u64) -> bool {
        let e = now_secs / COOKIE_EPOCH_SECS;
        cookie != 0 && (cookie == self.make_for_epoch(addr, client_nonce, e) || (e > 0 && cookie == self.make_for_epoch(addr, client_nonce, e - 1)))
    }
}

/// A random 32-hex-digit join key (128 bits from the OS CSPRNG); what `--key auto` prints.
pub fn random_key() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    crate::crypto::fill_random(&mut bytes)?;
    Ok(crate::crypto::hex(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(port: u16) -> SocketAddr {
        SocketAddr::from(([203, 0, 113, 9], port))
    }

    #[test]
    fn a_signed_packet_verifies_and_any_change_or_the_wrong_direction_or_key_fails() {
        let key = SessionKey::derive(b"secret", 1, 2);
        let mut p = b"hello world datagram".to_vec();
        key.sign(Direction::ToServer, &mut p);
        assert_eq!(p.len(), 20 + TAG_LEN);
        assert_eq!(key.verify(Direction::ToServer, &p), Some(&p[..20]));
        assert_eq!(key.verify(Direction::ToClient, &p), None, "a client packet replayed as a server packet");
        assert_eq!(SessionKey::derive(b"other", 1, 2).verify(Direction::ToServer, &p), None, "wrong join key");
        assert_eq!(SessionKey::derive(b"secret", 1, 3).verify(Direction::ToServer, &p), None, "another session's key");
        for i in 0..p.len() {
            let mut q = p.clone();
            q[i] ^= 1;
            assert_eq!(key.verify(Direction::ToServer, &q), None, "a flipped bit at {i} must fail");
        }
        assert_eq!(key.verify(Direction::ToServer, &p[..TAG_LEN - 1]), None, "shorter than a tag");
        assert_eq!(key.verify(Direction::ToServer, &[]), None);
    }

    #[test]
    fn the_join_proof_needs_the_key_and_is_bound_to_this_handshake() {
        let good = join_proof(b"k", 10, 20, 0xabcd, 3);
        assert!(proof_matches(b"k", None, 10, 20, 0xabcd, 3, &good));
        assert!(!proof_matches(b"K", None, 10, 20, 0xabcd, 3, &good), "wrong key");
        assert!(!proof_matches(b"k", None, 11, 20, 0xabcd, 3, &good), "replayed with another client nonce");
        assert!(!proof_matches(b"k", None, 10, 21, 0xabcd, 3, &good), "replayed with another cookie");
        assert!(!proof_matches(b"k", None, 10, 20, 0xabce, 3, &good), "another map");
        assert!(!proof_matches(b"k", None, 10, 20, 0xabcd, 4, &good), "another protocol version");
        // Bound to a QUIC connection: only that connection's exporter verifies it, and an unbound proof does not pass as a bound one.
        let (b1, b2) = ([1u8; 32], [2u8; 32]);
        let bound = join_proof_bound(b"k", &b1, 10, 20, 0xabcd, 3);
        assert!(proof_matches(b"k", Some(&b1), 10, 20, 0xabcd, 3, &bound));
        assert!(!proof_matches(b"k", Some(&b2), 10, 20, 0xabcd, 3, &bound), "relayed to another connection");
        assert!(!proof_matches(b"k", Some(&b1), 10, 20, 0xabcd, 3, &good), "an unbound proof on QUIC");
        assert!(!proof_matches(b"k", None, 10, 20, 0xabcd, 3, &bound));
    }

    #[test]
    fn cookies_belong_to_an_address_a_nonce_and_a_time() {
        let jar = CookieJar::new().unwrap();
        let c = jar.make(addr(1000), 77, 5);
        assert_ne!(c, 0);
        assert!(jar.valid(addr(1000), 77, c, 5));
        assert!(jar.valid(addr(1000), 77, c, 5 + COOKIE_EPOCH_SECS), "still good one epoch later");
        assert!(!jar.valid(addr(1000), 77, c, 5 + 2 * COOKIE_EPOCH_SECS), "expired after two epochs");
        assert!(!jar.valid(addr(1001), 77, c, 5), "another source port (a spoofer)");
        assert!(!jar.valid(addr(1000), 78, c, 5), "another client nonce");
        assert!(!jar.valid(addr(1000), 77, 0, 5), "0 is never a cookie");
        let other = CookieJar::with_secret([9; 32]);
        assert!(!other.valid(addr(1000), 77, c, 5), "another server's secret");
    }

    #[test]
    fn random_keys_are_32_hex_digits_and_differ() {
        let (a, b) = (random_key().unwrap(), random_key().unwrap());
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn a_session_key_never_prints() {
        assert_eq!(format!("{:?}", SessionKey::derive(b"secret", 1, 2)), "SessionKey(..)");
    }
}
