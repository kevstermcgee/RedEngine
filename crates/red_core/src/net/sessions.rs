//! Who is connected: the per-client session record, the parked (recently dropped) players a returning
//! client can resume, and where resume tokens come from. Plain data, no sockets — `server.rs` drives it.

use super::auth::SessionKey;
use super::limits::{TokenBucket, INPUT_BURST, INPUT_PACKETS_PER_SEC, LOBBY_BURST, LOBBY_PACKETS_PER_SEC};
use crate::sim::change::Generation;
use crate::sim::player::PlayerState;
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
    /// The player id: stable from the lobby through every round and across a reconnect.
    pub slot: usize,
    pub token: u64,
    /// Authenticates every datagram of this session on the development UDP transport (see `net::auth`). `None` on QUIC, whose
    /// connection already encrypts and authenticates every datagram.
    pub key: Option<SessionKey>,
    /// The nonce and cookie this session's key was derived from (a retransmitted Hello carries the same pair).
    pub client_nonce: u64,
    pub cookie: u64,
    pub name: String,
    /// `0` human, `1` rat: what the player asked for (used at the next spawn).
    pub character: u8,
    pub ready: bool,
    /// The team the player is on in a team match (`1` or `2`; `0` in a match without teams).
    pub team: u8,
    /// Whether the player has a body in the running world.
    pub in_round: bool,
    /// The newest round whose `Welcome` the client has applied.
    pub round_ack: u16,
    /// The client's own reported round-trip time, ms (cosmetic).
    pub rtt_ms: u16,
    pub last_heard: Instant,
    pub snapshot_seq: u32,
    /// Sequence of the next `Status`.
    pub status_seq: u16,
    /// Per moving prop (by entity slot): the generation of the pose the client has acknowledged.
    pub known: Vec<Generation>,
    pub sent: [SentSnap; 64],
    pub last_client_time_ms: u32,
    pub last_client_packet_at: Instant,
    /// Input-packet budget (see [`super::limits`]).
    pub input_bucket: TokenBucket,
    /// Lobby-packet budget.
    pub lobby_bucket: TokenBucket,
}

impl Session {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        addr: SocketAddr,
        slot: usize,
        token: u64,
        key: Option<SessionKey>,
        client_nonce: u64,
        cookie: u64,
        name: String,
        character: u8,
        now: Instant,
    ) -> Self {
        Session {
            addr,
            slot,
            token,
            key,
            client_nonce,
            cookie,
            name,
            character,
            ready: false,
            team: 0,
            in_round: false,
            round_ack: 0,
            rtt_ms: 0,
            last_heard: now,
            snapshot_seq: 0,
            status_seq: 0,
            known: Vec::new(),
            sent: std::array::from_fn(|_| SentSnap::default()),
            last_client_time_ms: 0,
            last_client_packet_at: now,
            input_bucket: TokenBucket::new(INPUT_PACKETS_PER_SEC, INPUT_BURST, now),
            lobby_bucket: TokenBucket::new(LOBBY_PACKETS_PER_SEC, LOBBY_BURST, now),
        }
    }

    /// Forgets which prop poses the client has confirmed (the world was rebuilt: every generation restarted).
    pub fn forget_world(&mut self) {
        self.known.clear();
        for s in self.sent.iter_mut() {
            s.seq = 0;
            s.props.clear();
        }
    }
}

/// A dropped player waiting for its owner to come back with the token.
pub(super) struct Parked {
    pub token: u64,
    pub slot: usize,
    /// The team the player was on (`0` in a match without teams).
    pub team: u8,
    /// Where the player stood (`None` if they had no body: they were in the lobby).
    pub state: Option<PlayerState>,
    /// The round `state` belongs to: resuming into a different round spawns fresh.
    pub round: u16,
    pub name: String,
    pub character: u8,
    pub expires: Instant,
}

/// Resume-token generator: every token is 64 bits from the operating system's CSPRNG (`crypto::random_u64`), so knowing one player's
/// token says nothing about another's. (Datagram protection is separate: the transport, ADR 0044, or `net::auth` on development UDP.)
pub(super) struct TokenSource;

impl TokenSource {
    pub fn new() -> Self {
        TokenSource
    }

    /// A fresh non-zero token (`0` means "no token" on the wire), or `None` if the OS has no randomness to give (the join is refused
    /// rather than handed a guessable token).
    pub fn next(&mut self) -> Option<u64> {
        crate::crypto::random_u64().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_distinct_nonzero_and_not_a_counter() {
        let mut a = TokenSource::new();
        let toks: Vec<u64> = (0..1000).map(|_| a.next().unwrap()).collect();
        let set: std::collections::HashSet<_> = toks.iter().collect();
        assert_eq!(set.len(), toks.len());
        assert!(toks.iter().all(|t| *t != 0));
        assert!(toks.windows(2).any(|w| w[1].abs_diff(w[0]) > 1 << 32), "consecutive tokens are not close together");
        let mut b = TokenSource::new();
        assert_ne!(a.next(), b.next(), "two processes do not share a sequence");
    }
}
