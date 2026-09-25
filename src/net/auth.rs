//! Who may join and who sent this datagram (ADR 0028). Authentication, **not encryption**: a Red datagram is readable on the wire, but it
//! cannot be forged, replayed into another session or injected by someone who is not in the conversation.
//!
//! 1. **Address cookie.** A `Hello` with no valid cookie is answered with a small `Challenge` carrying one. The cookie is
//!    `HMAC(server secret, source address ‖ client nonce ‖ epoch)`, so the server keeps *no state* for someone who has only said
//!    Hello, and a spoofed source address gets a reply smaller than its request (no amplification) that it cannot answer.
//! 2. **Join key.** A server started with a key (`--key`) makes the client prove it knows it: the `Hello` carries
//!    `HMAC(key, client nonce ‖ cookie ‖ map hash ‖ version)`, never the key. A wrong or missing key is a `BadKey` rejection.
//! 3. **Session key + tags.** Both sides derive `HMAC(key, client nonce ‖ cookie)` and append an 8-byte tag (truncated HMAC of the
//!    direction and the whole datagram) to every datagram after the handshake. A forged `Input`, `Bye` or `Snapshot`, or a packet
//!    captured from another session, fails the tag and is dropped before it reaches the simulation.
//!
//! What it does not do: hide the traffic, or protect a join key that is a short word from an eavesdropper who records the handshake and
//! guesses offline. Use a long random key (`red_server --key auto` makes one). On an *open* server the session key is derivable by anyone
//! who sees the handshake, so tags then only stop blind (off-path) attackers, which is still most of them.

use crate::crypto::{ct_eq, hmac_sha256, Digest};
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
        m.extend_from_slice(b"red-session-v3");
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

/// The proof a client sends to show it knows `join_key` without sending it.
pub fn join_proof(join_key: &[u8], client_nonce: u64, cookie: u64, map_hash: u32, version: u16) -> [u8; PROOF_LEN] {
    let mut m = Vec::with_capacity(32);
    m.extend_from_slice(b"red-join-v3");
    m.extend_from_slice(&client_nonce.to_le_bytes());
    m.extend_from_slice(&cookie.to_le_bytes());
    m.extend_from_slice(&map_hash.to_le_bytes());
    m.extend_from_slice(&version.to_le_bytes());
    let full = hmac_sha256(join_key, &m);
    let mut p = [0u8; PROOF_LEN];
    p.copy_from_slice(&full[..PROOF_LEN]);
    p
}

/// Whether `given` is the right proof.
pub fn proof_matches(join_key: &[u8], client_nonce: u64, cookie: u64, map_hash: u32, version: u16, given: &[u8; PROOF_LEN]) -> bool {
    ct_eq(&join_proof(join_key, client_nonce, cookie, map_hash, version), given)
}

/// Makes and checks the stateless address cookies.
pub struct CookieJar {
    secret: [u8; 32],
}

impl CookieJar {
    /// A jar with a fresh secret made from `entropy` (the server's per-process random source).
    pub fn new(mut entropy: impl FnMut() -> u64) -> CookieJar {
        let mut secret = [0u8; 32];
        for chunk in secret.chunks_mut(8) {
            chunk.copy_from_slice(&entropy().to_le_bytes());
        }
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

/// A random 32-hex-digit join key (128 bits) from the OS-seeded hasher; what `--key auto` prints.
pub fn random_key(mut entropy: impl FnMut() -> u64) -> String {
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&entropy().to_le_bytes());
    bytes[8..].copy_from_slice(&entropy().to_le_bytes());
    crate::crypto::hex(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(port: u16) -> SocketAddr {
        SocketAddr::from(([203, 0, 113, 9], port))
    }

    fn counter() -> impl FnMut() -> u64 {
        let mut n = 0x9e37_79b9_7f4a_7c15u64;
        move || {
            n = n.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            n
        }
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
        assert!(proof_matches(b"k", 10, 20, 0xabcd, 3, &good));
        assert!(!proof_matches(b"K", 10, 20, 0xabcd, 3, &good), "wrong key");
        assert!(!proof_matches(b"k", 11, 20, 0xabcd, 3, &good), "replayed with another client nonce");
        assert!(!proof_matches(b"k", 10, 21, 0xabcd, 3, &good), "replayed with another cookie");
        assert!(!proof_matches(b"k", 10, 20, 0xabce, 3, &good), "another map");
        assert!(!proof_matches(b"k", 10, 20, 0xabcd, 4, &good), "another protocol version");
    }

    #[test]
    fn cookies_belong_to_an_address_a_nonce_and_a_time() {
        let jar = CookieJar::new(counter());
        let c = jar.make(addr(1000), 77, 5);
        assert_ne!(c, 0);
        assert!(jar.valid(addr(1000), 77, c, 5));
        assert!(jar.valid(addr(1000), 77, c, 5 + COOKIE_EPOCH_SECS), "still good one epoch later");
        assert!(!jar.valid(addr(1000), 77, c, 5 + 2 * COOKIE_EPOCH_SECS), "expired after two epochs");
        assert!(!jar.valid(addr(1001), 77, c, 5), "another source port (a spoofer)");
        assert!(!jar.valid(addr(1000), 78, c, 5), "another client nonce");
        assert!(!jar.valid(addr(1000), 77, 0, 5), "0 is never a cookie");
        let other = CookieJar::new({
            let mut n = 1u64;
            move || {
                n = n.wrapping_mul(31).wrapping_add(7);
                n
            }
        });
        assert!(!other.valid(addr(1000), 77, c, 5), "another server's secret");
    }

    #[test]
    fn random_keys_are_32_hex_digits_and_differ() {
        let mut e = counter();
        let (a, b) = (random_key(&mut e), random_key(&mut e));
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn a_session_key_never_prints() {
        assert_eq!(format!("{:?}", SessionKey::derive(b"secret", 1, 2)), "SessionKey(..)");
    }
}
