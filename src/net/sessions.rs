//! Who is connected: the per-client session record, the parked (recently dropped) players a returning
//! client can resume, and where resume tokens come from. Plain data, no sockets — `server.rs` drives it.

use super::limits::{TokenBucket, INPUT_BURST, INPUT_PACKETS_PER_SEC};
use crate::sim::change::Generation;
use crate::sim::player::PlayerState;
use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::net::SocketAddr;
use std::time::Instant;

/// What the server remembers about one snapshot it sent: which props' poses it carried, so an acknowledgement can
/// confirm them. (The vector keeps its allocation between reuses of the slot.)
#[derive(Clone, Default)]
pub(super) struct SentSnap {
    pub seq: u32,
    /// `(entity slot, generation of the pose sent)`.
    pub props: Vec<(usize, Generation)>,
}

/// One connected client.
pub(super) struct Session {
    pub addr: SocketAddr,
    pub slot: usize,
    pub token: u64,
    pub last_heard: Instant,
    pub snapshot_seq: u32,
    /// Per moving prop (by entity slot): the generation of the pose the client has acknowledged.
    pub known: Vec<Generation>,
    pub sent: [SentSnap; 64],
    pub last_client_time_ms: u32,
    pub last_client_packet_at: Instant,
    /// Input-packet budget (see [`super::limits`]).
    pub input_bucket: TokenBucket,
}

impl Session {
    pub fn new(addr: SocketAddr, slot: usize, token: u64, now: Instant) -> Self {
        Session {
            addr,
            slot,
            token,
            last_heard: now,
            snapshot_seq: 0,
            known: Vec::new(),
            sent: std::array::from_fn(|_| SentSnap::default()),
            last_client_time_ms: 0,
            last_client_packet_at: now,
            input_bucket: TokenBucket::new(INPUT_PACKETS_PER_SEC, INPUT_BURST, now),
        }
    }
}

/// A dropped player waiting for its owner to come back with the token.
pub(super) struct Parked {
    pub token: u64,
    pub state: PlayerState,
    pub expires: Instant,
}

/// Resume-token generator. Each process gets random SipHash keys from the OS (`RandomState`), so a token is
/// not guessable from the join order or the clock — knowing one player's token says nothing about another's.
/// (Real authentication/encryption is a transport layer that can wrap this later; ADR 0016.)
pub(super) struct TokenSource {
    keys: RandomState,
    counter: u64,
}

impl TokenSource {
    pub fn new() -> Self {
        TokenSource { keys: RandomState::new(), counter: 0 }
    }

    /// A fresh non-zero token (`0` means "no token" on the wire).
    pub fn next(&mut self) -> u64 {
        self.counter = self.counter.wrapping_add(1);
        self.keys.hash_one(self.counter).max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_distinct_nonzero_and_not_a_counter() {
        let mut a = TokenSource::new();
        let toks: Vec<u64> = (0..1000).map(|_| a.next()).collect();
        let set: std::collections::HashSet<_> = toks.iter().collect();
        assert_eq!(set.len(), toks.len());
        assert!(toks.iter().all(|t| *t != 0));
        assert!(toks.windows(2).any(|w| w[1].abs_diff(w[0]) > 1 << 32), "consecutive tokens are not close together");
        let mut b = TokenSource::new();
        assert_ne!(a.next(), b.next(), "two processes do not share a sequence");
    }
}
