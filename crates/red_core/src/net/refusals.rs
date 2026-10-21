//! Why the server turned a join away: counted, and said once a minute (ADR 2026-10-20-refused-joins-are-counted-and-said).
//!
//! Until this module the only trace of a refused `Hello` was the reply the client got. The server's journal read "0 joins, 0 bad packets" while a
//! whole family could not get in (a v15 server silently refusing every v16 and v17 client, 2026-10-20). Now every refusal is counted per reason,
//! shown in the periodic `stats:` line and the stop line, and written to the log as **one line per reason per minute** naming the reason, the
//! protocol version the client speaks, its map hash when the server can read it, and the peer address masked to its first two octets. A join key
//! or any proof is never logged. Nothing here touches the wire: the replies are the ones the protocol already has.
//!
//! **`NeedsKey` is inferred.** The server never sends that reply: a client that is challenged for a key it does not have gives up on its own
//! ([`super::protocol::RejectReason::NeedsKey`] is "decided by the client"). What the server can see is a `Challenge` on a keyed server that no
//! second `Hello` ever answered, so [`Refusals`] remembers the few most recent unanswered challenges and, after [`NEEDS_KEY_GRACE`], counts each as a
//! `NeedsKey`: "a client came, was asked for a key and said nothing more". A handshake is matched by its nonce alone (a NAT may change the client's port
//! between its two Hellos), an answered nonce is remembered for a while (a late copy of the first Hello must not open a new entry), and one source address
//! holds at most [`MAX_PENDING_PER_ADDRESS`] entries: anyone who completes a QUIC handshake can send first Hellos and never answer.
//!
//! **A counter is join attempts, not Hellos.** The client resends its Hello every 250 ms until a reply arrives; a refused Hello the same handshake sends again
//! within [`REPEAT_WINDOW`] is replied to but not counted again ([`Refusals::repeat`]).

use super::protocol::{RejectReason, PROTOCOL_VERSION};
use super::server::ServerStats;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

/// A reason is written to the log at most this often; the ones in between are counted and mentioned in the next line.
pub const LOG_EVERY: Duration = Duration::from_secs(60);
/// A challenge on a keyed server that no second `Hello` answered within this long counts as a `NeedsKey` (a real client answers within a round trip).
pub const NEEDS_KEY_GRACE: Duration = Duration::from_secs(8);
/// Unanswered challenges remembered at once (a join flood is cut off earlier, by the server-wide `Hello` budget).
const MAX_PENDING: usize = 32;
/// Unanswered challenges remembered for one source address: one machine cannot fill the list (and push `k` up by the whole list every grace period).
pub const MAX_PENDING_PER_ADDRESS: usize = 2;
/// A `Hello` the same handshake (address and nonce) sends again within this long of the last refused copy is the same attempt: replied to, not counted
/// again. The real client resends every 250 ms until a reply arrives.
pub const REPEAT_WINDOW: Duration = Duration::from_secs(5);
/// How long an answered handshake's nonce is remembered, so a late copy of its first Hello is not taken for a new client that never answers.
const ANSWERED_MEMORY: Duration = Duration::from_secs(30);
/// Answered nonces remembered at once (the oldest goes first).
const MAX_ANSWERED: usize = 64;
/// Recently refused handshakes remembered at once for [`REPEAT_WINDOW`] (the oldest goes first).
const MAX_RECENT: usize = 64;

/// Why a join did not happen, as the server counts it. The first four are the ones the README and the clients name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The client speaks another protocol version.
    Version,
    /// The client's map is not the one this server plays.
    WrongMap,
    /// A client was challenged for a join key and sent none (inferred, see the module docs).
    NeedsKey,
    /// The join key proof did not match.
    BadKey,
    /// The match is full (or a slot could not be given).
    Full,
    /// Any other reason a `Reject` carries (`ServerIsOpen` and `ServerIdentity` are decided by clients, so the server does not produce them today).
    Other,
}

impl Refusal {
    /// Every reason, in the order of the `stats:` line.
    pub const ALL: [Refusal; 6] = [Refusal::Version, Refusal::WrongMap, Refusal::NeedsKey, Refusal::BadKey, Refusal::Full, Refusal::Other];

    /// The refusal a `Reject` reply stands for. Written as a total match, so a new [`RejectReason`] cannot be added without deciding what it counts as.
    pub fn of(reason: RejectReason) -> Refusal {
        match reason {
            RejectReason::Version => Refusal::Version,
            RejectReason::WrongMap => Refusal::WrongMap,
            RejectReason::NeedsKey => Refusal::NeedsKey,
            RejectReason::BadKey => Refusal::BadKey,
            RejectReason::Full => Refusal::Full,
            RejectReason::ServerIsOpen | RejectReason::ServerIdentity => Refusal::Other,
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|r| *r == self).unwrap_or(0)
    }

    /// The word the log line puts in brackets.
    fn word(self) -> &'static str {
        match self {
            Refusal::Version => "version",
            Refusal::WrongMap => "map",
            Refusal::NeedsKey => "needs key",
            Refusal::BadKey => "key",
            Refusal::Full => "full",
            Refusal::Other => "other",
        }
    }
}

/// What a `Hello` says about its sender, as far as the server can read it: never the key, the proof or the name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HelloFacts {
    /// The protocol version the client says it speaks.
    pub version: u16,
    /// Its map hash; `None` when the Hello is of another version (its layout is not known, so the hash cannot be read).
    pub map_hash: Option<u32>,
}

impl HelloFacts {
    /// Reads a decoded `Hello`. Another version's Hello decodes to its version alone (see `ClientMsg::decode`), so its map hash is unknown.
    pub fn of(h: &super::protocol::Hello) -> HelloFacts {
        HelloFacts { version: h.version, map_hash: (h.version == PROTOCOL_VERSION).then_some(h.map_hash) }
    }
}

impl ServerStats {
    /// Counts one refusal under its reason (a wrong key is `bad_keys`, which existed before the others).
    pub fn count_refusal(&mut self, which: Refusal) {
        match which {
            Refusal::Version => self.rejected_version += 1,
            Refusal::WrongMap => self.rejected_wrong_map += 1,
            Refusal::NeedsKey => self.rejected_needs_key += 1,
            Refusal::BadKey => self.bad_keys += 1,
            Refusal::Full => self.rejected_full += 1,
            Refusal::Other => self.rejected_other += 1,
        }
    }

    /// Every refusal so far, whatever the reason.
    pub fn rejected_total(&self) -> u64 {
        self.rejected_version + self.rejected_wrong_map + self.rejected_needs_key + self.bad_keys + self.rejected_full + self.rejected_other
    }

    /// The counters as the `stats:` line and the stop line print them: `rejected v1 m0 k0 b0 f0 o0` (version, map, needs key, bad key, full, other).
    pub fn rejected_summary(&self) -> String {
        format!(
            "rejected v{} m{} k{} b{} f{} o{}",
            self.rejected_version, self.rejected_wrong_map, self.rejected_needs_key, self.bad_keys, self.rejected_full, self.rejected_other
        )
    }
}

/// A challenge sent to a client that has not answered it yet.
struct Pending {
    peer: SocketAddr,
    nonce: u64,
    facts: HelloFacts,
    at: Instant,
}

/// The rate limit of the refusal log, the unanswered challenges that become `NeedsKey`, and the handshakes refused a moment ago.
#[derive(Default)]
pub struct Refusals {
    last_said: [Option<Instant>; 6],
    held_back: [u32; 6],
    pending: Vec<Pending>,
    /// Nonces of handshakes whose second Hello arrived, and when.
    answered: Vec<(u64, Instant)>,
    /// `(peer, nonce, last refused copy)` of handshakes refused within [`REPEAT_WINDOW`].
    recent: Vec<(SocketAddr, u64, Instant)>,
}

impl Refusals {
    /// Nothing said, nothing pending.
    pub fn new() -> Refusals {
        Refusals::default()
    }

    /// The log line for one refusal, or `None` when this reason was already written within [`LOG_EVERY`] (it is counted for the next line).
    pub fn line(&mut self, which: Refusal, peer: SocketAddr, facts: HelloFacts, server_map: u32, now: Instant) -> Option<String> {
        let i = which.index();
        if self.last_said[i].is_some_and(|last| now.saturating_duration_since(last) < LOG_EVERY) {
            self.held_back[i] = self.held_back[i].saturating_add(1);
            return None;
        }
        self.last_said[i] = Some(now);
        let more = std::mem::take(&mut self.held_back[i]);
        let map = match facts.map_hash {
            Some(h) => format!("map {h:08x}"),
            None => "map unknown (another version's Hello cannot be read)".to_string(),
        };
        let why = match which {
            Refusal::Version => {
                let who = if facts.version < PROTOCOL_VERSION { "the client is the old side and must update" } else { "this server is the old side and must be redeployed" };
                format!("the client speaks protocol v{} and this server v{PROTOCOL_VERSION}, so {who}; {map}", facts.version)
            }
            Refusal::WrongMap => format!(
                "the client's map is {:08x} and this server plays {server_map:08x}; client protocol v{}",
                facts.map_hash.unwrap_or(0),
                facts.version
            ),
            Refusal::BadKey => format!("the join-key proof did not match (the key is never logged); client protocol v{}, {map}", facts.version),
            Refusal::NeedsKey => format!(
                "a client was challenged for a join key and sent none within {} s, so it gave up (usually a code without the key part); client protocol v{}, {map}",
                NEEDS_KEY_GRACE.as_secs(),
                facts.version
            ),
            Refusal::Full => format!("the match is full; client protocol v{}, {map}", facts.version),
            Refusal::Other => format!("another reason; client protocol v{}, {map}", facts.version),
        };
        let more = if more > 0 { format!("; {more} more like it since the last line") } else { String::new() };
        Some(format!("join refused ({}): {why}; peer {}{more}", which.word(), mask(peer.ip())))
    }

    /// A challenge went out on a keyed server: remember it until the second `Hello` arrives. One entry per handshake however often its first Hello repeats,
    /// none for a handshake that was already answered (a late or replayed copy of its first Hello), and at most [`MAX_PENDING_PER_ADDRESS`] per source address.
    pub fn challenged(&mut self, peer: SocketAddr, nonce: u64, facts: HelloFacts, now: Instant) {
        self.answered.retain(|(_, at)| now.saturating_duration_since(*at) < ANSWERED_MEMORY);
        let known = self.pending.iter().any(|p| p.nonce == nonce) || self.answered.iter().any(|(n, _)| *n == nonce);
        let from_peer = self.pending.iter().filter(|p| p.peer.ip() == peer.ip()).count();
        if !known && from_peer < MAX_PENDING_PER_ADDRESS && self.pending.len() < MAX_PENDING {
            self.pending.push(Pending { peer, nonce, facts, at: now });
        }
    }

    /// The second `Hello` of a handshake arrived: that challenge was answered (matched by the nonce alone: the address may have changed since the first).
    pub fn answered(&mut self, nonce: u64, now: Instant) {
        self.pending.retain(|p| p.nonce != nonce);
        if let Some(seen) = self.answered.iter_mut().find(|(n, _)| *n == nonce) {
            seen.1 = now;
            return;
        }
        if self.answered.len() >= MAX_ANSWERED {
            self.answered.remove(0);
        }
        self.answered.push((nonce, now));
    }

    /// Whether a refusal is a copy of one this handshake (address and nonce) was given within [`REPEAT_WINDOW`]: the client resends until a reply arrives, so
    /// that copy is the same attempt and is not counted again. Not a repeat: it is remembered, and the next copy within the window is one.
    pub fn repeat(&mut self, peer: SocketAddr, nonce: u64, now: Instant) -> bool {
        self.recent.retain(|(_, _, at)| now.saturating_duration_since(*at) < REPEAT_WINDOW);
        if let Some(seen) = self.recent.iter_mut().find(|(p, n, _)| *p == peer && *n == nonce) {
            seen.2 = now;
            return true;
        }
        if self.recent.len() >= MAX_RECENT {
            self.recent.remove(0);
        }
        self.recent.push((peer, nonce, now));
        false
    }

    /// The challenges nobody answered within [`NEEDS_KEY_GRACE`], removed from the list: each is a `NeedsKey`.
    pub fn unanswered(&mut self, now: Instant) -> Vec<(SocketAddr, HelloFacts)> {
        let mut gone = Vec::new();
        self.pending.retain(|p| {
            let stale = now.saturating_duration_since(p.at) >= NEEDS_KEY_GRACE;
            if stale {
                gone.push((p.peer, p.facts));
            }
            !stale
        });
        gone
    }

    /// How many challenges wait for an answer (tests).
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

/// An address for a log line that is kept and shared: IPv4 to its first two octets (`73.157.x.x`), IPv6 to its first two groups (`2601:abcd:x`).
pub fn mask(ip: IpAddr) -> String {
    match ip.to_canonical() {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            format!("{}.{}.x.x", o[0], o[1])
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            format!("{:x}:{:x}:x", s[0], s[1])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(last: u8) -> SocketAddr {
        SocketAddr::new(IpAddr::from([73, 157, 20, last]), 5000)
    }
    const HERE: HelloFacts = HelloFacts { version: PROTOCOL_VERSION, map_hash: Some(0xdead_beef) };

    #[test]
    fn an_address_is_masked_to_its_first_two_octets_or_groups() {
        assert_eq!(mask("73.157.20.99".parse().unwrap()), "73.157.x.x");
        assert_eq!(mask("::ffff:73.157.20.99".parse().unwrap()), "73.157.x.x", "an IPv4 peer seen through a dual-stack socket");
        assert_eq!(mask("2601:abcd:1234:5678::1".parse().unwrap()), "2601:abcd:x");
    }

    #[test]
    fn a_reason_is_said_once_a_minute_and_the_rest_are_counted_into_the_next_line() {
        let (mut r, t0) = (Refusals::new(), Instant::now());
        let first = r.line(Refusal::BadKey, peer(1), HERE, 7, t0).expect("the first of a reason is always said");
        assert!(first.starts_with("join refused (key): "), "{first}");
        assert!(r.line(Refusal::BadKey, peer(2), HERE, 7, t0 + Duration::from_secs(1)).is_none());
        assert!(r.line(Refusal::BadKey, peer(3), HERE, 7, t0 + Duration::from_secs(59)).is_none());
        assert!(r.line(Refusal::Full, peer(3), HERE, 7, t0 + Duration::from_secs(2)).is_some(), "another reason has its own minute");
        let again = r.line(Refusal::BadKey, peer(4), HERE, 7, t0 + LOG_EVERY).expect("a minute later");
        assert!(again.contains("2 more like it since the last line"), "{again}");
        let third = r.line(Refusal::BadKey, peer(5), HERE, 7, t0 + LOG_EVERY * 2).expect("and again");
        assert!(!third.contains("more like it"), "the held-back count starts over: {third}");
    }

    #[test]
    fn each_line_names_the_reason_the_protocol_the_map_and_a_masked_peer_and_nothing_secret() {
        let (mut r, t0) = (Refusals::new(), Instant::now());
        let old = r.line(Refusal::Version, peer(1), HelloFacts { version: PROTOCOL_VERSION - 1, map_hash: None }, 7, t0).unwrap();
        assert!(old.contains(&format!("client speaks protocol v{} and this server v{PROTOCOL_VERSION}", PROTOCOL_VERSION - 1)), "{old}");
        assert!(old.contains("the client is the old side"), "{old}");
        assert!(old.contains("map unknown"), "a v-1 Hello cannot be read for its map: {old}");
        let new = r.line(Refusal::Version, peer(1), HelloFacts { version: PROTOCOL_VERSION + 1, map_hash: None }, 7, t0 + LOG_EVERY).unwrap();
        assert!(new.contains("this server is the old side"), "{new}");
        let map = r.line(Refusal::WrongMap, peer(9), HERE, 0x1234_abcd, t0).unwrap();
        assert!(map.contains("deadbeef") && map.contains("1234abcd") && map.contains("peer 73.157.x.x"), "{map}");
        assert!(!map.contains("73.157.20"), "the address is masked: {map}");
        for which in Refusal::ALL {
            let line = Refusals::new().line(which, peer(1), HERE, 7, t0).unwrap();
            assert!(line.starts_with("join refused ("), "{line}");
            assert!(line.contains(&format!("v{PROTOCOL_VERSION}")), "{line}");
        }
    }

    #[test]
    fn a_challenge_nobody_answers_becomes_a_needs_key_and_an_answered_one_does_not() {
        let (mut r, t0) = (Refusals::new(), Instant::now());
        r.challenged(peer(1), 11, HERE, t0);
        r.challenged(peer(1), 11, HERE, t0 + Duration::from_millis(250)); // the first Hello repeats until the Challenge arrives
        r.challenged(peer(2), 22, HERE, t0);
        assert_eq!(r.pending_count(), 2, "one entry per handshake");
        r.answered(22, t0);
        assert!(r.unanswered(t0 + NEEDS_KEY_GRACE - Duration::from_millis(1)).is_empty(), "still within the grace");
        let gone = r.unanswered(t0 + NEEDS_KEY_GRACE);
        assert_eq!(gone, vec![(peer(1), HERE)], "only the handshake nobody answered");
        assert!(r.unanswered(t0 + NEEDS_KEY_GRACE * 2).is_empty(), "each is counted once");
    }

    #[test]
    fn the_unanswered_list_is_bounded() {
        let (mut r, t0) = (Refusals::new(), Instant::now());
        for n in 0..1000u64 {
            r.challenged(peer((n % 200) as u8), n, HERE, t0);
        }
        assert_eq!(r.pending_count(), MAX_PENDING);
    }

    #[test]
    fn one_address_cannot_fill_the_unanswered_list() {
        let (mut r, t0) = (Refusals::new(), Instant::now());
        // One machine, many ports and nonces.
        for n in 0..40u64 {
            r.challenged(SocketAddr::new(IpAddr::from([73, 157, 20, 1]), 5000 + n as u16), n, HERE, t0);
        }
        assert_eq!(r.pending_count(), MAX_PENDING_PER_ADDRESS);
        r.challenged(peer(2), 1000, HERE, t0);
        assert_eq!(r.pending_count(), MAX_PENDING_PER_ADDRESS + 1, "another address still gets its entry");
    }

    #[test]
    fn an_answered_handshake_is_matched_by_its_nonce_and_a_late_first_hello_opens_nothing() {
        let (mut r, t0) = (Refusals::new(), Instant::now());
        r.challenged(peer(1), 11, HERE, t0);
        r.answered(11, t0 + Duration::from_millis(100)); // the second Hello came from another port of the same client (a NAT rebinding)
        assert_eq!(r.pending_count(), 0);
        r.challenged(peer(1), 11, HERE, t0 + Duration::from_millis(300)); // a late copy of the first Hello
        assert_eq!(r.pending_count(), 0, "an answered handshake is not opened again");
        assert!(r.unanswered(t0 + NEEDS_KEY_GRACE * 2).is_empty());
        r.challenged(peer(1), 11, HERE, t0 + ANSWERED_MEMORY + Duration::from_secs(1));
        assert_eq!(r.pending_count(), 1, "the memory is bounded in time");
        for n in 0..(MAX_ANSWERED as u64 * 2) {
            r.answered(1000 + n, t0);
        }
        assert_eq!(r.answered.len(), MAX_ANSWERED, "and in size");
    }

    #[test]
    fn a_refusal_the_same_handshake_repeats_within_the_window_is_one_attempt() {
        let (mut r, t0) = (Refusals::new(), Instant::now());
        assert!(!r.repeat(peer(1), 5, t0), "the first refusal counts");
        assert!(r.repeat(peer(1), 5, t0 + Duration::from_millis(250)), "the resent Hello is the same attempt");
        assert!(!r.repeat(peer(1), 6, t0 + Duration::from_millis(250)), "another handshake");
        assert!(!r.repeat(peer(2), 5, t0 + Duration::from_millis(250)), "another address");
        assert!(r.repeat(peer(1), 5, t0 + Duration::from_millis(250) + REPEAT_WINDOW - Duration::from_millis(1)), "measured from the last copy");
        assert!(!r.repeat(peer(1), 5, t0 + Duration::from_secs(1) + REPEAT_WINDOW * 2), "a try after the window is a new attempt");
        for n in 0..(MAX_RECENT as u64 * 2) {
            r.repeat(peer(3), 100 + n, t0 + REPEAT_WINDOW * 3);
        }
        assert_eq!(r.recent.len(), MAX_RECENT, "bounded");
    }

    #[test]
    fn every_reject_reply_maps_to_a_counter_and_the_summary_keeps_its_order() {
        let mut s = ServerStats::default();
        for reason in
            [RejectReason::Version, RejectReason::WrongMap, RejectReason::BadKey, RejectReason::Full, RejectReason::NeedsKey, RejectReason::ServerIsOpen]
        {
            s.count_refusal(Refusal::of(reason));
        }
        assert_eq!(s.rejected_summary(), "rejected v1 m1 k1 b1 f1 o1");
        assert_eq!(s.rejected_total(), 6);
        assert_eq!(s.bad_keys, 1, "a wrong key is still the counter that existed before");
    }
}
