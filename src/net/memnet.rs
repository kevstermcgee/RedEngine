//! An in-memory network with a virtual clock: a [`ServerTransport`] and any number of [`ClientTransport`]s joined by a wire that lives in one process, where
//! "time" is whatever the caller says it is.
//!
//! The game-level networking (`Server::pump`/`tick`, `NetClient::poll`, `Bot::pump`) already takes the time as an argument and sits behind the two transport traits,
//! so it behaves like a state machine: datagrams and a clock go in, datagrams and state changes come out. This module supplies the missing piece for testing it as one.
//! A test builds a [`MemNet`], hands its transports to `Server::with_transport` and `NetClient::with_transport`, and loops: advance the wire's clock, call
//! `pump`/`tick` with `epoch + net.now()`. No socket, no thread, no `sleep`; a four-second match takes milliseconds, and a run is a pure function of the seed, the
//! [`LinkModel`] and the calls (loss, delay and reordering come from a seeded generator, so a failing run can be replayed exactly and shrunk).
//!
//! It models the link, not the protocol stack: a datagram is delivered whole or lost, after a delay with jitter (so jitter reorders), and a peer can be cut off
//! ([`MemNet::partition`]) to look like a dead connection without the client cooperating. It does not replace the real-UDP and QUIC suites, which still prove the
//! sockets, the TLS handshake and the OS; it lets the far larger number of *logic* tests (lobby, rounds, timeouts, reconnects, loss) skip them.

use super::transport::{ClientTransport, Security, ServerTransport, TransportStatus};
use std::collections::{HashMap, HashSet};
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Largest datagram the in-memory link carries (the development UDP limit).
pub const MEM_MAX_DATAGRAM: usize = super::transport::UDP_MAX_DATAGRAM;

/// What the link does to a datagram.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LinkModel {
    /// Chance a datagram is lost, in parts per thousand (0 = lossless).
    pub loss_permille: u32,
    /// Fixed one-way delay.
    pub delay: Duration,
    /// Extra one-way delay, uniform in `0..=jitter`; datagrams whose delays cross arrive out of order.
    pub jitter: Duration,
}

#[derive(Debug)]
struct Packet {
    due: Duration,
    order: u64,
    from: SocketAddr,
    bytes: Vec<u8>,
}

#[derive(Debug)]
struct Wire {
    now: Duration,
    rng: u64,
    link: LinkModel,
    order: u64,
    server: SocketAddr,
    to_server: Vec<Packet>,
    to_client: HashMap<SocketAddr, Vec<Packet>>,
    cut: HashSet<SocketAddr>,
    sent: u64,
    dropped: u64,
}

impl Wire {
    fn rand(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Queues a datagram (to the server if `to_server`, else to `peer`) unless the link loses it or the peer is cut off.
    fn send(&mut self, to_server: bool, peer: SocketAddr, from: SocketAddr, bytes: &[u8]) {
        self.sent += 1;
        let lost = self.cut.contains(&peer) || (self.link.loss_permille > 0 && self.rand() % 1000 < self.link.loss_permille as u64);
        if lost || bytes.len() > MEM_MAX_DATAGRAM {
            self.dropped += 1;
            return;
        }
        let jitter = if self.link.jitter.is_zero() { Duration::ZERO } else { Duration::from_nanos(self.rand() % (self.link.jitter.as_nanos() as u64 + 1)) };
        self.order += 1;
        let packet = Packet { due: self.now + self.link.delay + jitter, order: self.order, from, bytes: bytes.to_vec() };
        if to_server {
            self.to_server.push(packet);
        } else {
            self.to_client.entry(peer).or_default().push(packet);
        }
    }
}

/// Removes and returns the earliest packet that has arrived by `now` (ties in send order), if any.
fn take_due(queue: &mut Vec<Packet>, now: Duration) -> Option<Packet> {
    let i = queue.iter().enumerate().filter(|(_, p)| p.due <= now).min_by_key(|(_, p)| (p.due, p.order)).map(|(i, _)| i)?;
    Some(queue.remove(i))
}

/// The shared wire and its clock. Clone it freely: every clone is the same network.
#[derive(Clone)]
pub struct MemNet(Arc<Mutex<Wire>>);

impl MemNet {
    /// A network whose server is at `server` (any address; nothing is bound) and whose random choices come from `seed`.
    pub fn new(seed: u64, server: SocketAddr) -> MemNet {
        MemNet(Arc::new(Mutex::new(Wire {
            now: Duration::ZERO,
            rng: seed ^ 0x9E37_79B9_7F4A_7C15 | 1,
            link: LinkModel::default(),
            order: 0,
            server,
            to_server: Vec::new(),
            to_client: HashMap::new(),
            cut: HashSet::new(),
            sent: 0,
            dropped: 0,
        })))
    }

    fn wire(&self) -> std::sync::MutexGuard<'_, Wire> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Sets what the link does to datagrams sent from now on.
    pub fn set_link(&self, link: LinkModel) {
        self.wire().link = link;
    }

    /// Moves the virtual clock forward.
    pub fn advance(&self, dt: Duration) {
        self.wire().now += dt;
    }

    /// The virtual time since the network was made. Drive the endpoints with `epoch + net.now()`.
    pub fn now(&self) -> Duration {
        self.wire().now
    }

    /// Cuts `peer` off (everything to and from it vanishes) or restores it: a dead connection that neither end announced.
    pub fn partition(&self, peer: SocketAddr, cut: bool) {
        let mut w = self.wire();
        if cut {
            w.cut.insert(peer);
        } else {
            w.cut.remove(&peer);
        }
    }

    /// `(sent, dropped)` datagram counts so far, both directions.
    pub fn counts(&self) -> (u64, u64) {
        let w = self.wire();
        (w.sent, w.dropped)
    }

    /// The server's end.
    pub fn server_transport(&self) -> MemServer {
        MemServer(self.clone())
    }

    /// A client's end: `addr` is the address the server will see it as (unique per client).
    pub fn client_transport(&self, addr: SocketAddr) -> MemClient {
        MemClient { net: self.clone(), addr }
    }
}

/// The server's end of a [`MemNet`].
pub struct MemServer(MemNet);

impl ServerTransport for MemServer {
    fn security(&self) -> Security {
        Security::DevUdp
    }
    fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(self.0.wire().server)
    }
    fn recv(&mut self, buf: &mut [u8]) -> Option<(SocketAddr, usize)> {
        let mut w = self.0.wire();
        let now = w.now;
        let p = take_due(&mut w.to_server, now)?;
        let n = p.bytes.len().min(buf.len());
        buf[..n].copy_from_slice(&p.bytes[..n]);
        Some((p.from, n))
    }
    fn send(&mut self, peer: SocketAddr, bytes: &[u8]) -> io::Result<usize> {
        let mut w = self.0.wire();
        let server = w.server;
        w.send(false, peer, server, bytes);
        Ok(bytes.len())
    }
    fn max_datagram(&self, _peer: SocketAddr) -> usize {
        MEM_MAX_DATAGRAM
    }
    fn channel_binding(&self, _peer: SocketAddr) -> Option<[u8; 32]> {
        None
    }
}

/// One client's end of a [`MemNet`].
pub struct MemClient {
    net: MemNet,
    addr: SocketAddr,
}

impl ClientTransport for MemClient {
    fn security(&self) -> Security {
        Security::DevUdp
    }
    fn send(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut w = self.net.wire();
        w.send(true, self.addr, self.addr, bytes);
        Ok(bytes.len())
    }
    fn recv(&mut self, buf: &mut [u8]) -> Option<usize> {
        let mut w = self.net.wire();
        let now = w.now;
        let p = take_due(w.to_client.get_mut(&self.addr)?, now)?;
        let n = p.bytes.len().min(buf.len());
        buf[..n].copy_from_slice(&p.bytes[..n]);
        Some(n)
    }
    fn max_datagram(&self) -> usize {
        MEM_MAX_DATAGRAM
    }
    fn channel_binding(&self) -> Option<[u8; 32]> {
        None
    }
    fn status(&self) -> TransportStatus {
        TransportStatus::Ready
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(port: u16) -> SocketAddr {
        SocketAddr::new("10.0.0.1".parse().unwrap(), port)
    }

    #[test]
    fn a_datagram_arrives_after_its_delay_on_the_virtual_clock_and_not_before() {
        let net = MemNet::new(1, addr(1));
        net.set_link(LinkModel { delay: Duration::from_millis(50), ..Default::default() });
        let (mut server, mut client) = (net.server_transport(), net.client_transport(addr(2)));
        let mut buf = [0u8; 64];
        client.send(b"hello").unwrap();
        assert_eq!(server.recv(&mut buf), None, "nothing has arrived at time zero");
        net.advance(Duration::from_millis(49));
        assert_eq!(server.recv(&mut buf), None);
        net.advance(Duration::from_millis(1));
        assert_eq!(server.recv(&mut buf), Some((addr(2), 5)));
        assert_eq!(&buf[..5], b"hello");
        assert_eq!(server.recv(&mut buf), None, "delivered once");
        server.send(addr(2), b"world").unwrap();
        assert_eq!(client.recv(&mut buf), None);
        net.advance(Duration::from_millis(50));
        assert_eq!(client.recv(&mut buf), Some(5));
    }

    #[test]
    fn loss_jitter_and_reordering_follow_the_seed_exactly() {
        let trace = |seed: u64| {
            let net = MemNet::new(seed, addr(1));
            net.set_link(LinkModel { loss_permille: 300, delay: Duration::from_millis(10), jitter: Duration::from_millis(40) });
            let (mut server, mut client) = (net.server_transport(), net.client_transport(addr(2)));
            for i in 0..200u8 {
                client.send(&[i]).unwrap();
                net.advance(Duration::from_millis(1));
            }
            net.advance(Duration::from_millis(200));
            let mut got = Vec::new();
            let mut buf = [0u8; 4];
            while let Some((_, n)) = server.recv(&mut buf) {
                got.push(buf[..n][0]);
            }
            (got, net.counts())
        };
        let (a, counts) = trace(9);
        assert_eq!(trace(9), (a.clone(), counts), "the same seed loses, delays and reorders the same datagrams");
        assert_ne!(trace(10).0, a, "another seed does something else");
        assert!(counts.1 > 30 && counts.1 < 100, "about 30% of 200 lost: {counts:?}");
        assert!(a.windows(2).any(|w| w[0] > w[1]), "jitter wider than the send spacing reorders datagrams");
        assert_eq!(a.len() as u64 + counts.1, 200, "every datagram was delivered or counted as lost");
    }

    #[test]
    fn a_partitioned_peer_hears_nothing_and_is_heard_by_nobody_until_it_is_restored() {
        let net = MemNet::new(1, addr(1));
        let (mut server, mut client) = (net.server_transport(), net.client_transport(addr(2)));
        let mut buf = [0u8; 8];
        net.partition(addr(2), true);
        client.send(b"a").unwrap();
        server.send(addr(2), b"b").unwrap();
        assert_eq!((server.recv(&mut buf), client.recv(&mut buf)), (None, None));
        net.partition(addr(2), false);
        client.send(b"c").unwrap();
        assert_eq!(server.recv(&mut buf), Some((addr(2), 1)));
        assert_eq!(net.counts(), (3, 2));
    }
}
