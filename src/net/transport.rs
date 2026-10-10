//! The transport boundary (ADR 0044): how Red's datagrams reach the other side, separated from what they say.
//!
//! The server and client speak Red's binary protocol (`protocol`) on top of one of two backends:
//!
//! - **QUIC** ([`super::quic`], production): QUIC DATAGRAM frames inside a TLS 1.3 connection (quinn + rustls). Every datagram is
//!   encrypted and authenticated by the connection; the server proves its identity with a deployment-specific certificate the client
//!   pins; address validation (Retry) and the handshake are the library's. Red's own per-datagram HMAC tags are not used on it.
//! - **Development UDP** ([`UdpServer`], [`UdpClient`]): plain UDP with Red's HMAC tags and join proofs (ADR 0028): authenticated, **not
//!   encrypted**. Chosen only explicitly (`red_server --dev-udp`, `re2 --dev-udp`, `ServerTransportConfig::DevUdp`), and refused on a
//!   non-loopback address unless the operator also says `--insecure-public-udp`. Tests and loopback tools use it.
//!
//! Nothing ever falls back from QUIC to UDP: a client configured for QUIC that cannot verify the server stops with
//! [`TransportStatus::Failed`].
//!
//! Peers are identified by the address their connection started from ([`SocketAddr`]); the QUIC backend keeps that key for the life of the
//! connection even if the path migrates, so the server's session table did not have to change.

use std::io::{self, ErrorKind};
use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

/// What protects the datagrams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    /// QUIC + TLS 1.3: confidentiality, integrity and a verified server identity from the transport.
    Quic,
    /// Plain UDP with Red's own HMAC tags: integrity after the handshake, no confidentiality, no server identity. Development only.
    DevUdp,
}

impl Security {
    /// True when the transport itself encrypts and authenticates every datagram (Red then adds no tags of its own).
    pub fn is_secure(self) -> bool {
        self == Security::Quic
    }

    /// A short name for logs.
    pub fn name(self) -> &'static str {
        match self {
            Security::Quic => "quic (TLS 1.3, encrypted)",
            Security::DevUdp => "dev-udp (authenticated, NOT encrypted)",
        }
    }
}

/// Counters a transport keeps about work it refused.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TransportStats {
    /// Connections accepted (QUIC) since start.
    pub connections_accepted: u64,
    /// Connection attempts refused because the connection limit was reached.
    pub connections_refused: u64,
    /// Handshakes that failed or timed out.
    pub handshakes_failed: u64,
    /// Incoming datagrams dropped because the application queue was full.
    pub queue_dropped: u64,
    /// Outgoing messages too large for a datagram, sent on a short unidirectional stream instead.
    pub sent_on_stream: u64,
    /// Outgoing messages dropped (no connection, or the stream budget was spent).
    pub send_dropped: u64,
}

/// A server's side of the transport: many peers, non-blocking.
pub trait ServerTransport: Send {
    /// Which backend this is.
    fn security(&self) -> Security;
    /// The bound address (useful with port 0).
    fn local_addr(&self) -> io::Result<SocketAddr>;
    /// The next waiting datagram, copied into `buf`: `(peer, length)`. `None` when nothing is waiting.
    fn recv(&mut self, buf: &mut [u8]) -> Option<(SocketAddr, usize)>;
    /// Waits up to `timeout` for a datagram to arrive, then returns without consuming it (the next [`Self::recv`] gets it). Lets the server
    /// loop sleep until its next tick and still react at once to a packet, instead of waking every 0.5 ms to look. The default is a
    /// short sleep, for backends with nothing to block on.
    fn wait(&mut self, timeout: Duration) {
        std::thread::sleep(timeout.min(Duration::from_micros(500)));
    }
    /// Sends one message to `peer`. A message larger than [`Self::max_datagram`] is either carried reliably (QUIC) or refused.
    fn send(&mut self, peer: SocketAddr, bytes: &[u8]) -> io::Result<usize>;
    /// The largest message `peer` can be sent in one datagram right now.
    fn max_datagram(&self, peer: SocketAddr) -> usize;
    /// Keying material unique to this peer's secure connection (TLS exporter), to bind an application-level proof to it.
    fn channel_binding(&self, peer: SocketAddr) -> Option<[u8; 32]>;
    /// Forgets `peer` (closes its connection, if the backend has one).
    fn close_peer(&mut self, _peer: SocketAddr, _reason: &str) {}
    /// Closes everything and waits briefly for goodbyes to leave.
    fn shutdown(&mut self) {}
    /// Peers whose connection closed since the last call (QUIC); the server ends their sessions at once. Always empty for UDP.
    fn take_closed(&mut self) -> Vec<SocketAddr> {
        Vec::new()
    }
    /// Refusal counters.
    fn stats(&self) -> TransportStats {
        TransportStats::default()
    }
}

/// Whether a client transport can carry traffic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportStatus {
    /// Establishing (QUIC handshake in flight).
    Connecting,
    /// Ready to send and receive.
    Ready,
    /// Gave up, with the reason (a wrong server identity, a refused connection). Never followed by a downgrade.
    Failed(String),
}

/// A client's side of the transport: one server, non-blocking.
pub trait ClientTransport: Send {
    /// Which backend this is.
    fn security(&self) -> Security;
    /// Sends one message to the server (dropped while not [`TransportStatus::Ready`]).
    fn send(&mut self, bytes: &[u8]) -> io::Result<usize>;
    /// The next waiting message from the server, copied into `buf`.
    fn recv(&mut self, buf: &mut [u8]) -> Option<usize>;
    /// The largest message the server can be sent in one datagram.
    fn max_datagram(&self) -> usize;
    /// See [`ServerTransport::channel_binding`].
    fn channel_binding(&self) -> Option<[u8; 32]>;
    /// Where the transport is.
    fn status(&self) -> TransportStatus;
    /// Starts a fresh connection (after the server went silent). No-op for UDP.
    fn reconnect(&mut self) {}
    /// Closes the connection.
    fn close(&mut self) {}
}

/// Largest datagram the development UDP backend sends or accepts (under a typical 1500-byte MTU).
pub const UDP_MAX_DATAGRAM: usize = 1400;

/// The development server transport: one non-blocking UDP socket.
pub struct UdpServer {
    socket: UdpSocket,
}

impl UdpServer {
    /// Binds `addr` (non-blocking).
    pub fn bind(addr: SocketAddr) -> io::Result<UdpServer> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(true)?;
        Ok(UdpServer { socket })
    }
}

impl ServerTransport for UdpServer {
    fn security(&self) -> Security {
        Security::DevUdp
    }
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }
    fn recv(&mut self, buf: &mut [u8]) -> Option<(SocketAddr, usize)> {
        loop {
            match self.socket.recv_from(buf) {
                Ok((n, from)) => return Some((from, n)),
                // Windows reports a previous send to a closed port as an error on the *next* receive.
                Err(e) if e.kind() == ErrorKind::ConnectionReset => continue,
                Err(_) => return None,
            }
        }
    }
    fn wait(&mut self, timeout: Duration) {
        // Block (with a deadline) until a datagram is queued, peeking so it stays queued; then back to non-blocking for `recv`.
        // A zero read timeout is an error on every platform, so a zero wait is simply no wait.
        if timeout.is_zero() || self.socket.set_nonblocking(false).is_err() {
            return;
        }
        let _ = self.socket.set_read_timeout(Some(timeout));
        let mut byte = [0u8; 1];
        let _ = self.socket.peek_from(&mut byte);
        let _ = self.socket.set_nonblocking(true);
    }
    fn send(&mut self, peer: SocketAddr, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > UDP_MAX_DATAGRAM {
            return Err(io::Error::new(ErrorKind::InvalidInput, "datagram larger than the UDP budget"));
        }
        self.socket.send_to(bytes, peer)
    }
    fn max_datagram(&self, _peer: SocketAddr) -> usize {
        UDP_MAX_DATAGRAM
    }
    fn channel_binding(&self, _peer: SocketAddr) -> Option<[u8; 32]> {
        None
    }
}

/// The development client transport: one non-blocking UDP socket talking to one server address.
pub struct UdpClient {
    socket: UdpSocket,
    server: SocketAddr,
}

impl UdpClient {
    /// Binds a local socket suited to `server` (loopback for a loopback server: no firewall prompt).
    pub fn connect(server: SocketAddr) -> io::Result<UdpClient> {
        let socket = UdpSocket::bind(super::client::local_bind_for(server))?;
        socket.set_nonblocking(true)?;
        Ok(UdpClient { socket, server })
    }
}

impl ClientTransport for UdpClient {
    fn security(&self) -> Security {
        Security::DevUdp
    }
    fn send(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.socket.send_to(bytes, self.server)
    }
    fn recv(&mut self, buf: &mut [u8]) -> Option<usize> {
        loop {
            match self.socket.recv_from(buf) {
                Ok((n, from)) if from == self.server => return Some(n),
                Ok(_) => continue, // not our server: ignore
                Err(e) if e.kind() == ErrorKind::ConnectionReset => continue,
                Err(_) => return None,
            }
        }
    }
    fn max_datagram(&self) -> usize {
        UDP_MAX_DATAGRAM
    }
    fn channel_binding(&self) -> Option<[u8; 32]> {
        None
    }
    fn status(&self) -> TransportStatus {
        TransportStatus::Ready
    }
}

/// Whether the development UDP transport may listen on `bind`: always on loopback; elsewhere only when the operator explicitly accepted
/// unencrypted traffic. `Err` explains the refusal (fail closed).
pub fn dev_udp_allowed(bind: SocketAddr, insecure_public: bool) -> Result<(), String> {
    if bind.ip().is_loopback() || insecure_public {
        Ok(())
    } else {
        Err(format!(
            "refusing development UDP on {bind}: it is not encrypted and has no server identity. Use QUIC (the default: --tls-cert/--tls-key, \
             `red_engine2 net-identity`) or, for a trusted LAN only, add --insecure-public-udp"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_udp_is_loopback_only_unless_explicitly_accepted() {
        assert!(dev_udp_allowed("127.0.0.1:1".parse().unwrap(), false).is_ok());
        assert!(dev_udp_allowed("[::1]:1".parse().unwrap(), false).is_ok());
        let err = dev_udp_allowed("0.0.0.0:27015".parse().unwrap(), false).unwrap_err();
        assert!(err.contains("not encrypted"), "{err}");
        assert!(dev_udp_allowed("0.0.0.0:27015".parse().unwrap(), true).is_ok());
    }

    #[test]
    fn udp_round_trip_and_the_budget_is_enforced() {
        let mut server = UdpServer::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let mut client = UdpClient::connect(server.local_addr().unwrap()).unwrap();
        client.send(b"hello").unwrap();
        let mut buf = [0u8; 2048];
        let t0 = std::time::Instant::now();
        let (peer, n) = loop {
            if let Some(x) = server.recv(&mut buf) {
                break x;
            }
            assert!(t0.elapsed().as_secs() < 2);
        };
        assert_eq!(&buf[..n], b"hello");
        assert!(server.send(peer, &[0u8; UDP_MAX_DATAGRAM + 1]).is_err());
        assert_eq!(client.status(), TransportStatus::Ready);
        assert!(!client.security().is_secure());
    }
}
