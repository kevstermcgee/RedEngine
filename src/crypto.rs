//! The engine's cryptographic primitives, all from maintained crates (ADR 0044): SHA-256 (`sha2`), HMAC-SHA256 (`hmac`), constant-time
//! comparison (`subtle`) and the operating system's CSPRNG (`getrandom`). This module is a thin, stable wrapper so callers (join proofs,
//! the development transport's datagram tags, address cookies, release fingerprints in `tools::package`) never pick primitives themselves.
//!
//! Nothing here encrypts. Confidentiality comes from the production transport (`net::quic`: QUIC with TLS 1.3), which uses `rustls`.
//! Everything in this file is graphics-free and part of the headless build.

use hmac::{Hmac, Mac};
use sha2::Digest as _;

/// A SHA-256 digest.
pub type Digest = [u8; 32];

/// An incremental SHA-256, for hashing a large file without holding it in memory.
#[derive(Clone, Default)]
pub struct Sha256(sha2::Sha256);

impl Sha256 {
    /// A fresh hasher.
    pub fn new() -> Self {
        Sha256(sha2::Sha256::new())
    }

    /// Feeds more bytes.
    pub fn update(&mut self, data: &[u8]) {
        self.0.update(data);
    }

    /// Finishes and returns the digest.
    pub fn finish(self) -> Digest {
        self.0.finalize().into()
    }
}

/// SHA-256 of `data`.
pub fn sha256(data: &[u8]) -> Digest {
    sha2::Sha256::digest(data).into()
}

/// HMAC-SHA256 of `message` under `key` (any key length).
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> Digest {
    hmac_sha256_parts(key, &[message])
}

/// HMAC-SHA256 of the concatenation of `parts` under `key`, without building the concatenation.
pub fn hmac_sha256_parts(key: &[u8], parts: &[&[u8]]) -> Digest {
    // `new_from_slice` accepts every key length for HMAC; the error arm is unreachable but must not panic a server.
    let Ok(mut mac) = <Hmac<sha2::Sha256> as Mac>::new_from_slice(key) else { return [0; 32] };
    for p in parts {
        mac.update(p);
    }
    mac.finalize().into_bytes().into()
}

/// Equality that takes the same time wherever the inputs differ (so a forged tag cannot be guessed a byte at a time).
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

/// Fills `out` from the operating system's CSPRNG. `Err` only when the OS has no randomness to give (then nothing secret may be made).
pub fn fill_random(out: &mut [u8]) -> Result<(), String> {
    getrandom::fill(out).map_err(|e| format!("the operating system's random number generator failed: {e}"))
}

/// A random 64-bit value from the OS CSPRNG (nonces, resume tokens), never `0` (`0` means "none" on the wire).
pub fn random_u64() -> Result<u64, String> {
    let mut b = [0u8; 8];
    fill_random(&mut b)?;
    Ok(u64::from_le_bytes(b).max(1))
}

/// Lower-case hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(DIGITS[(b >> 4) as usize] as char);
        s.push(DIGITS[(b & 15) as usize] as char);
    }
    s
}

/// Parses lower- or upper-case hex (`None` for an odd length or a non-hex digit).
pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_the_published_vectors() {
        assert_eq!(hex(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            hex(&sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn a_million_a_and_every_chunking_agree() {
        let big = vec![b'a'; 1_000_000];
        assert_eq!(hex(&sha256(&big)), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
        let mut h = Sha256::new();
        for chunk in big.chunks(37) {
            h.update(chunk);
        }
        assert_eq!(h.finish(), sha256(&big), "incremental hashing must equal one-shot for any chunk size");
    }

    #[test]
    fn hmac_matches_rfc_4231() {
        // Test case 1 and 2 (short key), 6 (key longer than the block size).
        assert_eq!(hex(&hmac_sha256(&[0x0b; 20], b"Hi There")), "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7");
        assert_eq!(hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        assert_eq!(
            hex(&hmac_sha256(&[0xaa; 131], b"Test Using Larger Than Block-Size Key - Hash Key First")),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
        assert_eq!(hmac_sha256_parts(b"Jefe", &[b"what do ya ", b"want for nothing?"]), hmac_sha256(b"Jefe", b"what do ya want for nothing?"));
    }

    #[test]
    fn ct_eq_compares_length_and_content() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
    }

    #[test]
    fn the_os_rng_gives_distinct_nonzero_values_and_hex_round_trips() {
        let (a, b) = (random_u64().unwrap(), random_u64().unwrap());
        assert!(a != 0 && b != 0 && a != b);
        let mut k = [0u8; 32];
        fill_random(&mut k).unwrap();
        assert_ne!(k, [0u8; 32]);
        assert_eq!(unhex(&hex(&k)).unwrap(), k);
        assert_eq!(unhex("abc"), None);
        assert_eq!(unhex("zz"), None);
    }
}
