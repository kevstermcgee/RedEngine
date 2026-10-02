//! The relay's actual socket I/O: one well-known public socket hosts and joining clients both talk to, and one
//! small ephemeral socket per accepted pairing used only to talk to that pairing's host (so the host can still
//! tell multiple joiners apart by address, exactly as it does today — nothing in `net::server`/`net::quic` has
//! to change). The protocol and bookkeeping this drives (`RelayTable`, `RelayMessage`) are pure and tested
//! without any sockets in `net::relay`; this is the thin, harder-to-unit-test shell around them, kept separate
//! the same way `net::server::Server` is separate from `red_server`'s own CLI/signal-handling shell.
//!
//! **A client's `Resolve` and its actual game traffic may arrive from different local ports.** A game client
//! does a tiny raw-UDP round trip to resolve a code *before* handing off to `net::quic`'s own client, which owns
//! and binds its own socket — forcing it to reuse one exact port would mean reaching into QUIC endpoint setup
//! for a rare, small benefit. Instead a resolved code is "pending" for its client's *address* (IP only, not
//! port) for a short window; the first real packet seen from that IP locks the pairing to the exact `(ip, port)`
//! it arrived from, exactly as if that had been the port that resolved. One IP resolving two codes in quick
//! succession is not a supported shape (a second `Resolve` from the same IP simply replaces the first's pending
//! slot) — fine for what this is: one person joining one game.

use super::relay::{RelayMessage, RelayTable};
use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a paired forwarding session may go without a reply from the host before it is torn down.
pub const DEFAULT_PAIR_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
/// How long a resolved code stays "pending" for its client's IP, waiting for the first real packet to lock the
/// pairing to its actual port.
pub const DEFAULT_PENDING_TIMEOUT: Duration = Duration::from_secs(15);
/// How often [`RelayServer::run`] checks for stale registrations/pending resolutions and its stop flag.
pub const DEFAULT_HOUSEKEEPING_TICK: Duration = Duration::from_secs(1);

/// [`RelayServer::bind`]'s settings.
#[derive(Debug, Clone, Copy)]
pub struct RelayServerOptions {
    /// Where the public socket binds. `0.0.0.0:0` for "any address, an OS-picked port" (tests; a real deployment
    /// names a fixed port so `--relay HOST:PORT` has something stable to point at).
    pub bind: SocketAddr,
    pub pair_idle_timeout: Duration,
    pub pending_timeout: Duration,
    pub housekeeping_tick: Duration,
}

impl Default for RelayServerOptions {
    fn default() -> Self {
        RelayServerOptions {
            bind: SocketAddr::from(([0, 0, 0, 0], 0)),
            pair_idle_timeout: DEFAULT_PAIR_IDLE_TIMEOUT,
            pending_timeout: DEFAULT_PENDING_TIMEOUT,
            housekeeping_tick: DEFAULT_HOUSEKEEPING_TICK,
        }
    }
}

#[derive(Clone)]
struct Pairing {
    to_host: Arc<UdpSocket>,
    host: SocketAddr,
}

#[derive(Clone, Copy)]
struct Pending {
    host: SocketAddr,
    deadline: Instant,
}

/// A running (or not-yet-started) relay: the public socket plus every live registration, pending resolution and
/// established pairing.
pub struct RelayServer {
    socket: Arc<UdpSocket>,
    /// Captured once at `bind` time (not re-queried): the OS call can fail in principle, and `net`'s own lint
    /// policy denies `.expect()`/`.unwrap()` outside tests, so this is resolved where the `?` can still propagate.
    local_addr: SocketAddr,
    table: Arc<Mutex<RelayTable>>,
    pending: Arc<Mutex<HashMap<IpAddr, Pending>>>,
    pairs: Arc<Mutex<HashMap<SocketAddr, Pairing>>>,
    options: RelayServerOptions,
}

impl RelayServer {
    /// Binds the public socket. Does not start serving yet — call [`RelayServer::run`] (typically on its own
    /// thread) for that.
    pub fn bind(options: RelayServerOptions) -> io::Result<RelayServer> {
        let socket = UdpSocket::bind(options.bind)?;
        socket.set_read_timeout(Some(options.housekeeping_tick))?;
        let local_addr = socket.local_addr()?;
        Ok(RelayServer {
            socket: Arc::new(socket),
            local_addr,
            table: Arc::new(Mutex::new(RelayTable::new())),
            pending: Arc::new(Mutex::new(HashMap::new())),
            pairs: Arc::new(Mutex::new(HashMap::new())),
            options,
        })
    }

    /// The public socket's real address (useful when `bind` named port `0`).
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Hosts currently registered.
    pub fn registered_count(&self) -> usize {
        self.table.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    /// Clients currently being relayed (established pairings; a pending, not-yet-locked resolution is not counted).
    pub fn paired_count(&self) -> usize {
        self.pairs.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    /// Serves forever, until `stop` is set (checked at least once per `housekeeping_tick`). Blocking: run it on
    /// its own thread.
    pub fn run(&self, stop: &AtomicBool) {
        let mut buf = [0u8; 1500];
        while !stop.load(Ordering::Relaxed) {
            match self.socket.recv_from(&mut buf) {
                Ok((n, from)) => self.handle_packet(from, &buf[..n]),
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
                Err(e) => eprintln!("red_relay: recv error: {e}"),
            }
            let now = Instant::now();
            // Registrations expire independently of any paired forwarding session: a host that stops sending its
            // keepalive loses its code, but a session already in progress keeps running until its own idle timeout.
            self.table.lock().unwrap_or_else(|p| p.into_inner()).expire(now);
            self.pending.lock().unwrap_or_else(|p| p.into_inner()).retain(|_, p| p.deadline > now);
        }
    }

    fn handle_packet(&self, from: SocketAddr, data: &[u8]) {
        let existing = self.pairs.lock().unwrap_or_else(|p| p.into_inner()).get(&from).cloned();
        if let Some(pairing) = existing {
            let _ = pairing.to_host.send_to(data, pairing.host);
            return;
        }
        // Not yet paired by exact address. A genuine control message always wins over any stale pending-by-IP
        // state: an IP can legitimately resolve a second, unrelated code (a mistyped first one, a fresh one from
        // a friend) before its first resolution's pending window lapses, and that second `Resolve` must be
        // looked up on its own merits, not silently swallowed as if it were the first resolution's follow-up
        // traffic — which is exactly what checking "pending by IP" before trying to decode would do, since a
        // `Resolve`'s bytes are, from the relay's point of view, just as much "the first real packet from that
        // IP" as a QUIC handshake byte would be.
        match RelayMessage::decode(data) {
            Some(RelayMessage::Register { fingerprint }) => {
                let registered = self.table.lock().unwrap_or_else(|p| p.into_inner()).register(from, fingerprint, Instant::now());
                match registered {
                    Ok(code) => {
                        let _ = self.socket.send_to(&RelayMessage::Registered { code }.encode(), from);
                    }
                    Err(e) => eprintln!("red_relay: register failed for {from}: {e}"),
                }
            }
            Some(RelayMessage::Resolve { code }) => {
                let resolved = self.table.lock().unwrap_or_else(|p| p.into_inner()).resolve(&code);
                match resolved {
                    Some((host, fingerprint)) => {
                        let deadline = Instant::now() + self.options.pending_timeout;
                        self.pending.lock().unwrap_or_else(|p| p.into_inner()).insert(from.ip(), Pending { host, deadline });
                        let _ = self.socket.send_to(&RelayMessage::Resolved { fingerprint }.encode(), from);
                    }
                    None => {
                        let _ = self.socket.send_to(&RelayMessage::CodeNotFound.encode(), from);
                    }
                }
            }
            // Not a control message: is this the first real packet (a QUIC handshake byte, say) from an IP that
            // just resolved a code?
            None => {
                let pending_host = {
                    let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
                    match pending.get(&from.ip()) {
                        Some(p) if p.deadline > Instant::now() => {
                            let host = p.host;
                            pending.remove(&from.ip());
                            Some(host)
                        }
                        _ => None,
                    }
                };
                if let Some(host) = pending_host {
                    self.lock_pairing(from, host);
                    self.handle_packet(from, data); // now paired: forward this same packet through the normal path
                }
                // Otherwise: stray traffic from an address we have never paired, pending or registered — nothing sensible to answer.
            }
            // A reply-shaped message from an address that never registered or resolved anything: not ours to answer.
            Some(RelayMessage::Registered { .. } | RelayMessage::Resolved { .. } | RelayMessage::CodeNotFound) => {}
        }
    }

    /// Opens this pairing's own small socket to `host`, remembers it under `client`'s exact address, and starts
    /// its forwarding thread. Called the moment a pending-by-IP resolution sees its first real packet.
    fn lock_pairing(&self, client: SocketAddr, host: SocketAddr) {
        let to_host = match UdpSocket::bind(("0.0.0.0", 0)) {
            Ok(s) => Arc::new(s),
            Err(e) => {
                eprintln!("red_relay: could not open a forwarding socket for {client}: {e}");
                return;
            }
        };
        if let Err(e) = to_host.set_read_timeout(Some(self.options.pair_idle_timeout)) {
            eprintln!("red_relay: could not set the forwarding socket's timeout: {e}");
            return;
        }
        self.pairs.lock().unwrap_or_else(|p| p.into_inner()).insert(client, Pairing { to_host: to_host.clone(), host });
        let public = self.socket.clone();
        let pairs = self.pairs.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1500];
            loop {
                match to_host.recv_from(&mut buf) {
                    Ok((n, src)) if src == host => {
                        let _ = public.send_to(&buf[..n], client);
                    }
                    Ok(_) => {} // not the real host (a stray or spoofed packet on this ephemeral port): ignore it
                    Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => break,
                    Err(_) => break,
                }
            }
            pairs.lock().unwrap_or_else(|p| p.into_inner()).remove(&client);
        });
    }
}

/// How often a hosting `HostBridge` repeats `Register` to the relay, refreshing its lease (`RelayTable`'s
/// registration lease is a few minutes, so this keepalive runs comfortably inside that).
pub const DEFAULT_HOST_KEEPALIVE: Duration = Duration::from_secs(60);

/// The host side of the rendezvous: registers once with a relay, then bridges every distinct remote address the
/// relay forwards from to the real game server on loopback — one small local socket per remote address, so the
/// real server still sees every joiner as a distinct peer, exactly as `net::transport`'s own "peers are keyed by
/// the address a connection started from" already assumes. Nothing about `net::quic`/`net::server` has to know
/// it is being relayed: from their side, each bridged socket is just another local client on loopback.
pub struct HostBridge {
    control: Arc<UdpSocket>,
    relay_addr: SocketAddr,
    local_game_addr: SocketAddr,
    /// The game server's own TLS fingerprint (`sha256:<64 hex>`), if it has one, passed through to every joiner
    /// via the relay so nobody ever has to see or type it.
    fingerprint: Option<String>,
    /// One local loopback socket per *remote* address the relay has forwarded from (a joining player's own
    /// relay-assigned forwarding address) — keyed by that remote address so replies from the real game server go
    /// back out to the right one.
    bridges: Arc<Mutex<HashMap<SocketAddr, Arc<UdpSocket>>>>,
}

impl HostBridge {
    /// Registers with `relay_addr` (retrying a few times; a relay that never answers is a startup error, not a
    /// silent hang) and returns the running bridge plus the code a friend can join with. `fingerprint` is the
    /// game server's own TLS identity (`sha256:<64 hex>`, from `ServerTransport::fingerprint`), or `None` for a
    /// loopback development-UDP server — passed through to joining clients so they verify the real host without
    /// ever needing to see or type it themselves.
    pub fn start(
        relay_addr: SocketAddr,
        local_game_addr: SocketAddr,
        fingerprint: Option<String>,
        stop: Arc<AtomicBool>,
    ) -> io::Result<(HostBridge, [u8; super::relay::CODE_LEN])> {
        let control = Arc::new(UdpSocket::bind(("0.0.0.0", 0))?);
        control.set_read_timeout(Some(Duration::from_secs(10)))?;
        let code = register_with_retry(&control, relay_addr, fingerprint.clone())?;
        let bridge = HostBridge { control: control.clone(), relay_addr, local_game_addr, fingerprint, bridges: Arc::new(Mutex::new(HashMap::new())) };
        bridge.spawn_keepalive(stop.clone());
        bridge.spawn_forwarding(stop);
        Ok((bridge, code))
    }

    fn spawn_keepalive(&self, stop: Arc<AtomicBool>) {
        let control = self.control.clone();
        let relay_addr = self.relay_addr;
        let fingerprint = self.fingerprint.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(DEFAULT_HOST_KEEPALIVE);
                let _ = control.send_to(&RelayMessage::Register { fingerprint: fingerprint.clone() }.encode(), relay_addr);
            }
        });
    }

    fn spawn_forwarding(&self, stop: Arc<AtomicBool>) {
        let control = self.control.clone();
        let local_game_addr = self.local_game_addr;
        let bridges = self.bridges.clone();
        std::thread::spawn(move || {
            control.set_read_timeout(Some(Duration::from_millis(500))).ok();
            let mut buf = [0u8; 1500];
            while !stop.load(Ordering::Relaxed) {
                match control.recv_from(&mut buf) {
                    Ok((n, from)) => forward_from_relay(&control, &bridges, local_game_addr, from, &buf[..n], &stop),
                    Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
                    Err(_) => break,
                }
            }
        });
    }

    /// Players currently bridged through this host.
    pub fn bridged_count(&self) -> usize {
        self.bridges.lock().unwrap_or_else(|p| p.into_inner()).len()
    }
}

/// What a joining client does before ever touching `net::quic`: resolve a short code against a relay, getting
/// back the host's own fingerprint (if it has one) to pin — then treat `relay_addr` itself exactly like a normal
/// server address for everything after this (the relay is transparent to the QUIC handshake that follows; see
/// this module's own doc comment for why a different local port for that handshake is fine).
pub fn resolve_code(relay_addr: SocketAddr, code: super::relay::RelayCode, timeout: Duration) -> Result<Option<String>, String> {
    let socket = UdpSocket::bind(("0.0.0.0", 0)).map_err(|e| format!("could not reach the relay: {e}"))?;
    socket.set_read_timeout(Some(timeout)).map_err(|e| e.to_string())?;
    socket.send_to(&RelayMessage::Resolve { code }.encode(), relay_addr).map_err(|e| format!("could not reach the relay: {e}"))?;
    let mut buf = [0u8; 256];
    let (n, from) =
        socket.recv_from(&mut buf).map_err(|_| format!("the relay at {relay_addr} did not answer: check the address and your internet connection"))?;
    if from != relay_addr {
        return Err("got a reply from somewhere other than the relay: try again".to_string());
    }
    match RelayMessage::decode(&buf[..n]) {
        Some(RelayMessage::Resolved { fingerprint }) => Ok(fingerprint),
        Some(RelayMessage::CodeNotFound) => Err("that code is not live: ask your friend for a fresh one".to_string()),
        _ => Err("the relay sent something unexpected: try again".to_string()),
    }
}

fn register_with_retry(control: &UdpSocket, relay_addr: SocketAddr, fingerprint: Option<String>) -> io::Result<[u8; super::relay::CODE_LEN]> {
    let mut buf = [0u8; 128];
    for _ in 0..5 {
        control.send_to(&RelayMessage::Register { fingerprint: fingerprint.clone() }.encode(), relay_addr)?;
        match control.recv_from(&mut buf) {
            Ok((n, from)) if from == relay_addr => {
                if let Some(RelayMessage::Registered { code }) = RelayMessage::decode(&buf[..n]) {
                    return Ok(code);
                }
            }
            _ => {}
        }
    }
    Err(io::Error::new(io::ErrorKind::TimedOut, format!("the relay at {relay_addr} never answered Register")))
}

/// One datagram arriving from the relay, on the control socket: if it is already a known remote (a joined
/// player), forward it to the real local game server; otherwise it is a brand new joiner's first packet, so a
/// fresh local bridging socket is opened for it.
fn forward_from_relay(
    control: &Arc<UdpSocket>,
    bridges: &Arc<Mutex<HashMap<SocketAddr, Arc<UdpSocket>>>>,
    local_game_addr: SocketAddr,
    from: SocketAddr,
    data: &[u8],
    stop: &Arc<AtomicBool>,
) {
    let existing = bridges.lock().unwrap_or_else(|p| p.into_inner()).get(&from).cloned();
    if let Some(local) = existing {
        let _ = local.send_to(data, local_game_addr);
        return;
    }
    let local = match UdpSocket::bind(("127.0.0.1", 0)) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("red_server: could not open a local bridge socket for a new joiner: {e}");
            return;
        }
    };
    if local.set_read_timeout(Some(DEFAULT_PAIR_IDLE_TIMEOUT)).is_err() {
        return;
    }
    let _ = local.send_to(data, local_game_addr);
    bridges.lock().unwrap_or_else(|p| p.into_inner()).insert(from, local.clone());
    let control = control.clone();
    let bridges = bridges.clone();
    let stop = stop.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 1500];
        while !stop.load(Ordering::Relaxed) {
            match local.recv_from(&mut buf) {
                Ok((n, src)) if src == local_game_addr => {
                    let _ = control.send_to(&buf[..n], from);
                }
                Ok(_) => {}
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => break,
                Err(_) => break,
            }
        }
        bridges.lock().unwrap_or_else(|p| p.into_inner()).remove(&from);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::relay::generate_code;
    use std::net::SocketAddr;

    fn loopback() -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], 0))
    }

    /// Sets an `AtomicBool` stop flag on drop, including during a panicking unwind — used for a `HostBridge`'s
    /// own `stop` (its background threads are plain, unscoped `thread::spawn`s, so this is just tidiness: letting
    /// them notice quickly instead of lingering for their own idle timeout, never a hang risk on its own).
    struct StopOnDrop<'a>(&'a AtomicBool);
    impl Drop for StopOnDrop<'_> {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }

    /// Runs `body` with `relay` serving on a background thread, guaranteeing that thread is told to stop *before*
    /// any panic from `body` is allowed to unwind through `thread::scope`'s own join-wait. `thread::scope` always
    /// joins its spawned threads as part of unwinding past it — a plain `stop.store(true, ...)` at a test's tail
    /// never runs if an earlier assertion fails, so without this, one flaky assertion (a slow reply under real
    /// system load, say) wedges the whole test binary instead of reporting one failed test. Confirmed the hard
    /// way: a real run hit exactly this and every later test silently never ran.
    fn with_relay_running<R>(relay: &RelayServer, body: impl FnOnce() -> R + std::panic::UnwindSafe) -> R {
        let stop = AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| relay.run(&stop));
            let result = std::panic::catch_unwind(body);
            stop.store(true, Ordering::Relaxed);
            match result {
                Ok(v) => v,
                Err(e) => std::panic::resume_unwind(e),
            }
        })
    }

    #[test]
    fn a_host_bridge_and_a_relay_together_carry_real_traffic_from_a_real_client() {
        // The actual end-to-end shape: a "game server" standing on loopback, a HostBridge registering it with a
        // relay, and a "game client" (a plain socket standing in for net::quic's own, on yet another port) that
        // resolves the code and talks to the relay — never directly to the host at all.
        let relay = RelayServer::bind(fast_options()).unwrap();
        let relay_addr = relay.local_addr();
        with_relay_running(&relay, || {
            let game_server = UdpSocket::bind(loopback()).unwrap();
            game_server.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            let game_addr = game_server.local_addr().unwrap();

            let stop = Arc::new(AtomicBool::new(false));
            let _stop_guard = StopOnDrop(&stop);
            let (bridge, code) = HostBridge::start(relay_addr, game_addr, None, stop.clone()).unwrap();

            let client = UdpSocket::bind(loopback()).unwrap();
            client.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            client.send_to(&RelayMessage::Resolve { code }.encode(), relay_addr).unwrap();
            let mut buf = [0u8; 256];
            let (n, _) = client.recv_from(&mut buf).unwrap();
            assert_eq!(RelayMessage::decode(&buf[..n]), Some(RelayMessage::Resolved { fingerprint: None }));

            client.send_to(b"a hello from the real client", relay_addr).unwrap();
            let (n, from_bridge) = game_server.recv_from(&mut buf).unwrap();
            assert_eq!(&buf[..n], b"a hello from the real client");
            assert_ne!(from_bridge, game_addr);

            game_server.send_to(b"welcome from the real server", from_bridge).unwrap();
            let (n, from) = client.recv_from(&mut buf).unwrap();
            assert_eq!(&buf[..n], b"welcome from the real server");
            assert_eq!(from, relay_addr, "the client only ever hears from the relay's public address");
            assert_eq!(bridge.bridged_count(), 1);
        });
    }

    #[test]
    fn a_second_unrelated_resolve_from_the_same_ip_is_not_swallowed_by_the_first_ones_pending_state() {
        // Regression: `pending` is keyed by IP alone (a resolver's real QUIC traffic can arrive from a different
        // port than the one that resolved, see this module's own doc comment), which once meant a *second*,
        // completely unrelated `Resolve` from the same IP — a mistyped code, then a fresh one — could be
        // swallowed as if it were the first resolution's own follow-up packet, silently misrouted to the first
        // code's host instead of being looked up on its own merits. `handle_packet` must try to decode a control
        // message before ever consulting pending-by-IP state.
        let relay = RelayServer::bind(fast_options()).unwrap();
        let relay_addr = relay.local_addr();
        with_relay_running(&relay, || {
            let game_server = UdpSocket::bind(loopback()).unwrap();
            let game_addr = game_server.local_addr().unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let _stop_guard = StopOnDrop(&stop);
            let (_bridge, real_code) = HostBridge::start(relay_addr, game_addr, None, stop.clone()).unwrap();

            // First resolve: succeeds, leaves this IP "pending" (no follow-up packet ever sent on this socket).
            let fp = super::resolve_code(relay_addr, real_code, Duration::from_secs(10)).unwrap();
            assert_eq!(fp, None);

            // Second, unrelated resolve from the same IP (a different throwaway socket, same as the first):
            // must be looked up on its own, not misrouted as the first resolution's own real traffic.
            let err = super::resolve_code(relay_addr, generate_code().unwrap(), Duration::from_secs(10)).unwrap_err();
            assert!(err.contains("not live"), "{err}");
        });
    }

    #[test]
    fn resolve_code_returns_the_hosts_fingerprint_and_errors_on_a_dead_code() {
        let relay = RelayServer::bind(fast_options()).unwrap();
        let relay_addr = relay.local_addr();
        with_relay_running(&relay, || {
            let game_server = UdpSocket::bind(loopback()).unwrap();
            let game_addr = game_server.local_addr().unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let _stop_guard = StopOnDrop(&stop);
            let (_bridge, code) = HostBridge::start(relay_addr, game_addr, Some("sha256:aa".to_string()), stop.clone()).unwrap();

            let fp = super::resolve_code(relay_addr, code, Duration::from_secs(10)).unwrap();
            assert_eq!(fp.as_deref(), Some("sha256:aa"));

            let err = super::resolve_code(relay_addr, generate_code().unwrap(), Duration::from_secs(10)).unwrap_err();
            assert!(err.contains("not live"), "{err}");
        });
    }

    #[test]
    fn a_hosts_fingerprint_reaches_the_resolving_client_untouched() {
        // The whole point of threading a fingerprint through: a person only ever sees/types the short code, but
        // the client still gets the real host's TLS identity to pin, so the connection stays end-to-end verified
        // (ADR 0044) exactly as if they had pasted the long form.
        let relay = RelayServer::bind(fast_options()).unwrap();
        let relay_addr = relay.local_addr();
        with_relay_running(&relay, || {
            let game_server = UdpSocket::bind(loopback()).unwrap();
            let game_addr = game_server.local_addr().unwrap();
            let fingerprint = "sha256:deadbeef00000000000000000000000000000000000000000000000000000001".to_string();
            let stop = Arc::new(AtomicBool::new(false));
            let _stop_guard = StopOnDrop(&stop);
            let (_bridge, code) = HostBridge::start(relay_addr, game_addr, Some(fingerprint.clone()), stop.clone()).unwrap();

            let client = UdpSocket::bind(loopback()).unwrap();
            client.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            client.send_to(&RelayMessage::Resolve { code }.encode(), relay_addr).unwrap();
            let mut buf = [0u8; 256];
            let (n, _) = client.recv_from(&mut buf).unwrap();
            assert_eq!(RelayMessage::decode(&buf[..n]), Some(RelayMessage::Resolved { fingerprint: Some(fingerprint) }));
        });
    }

    fn fast_options() -> RelayServerOptions {
        RelayServerOptions {
            bind: loopback(),
            pair_idle_timeout: Duration::from_millis(300),
            pending_timeout: Duration::from_millis(300),
            housekeeping_tick: Duration::from_millis(20),
        }
    }

    #[test]
    fn a_host_registers_a_client_resolves_and_bytes_forward_both_ways() {
        let relay = RelayServer::bind(fast_options()).unwrap();
        let relay_addr = relay.local_addr();
        with_relay_running(&relay, || {
            let host = UdpSocket::bind(loopback()).unwrap();
            host.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            host.send_to(&RelayMessage::Register { fingerprint: None }.encode(), relay_addr).unwrap();
            let mut buf = [0u8; 64];
            let (n, from) = host.recv_from(&mut buf).unwrap();
            assert_eq!(from, relay_addr);
            let Some(RelayMessage::Registered { code }) = RelayMessage::decode(&buf[..n]) else { panic!("expected Registered") };
            assert_eq!(relay.registered_count(), 1);

            let client = UdpSocket::bind(loopback()).unwrap();
            client.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            client.send_to(&RelayMessage::Resolve { code }.encode(), relay_addr).unwrap();
            let (n, from) = client.recv_from(&mut buf).unwrap();
            assert_eq!(from, relay_addr);
            assert_eq!(RelayMessage::decode(&buf[..n]), Some(RelayMessage::Resolved { fingerprint: None }));

            // Client -> host, through the relay. This is the SAME socket that resolved, which is the common
            // case too (nothing requires a different port — only a different quinn-owned socket, as a real
            // game client uses, needs the IP-based promotion path; the next test covers that).
            client.send_to(b"hello from client", relay_addr).unwrap();
            let (n, from) = host.recv_from(&mut buf).unwrap();
            assert_eq!(&buf[..n], b"hello from client");
            assert_ne!(from, relay_addr, "the host sees the pairing's own forwarding socket, not the public one");
            let pair_addr = from;
            assert_eq!(relay.paired_count(), 1);

            // Host -> client, through that same pairing's socket.
            host.send_to(b"hello from host", pair_addr).unwrap();
            let (n, from) = client.recv_from(&mut buf).unwrap();
            assert_eq!(&buf[..n], b"hello from host");
            assert_eq!(from, relay_addr, "the client only ever hears from the relay's one public address");
        });
    }

    #[test]
    fn a_different_local_port_for_the_real_traffic_still_locks_in_by_ip() {
        // Models a real game client: resolve on one throwaway socket, then start sending real traffic from a
        // completely different one (what quinn's own client socket looks like from the relay's point of view).
        let relay = RelayServer::bind(fast_options()).unwrap();
        let relay_addr = relay.local_addr();
        with_relay_running(&relay, || {
            let host = UdpSocket::bind(loopback()).unwrap();
            host.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            host.send_to(&RelayMessage::Register { fingerprint: None }.encode(), relay_addr).unwrap();
            let mut buf = [0u8; 64];
            let (n, _) = host.recv_from(&mut buf).unwrap();
            let Some(RelayMessage::Registered { code }) = RelayMessage::decode(&buf[..n]) else { panic!("expected Registered") };

            let resolver = UdpSocket::bind(loopback()).unwrap();
            resolver.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            resolver.send_to(&RelayMessage::Resolve { code }.encode(), relay_addr).unwrap();
            let (n, _) = resolver.recv_from(&mut buf).unwrap();
            assert_eq!(RelayMessage::decode(&buf[..n]), Some(RelayMessage::Resolved { fingerprint: None }));
            drop(resolver); // the real client would move on to a different socket (quinn's own) here

            let quic_socket = UdpSocket::bind(loopback()).unwrap();
            quic_socket.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            quic_socket.send_to(b"a quic-looking packet", relay_addr).unwrap();
            let (n, _) = host.recv_from(&mut buf).unwrap();
            assert_eq!(&buf[..n], b"a quic-looking packet", "the relay locked the pairing to this new port by IP alone");
        });
    }

    #[test]
    fn two_clients_joining_the_same_host_get_distinct_forwarding_addresses() {
        let relay = RelayServer::bind(fast_options()).unwrap();
        let relay_addr = relay.local_addr();
        with_relay_running(&relay, || {
            let host = UdpSocket::bind(loopback()).unwrap();
            host.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            host.send_to(&RelayMessage::Register { fingerprint: None }.encode(), relay_addr).unwrap();
            let mut buf = [0u8; 64];
            let (n, _) = host.recv_from(&mut buf).unwrap();
            let Some(RelayMessage::Registered { code }) = RelayMessage::decode(&buf[..n]) else { panic!("expected Registered") };

            let mut from_addrs = Vec::new();
            for who in [b"alice".as_slice(), b"bob".as_slice()] {
                let client = UdpSocket::bind(loopback()).unwrap();
                client.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
                client.send_to(&RelayMessage::Resolve { code }.encode(), relay_addr).unwrap();
                client.recv_from(&mut buf).unwrap(); // Resolved
                client.send_to(who, relay_addr).unwrap();
                let (n, from) = host.recv_from(&mut buf).unwrap();
                assert_eq!(&buf[..n], who);
                from_addrs.push(from);
            }
            assert_ne!(from_addrs[0], from_addrs[1], "each joiner gets its own forwarding address at the host");
        });
    }

    #[test]
    fn resolving_an_unknown_code_is_told_so() {
        let relay = RelayServer::bind(fast_options()).unwrap();
        let relay_addr = relay.local_addr();
        with_relay_running(&relay, || {
            let client = UdpSocket::bind(loopback()).unwrap();
            client.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            client.send_to(&RelayMessage::Resolve { code: generate_code().unwrap() }.encode(), relay_addr).unwrap();
            let mut buf = [0u8; 64];
            let (n, _) = client.recv_from(&mut buf).unwrap();
            assert_eq!(RelayMessage::decode(&buf[..n]), Some(RelayMessage::CodeNotFound));
        });
    }

    #[test]
    fn an_idle_pairing_is_torn_down_and_its_count_drops() {
        let relay = RelayServer::bind(fast_options()).unwrap();
        let relay_addr = relay.local_addr();
        with_relay_running(&relay, || {
            let host = UdpSocket::bind(loopback()).unwrap();
            host.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            host.send_to(&RelayMessage::Register { fingerprint: None }.encode(), relay_addr).unwrap();
            let mut buf = [0u8; 64];
            let (n, _) = host.recv_from(&mut buf).unwrap();
            let Some(RelayMessage::Registered { code }) = RelayMessage::decode(&buf[..n]) else { panic!("expected Registered") };

            let client = UdpSocket::bind(loopback()).unwrap();
            client.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            client.send_to(&RelayMessage::Resolve { code }.encode(), relay_addr).unwrap();
            client.recv_from(&mut buf).unwrap();
            client.send_to(b"hi", relay_addr).unwrap(); // lock the pairing in (pending alone is not "paired")
            host.recv_from(&mut buf).unwrap();
            assert_eq!(relay.paired_count(), 1);

            std::thread::sleep(Duration::from_millis(600)); // past fast_options()'s 300ms pair_idle_timeout
            assert_eq!(relay.paired_count(), 0, "a pairing the host never answers on should tear itself down");
        });
    }

    #[test]
    fn a_pending_resolution_nobody_follows_up_on_expires() {
        let relay = RelayServer::bind(fast_options()).unwrap();
        let relay_addr = relay.local_addr();
        with_relay_running(&relay, || {
            let host = UdpSocket::bind(loopback()).unwrap();
            host.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            host.send_to(&RelayMessage::Register { fingerprint: None }.encode(), relay_addr).unwrap();
            let mut buf = [0u8; 64];
            let (n, _) = host.recv_from(&mut buf).unwrap();
            let Some(RelayMessage::Registered { code }) = RelayMessage::decode(&buf[..n]) else { panic!("expected Registered") };

            let client = UdpSocket::bind(loopback()).unwrap();
            client.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            client.send_to(&RelayMessage::Resolve { code }.encode(), relay_addr).unwrap();
            client.recv_from(&mut buf).unwrap(); // Resolved, but never actually followed up with real traffic
            assert_eq!(relay.pending.lock().unwrap().len(), 1);
            std::thread::sleep(Duration::from_millis(500)); // past fast_options()'s 300ms pending_timeout
            assert_eq!(relay.pending.lock().unwrap().len(), 0, "an abandoned resolution should not linger forever");
        });
    }
}
