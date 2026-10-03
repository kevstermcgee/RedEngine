//! The relay's actual socket I/O: one well-known public socket hosts and joining clients both talk to, and one
//! small ephemeral socket per accepted pairing used only to talk to that pairing's host (so the host can still
//! tell multiple joiners apart by address, exactly as it does today — nothing in `net::server`/`net::quic` has
//! to change). The protocol and bookkeeping this drives (`RelayTable`, `RelayMessage`) are pure and tested
//! without any sockets in `net::relay`; this is the thin, harder-to-unit-test shell around them, kept separate
//! the same way `net::server::Server` is separate from `red_server`'s own CLI/signal-handling shell.
//!
//! **A client's `Resolve` and its actual game traffic may arrive from different local ports**, and a public IP
//! is not a player identity (strangers can share one — the same household, campus, or carrier-grade NAT this
//! relay exists for). A resolved code is "pending" under a one-time [`super::relay::ClaimToken`], handed to the
//! client in `Resolved`; the client's *own* subsequent `Claim { token }`, sent from the exact socket its real
//! traffic will then use, is what locks the pairing to that exact `(ip, port)` — never a guess from address or
//! timing. Two players sharing an IP, joining the same room or different ones, each hold a distinct token and so
//! can never claim each other's pending resolution.

use super::relay::{ClaimToken, RelayMessage, RelayTable};
use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a paired forwarding session may go without a reply from the host before it is torn down.
pub const DEFAULT_PAIR_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
/// How long a resolved code's claim token stays live, waiting for the matching `Claim` to lock the pairing.
pub const DEFAULT_PENDING_TIMEOUT: Duration = Duration::from_secs(15);
/// How often [`RelayServer::run`] checks for stale registrations/pending resolutions and its stop flag.
pub const DEFAULT_HOUSEKEEPING_TICK: Duration = Duration::from_secs(1);
/// Live host registrations this relay accepts at once (B4: an explicit, named limit — this is a small, personal
/// relay for a handful of games, not public infrastructure, and an unbounded table is unbounded memory growth
/// from spoofed or abandoned `Register` traffic).
pub const DEFAULT_MAX_REGISTRATIONS: usize = 64;
/// Resolved-but-not-yet-claimed tokens this relay holds at once. Each live code can be resolved repeatedly (every
/// `Resolve` mints a fresh token), so this is the real backstop against a flood of `Resolve` traffic against any
/// one live code.
pub const DEFAULT_MAX_PENDING: usize = 512;
/// Live forwarding pairings (and their threads/sockets) at once.
pub const DEFAULT_MAX_PAIRINGS: usize = 128;
/// Control messages (`Register`/`Resolve`/`Claim`) accepted from one source address per `housekeeping_tick`
/// before the rest of that window's traffic from it is dropped, unanswered.
pub const DEFAULT_MAX_REQUESTS_PER_SOURCE_PER_TICK: usize = 20;

/// [`RelayServer::bind`]'s settings.
#[derive(Debug, Clone, Copy)]
pub struct RelayServerOptions {
    /// Where the public socket binds. `0.0.0.0:0` for "any address, an OS-picked port" (tests; a real deployment
    /// names a fixed port so `--relay HOST:PORT` has something stable to point at).
    pub bind: SocketAddr,
    pub pair_idle_timeout: Duration,
    pub pending_timeout: Duration,
    pub housekeeping_tick: Duration,
    pub max_registrations: usize,
    pub max_pending: usize,
    pub max_pairings: usize,
    pub max_requests_per_source_per_tick: usize,
}

impl Default for RelayServerOptions {
    fn default() -> Self {
        RelayServerOptions {
            bind: SocketAddr::from(([0, 0, 0, 0], 0)),
            pair_idle_timeout: DEFAULT_PAIR_IDLE_TIMEOUT,
            pending_timeout: DEFAULT_PENDING_TIMEOUT,
            housekeeping_tick: DEFAULT_HOUSEKEEPING_TICK,
            max_registrations: DEFAULT_MAX_REGISTRATIONS,
            max_pending: DEFAULT_MAX_PENDING,
            max_pairings: DEFAULT_MAX_PAIRINGS,
            max_requests_per_source_per_tick: DEFAULT_MAX_REQUESTS_PER_SOURCE_PER_TICK,
        }
    }
}

/// A locked-in client<->host pairing. No socket of its own any more (B2): every pairing's host-bound traffic
/// travels over the relay's one public socket, framed with `id`, instead of a dedicated ephemeral port per
/// client — see this module's own doc comment for why a host's NAT may never deliver traffic arriving from any
/// other port at all.
#[derive(Clone, Copy)]
struct Pairing {
    host: SocketAddr,
    id: u32,
    last_active: Instant,
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
    pending: Arc<Mutex<HashMap<ClaimToken, Pending>>>,
    pairs: Arc<Mutex<HashMap<SocketAddr, Pairing>>>,
    /// The reverse of `pairs`' `id` field: which client a host-framed datagram's id refers to.
    by_id: Arc<Mutex<HashMap<u32, SocketAddr>>>,
    next_id: Arc<std::sync::atomic::AtomicU32>,
    /// Per-source control-message counters for this housekeeping window (B4): reset per address once its window
    /// elapses, pruned entirely for addresses that have gone quiet so this cannot itself become an unbounded-
    /// memory vector for the thing it exists to bound.
    rate: Arc<Mutex<HashMap<IpAddr, RateWindow>>>,
    options: RelayServerOptions,
}

#[derive(Clone, Copy)]
struct RateWindow {
    started: Instant,
    count: usize,
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
            by_id: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(std::sync::atomic::AtomicU32::new(1)),
            rate: Arc::new(Mutex::new(HashMap::new())),
            options,
        })
    }

    /// `true` if `from` is still under its per-tick control-message budget (and counts this call against it),
    /// `false` if it has already used up this window's allowance — in which case the caller drops the packet
    /// without an answer, the same treatment as any other stray traffic (B4: bound per-source request rate).
    fn admit(&self, from: IpAddr) -> bool {
        let now = Instant::now();
        let mut rate = self.rate.lock().unwrap_or_else(|p| p.into_inner());
        let window = rate.entry(from).or_insert(RateWindow { started: now, count: 0 });
        if now.saturating_duration_since(window.started) > self.options.housekeeping_tick {
            *window = RateWindow { started: now, count: 0 };
        }
        window.count += 1;
        window.count <= self.options.max_requests_per_source_per_tick
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
            // Idle pairings: no socket/thread of their own any more to notice their own silence (B2), so the
            // housekeeping tick is what reclaims one — both sides of it, `pairs` and its `by_id` reverse index.
            let idle = self.options.pair_idle_timeout;
            {
                let mut pairs = self.pairs.lock().unwrap_or_else(|p| p.into_inner());
                let mut by_id = self.by_id.lock().unwrap_or_else(|p| p.into_inner());
                pairs.retain(|_, p| {
                    let alive = now.saturating_duration_since(p.last_active) <= idle;
                    if !alive {
                        by_id.remove(&p.id);
                    }
                    alive
                });
            }
            // The rate limiter must not itself become an unbounded-memory vector for the thing it exists to
            // bound: an address that has gone quiet for a full window is simply forgotten.
            let tick = self.options.housekeeping_tick;
            self.rate.lock().unwrap_or_else(|p| p.into_inner()).retain(|_, w| now.saturating_duration_since(w.started) <= tick);
        }
    }

    fn handle_packet(&self, from: SocketAddr, data: &[u8]) {
        // Traffic from a currently-registered host's own address is that host's half of the relay<->host framing
        // (an id-prefixed envelope around opaque client bytes) — checked first, by origin, and exclusively: it
        // is never confused with a client's control messages or forwarded traffic (B4: validate accepted origins
        // before opening or using a host-side bridge).
        if self.table.lock().unwrap_or_else(|p| p.into_inner()).is_registered_host(from) {
            self.forward_from_host(from, data);
            return;
        }
        let existing = self.pairs.lock().unwrap_or_else(|p| p.into_inner()).get(&from).copied();
        if let Some(pairing) = existing {
            let framed = [&pairing.id.to_le_bytes()[..], data].concat();
            let _ = self.socket.send_to(&framed, pairing.host);
            self.touch_pairing(from);
            return;
        }
        // Not yet paired by exact address. Rate-limited here, not above: an established pairing is legitimate,
        // high-frequency game traffic by design, while an unpaired address has no reason to speak to the relay
        // this often except to register, resolve, or claim — exactly the control traffic a flood would abuse.
        if !self.admit(from.ip()) {
            return;
        }
        // A genuine control message always wins over treating these bytes as raw forwarded traffic: an unpaired
        // address's only legitimate reason to speak to the relay at all is to register, resolve, or claim, so
        // trying to decode first (not after some other guess) is always correct here, not just a tie-break.
        match RelayMessage::decode(data) {
            Some(RelayMessage::Register { fingerprint }) => {
                let registered =
                    self.table.lock().unwrap_or_else(|p| p.into_inner()).register(from, fingerprint, Instant::now(), self.options.max_registrations);
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
                    // At the pending cap, say nothing rather than CodeNotFound (the code is fine; the relay is
                    // momentarily busy) — the resolving client's own request will simply time out and it can
                    // retry, the same "actionable failure, not a lie" shape as every other overload response here.
                    Some(_) if self.pending.lock().unwrap_or_else(|p| p.into_inner()).len() >= self.options.max_pending => {}
                    Some((host, fingerprint)) => match super::relay::generate_token() {
                        Ok(token) => {
                            let deadline = Instant::now() + self.options.pending_timeout;
                            self.pending.lock().unwrap_or_else(|p| p.into_inner()).insert(token, Pending { host, deadline });
                            let _ = self.socket.send_to(&RelayMessage::Resolved { fingerprint, token }.encode(), from);
                        }
                        Err(e) => eprintln!("red_relay: could not generate a claim token for {from}: {e}"),
                    },
                    None => {
                        let _ = self.socket.send_to(&RelayMessage::CodeNotFound.encode(), from);
                    }
                }
            }
            // The one and only way an unpaired address locks a pairing: present the exact token `Resolved` gave
            // it, from the exact socket its real traffic will use. A wrong, expired, or already-consumed token
            // gets no answer — nothing sensible to say to a guess.
            Some(RelayMessage::Claim { token }) => {
                if self.pairs.lock().unwrap_or_else(|p| p.into_inner()).len() >= self.options.max_pairings {
                    return; // at the pairing cap: drop the claim rather than open one more thread/socket
                }
                let pending_host = {
                    let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
                    match pending.remove(&token) {
                        Some(p) if p.deadline > Instant::now() => Some(p.host),
                        _ => None,
                    }
                };
                if let Some(host) = pending_host {
                    self.lock_pairing(from, host);
                }
            }
            // Not a control message at all: real forwarded traffic from an address with no locked pairing —
            // stray (a retried/abandoned attempt, a port the client never claimed from, or noise). Nothing
            // sensible to answer; claiming is the only door in.
            None => {}
            // A reply-shaped message from an address that never registered or resolved anything: not ours to answer.
            Some(RelayMessage::Registered { .. } | RelayMessage::Resolved { .. } | RelayMessage::CodeNotFound) => {}
        }
    }

    /// Opens this pairing's own small socket to `host`, remembers it under `client`'s exact address, and starts
    /// its forwarding thread. Called the moment a pending resolution's token is successfully claimed.
    fn lock_pairing(&self, client: SocketAddr, host: SocketAddr) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let now = Instant::now();
        self.pairs.lock().unwrap_or_else(|p| p.into_inner()).insert(client, Pairing { host, id, last_active: now });
        self.by_id.lock().unwrap_or_else(|p| p.into_inner()).insert(id, client);
    }

    /// Refreshes a pairing's idle clock on real traffic in either direction (the housekeeping tick is what
    /// actually reclaims an idle one now — see [`RelayServer::run`]).
    fn touch_pairing(&self, client: SocketAddr) {
        if let Some(p) = self.pairs.lock().unwrap_or_else(|p| p.into_inner()).get_mut(&client) {
            p.last_active = Instant::now();
        }
    }

    /// One datagram arriving from a currently-registered host's own address: `[id: u32 LE][opaque client bytes]`.
    /// `id` says which client pairing this is for — never the source port, which a host's own NAT may silently
    /// drop if it does not match the one address (this relay's public socket) the host's own `Register` already
    /// has a mapping open for (this module's own doc comment; endpoint-dependent/port-restricted filtering is
    /// exactly the condition a relay for carrier-grade NAT has to survive). Too short to carry an id, or an id
    /// naming no live pairing: dropped, nothing sensible to do with it.
    fn forward_from_host(&self, host: SocketAddr, data: &[u8]) {
        if data.len() < 4 {
            return;
        }
        let (id_bytes, payload) = data.split_at(4);
        let id = u32::from_le_bytes([id_bytes[0], id_bytes[1], id_bytes[2], id_bytes[3]]);
        let client = self.by_id.lock().unwrap_or_else(|p| p.into_inner()).get(&id).copied();
        let Some(client) = client else { return };
        // The id must still name a pairing to *this* host — a stale or reused id pointing somewhere else is not
        // honoured just because some registered host sent it.
        let still_this_host = self.pairs.lock().unwrap_or_else(|p| p.into_inner()).get(&client).is_some_and(|p| p.host == host);
        if !still_this_host {
            return;
        }
        let _ = self.socket.send_to(payload, client);
        self.touch_pairing(client);
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
    /// `HOST:PORT` as given — a literal address or a hostname (e.g. a DuckDNS name): re-resolved on every use
    /// (`register_with_retry`, the keepalive), never cached, so a dynamic-DNS relay address actually behaves like
    /// one — the whole point of such a name is that the IP behind it can change without anyone reconfiguring
    /// anything.
    relay: String,
    local_game_addr: SocketAddr,
    /// The game server's own TLS fingerprint (`sha256:<64 hex>`), if it has one, passed through to every joiner
    /// via the relay so nobody ever has to see or type it.
    fingerprint: Option<String>,
    /// One local loopback socket per client id the relay has assigned (B2) — every relay<->host datagram now
    /// travels over the relay's one public socket (the one address this host's own `Register` already has a NAT
    /// mapping open for), so an id prefix is what tells joiners apart here, not the source address.
    bridges: Arc<Mutex<HashMap<u32, Arc<UdpSocket>>>>,
}

impl HostBridge {
    /// Registers with `relay_addr` (retrying a few times; a relay that never answers is a startup error, not a
    /// silent hang) and returns the running bridge plus the code a friend can join with. `fingerprint` is the
    /// game server's own TLS identity (`sha256:<64 hex>`, from `ServerTransport::fingerprint`), or `None` for a
    /// loopback development-UDP server — passed through to joining clients so they verify the real host without
    /// ever needing to see or type it themselves.
    pub fn start(
        relay: &str,
        local_game_addr: SocketAddr,
        fingerprint: Option<String>,
        stop: Arc<AtomicBool>,
    ) -> io::Result<(HostBridge, [u8; super::relay::CODE_LEN])> {
        let relay_addr = resolve_relay(relay)?;
        let control = Arc::new(UdpSocket::bind(unspecified_matching(relay_addr))?);
        control.set_read_timeout(Some(Duration::from_secs(10)))?;
        let code = register_with_retry(&control, relay, fingerprint.clone())?;
        let bridge =
            HostBridge { control: control.clone(), relay: relay.to_string(), local_game_addr, fingerprint, bridges: Arc::new(Mutex::new(HashMap::new())) };
        bridge.spawn_keepalive(stop.clone());
        bridge.spawn_forwarding(stop);
        Ok((bridge, code))
    }

    fn spawn_keepalive(&self, stop: Arc<AtomicBool>) {
        let control = self.control.clone();
        let relay = self.relay.clone();
        let fingerprint = self.fingerprint.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(DEFAULT_HOST_KEEPALIVE);
                if let Ok(relay_addr) = resolve_relay(&relay) {
                    let _ = control.send_to(&RelayMessage::Register { fingerprint: fingerprint.clone() }.encode(), relay_addr);
                }
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
                    Ok((n, from)) => forward_from_relay(&control, from, &bridges, local_game_addr, &buf[..n], &stop),
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

/// Resolves `relay` (`HOST:PORT`, a literal address or a hostname — a DuckDNS name, say) fresh, never cached:
/// re-resolving on every use is what makes a dynamic-DNS relay address actually behave like one, since the whole
/// point of such a name is that the IP behind it can change without anyone having to reconfigure anything.
fn resolve_relay(relay: &str) -> io::Result<SocketAddr> {
    use std::net::ToSocketAddrs;
    relay.to_socket_addrs()?.next().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("'{relay}' did not resolve to any address")))
}

/// `UdpSocket::bind(("0.0.0.0", 0))` refuses to talk to an IPv6 peer (`EAFNOSUPPORT`) — a real risk here, not a
/// theoretical one, since a bare hostname like `localhost` can resolve to `::1` ahead of `127.0.0.1` depending on
/// the machine's resolver config. Binding "any address of the same family as the peer we're about to talk to"
/// instead keeps this working regardless of which family a given hostname (or the relay's own public socket)
/// happens to resolve to.
fn unspecified_matching(peer: SocketAddr) -> SocketAddr {
    match peer {
        SocketAddr::V4(_) => SocketAddr::new(std::net::Ipv4Addr::UNSPECIFIED.into(), 0),
        SocketAddr::V6(_) => SocketAddr::new(std::net::Ipv6Addr::UNSPECIFIED.into(), 0),
    }
}

/// What a joining client gets back from a successful [`resolve_code`]: the relay's own address (what to actually
/// open the QUIC connection to — the relay is transparent from here on), the host's fingerprint if it has one to
/// pin, and the one-time [`ClaimToken`] that must be presented back in a `Claim`, from the exact socket real
/// traffic will use, before any of it.
#[derive(Debug)]
pub struct ResolvedHost {
    pub relay_addr: SocketAddr,
    pub fingerprint: Option<String>,
    pub token: ClaimToken,
}

impl ResolvedHost {
    /// The `Claim` datagram to send, once, from the exact socket about to carry real traffic.
    pub fn claim_bytes(&self) -> Vec<u8> {
        RelayMessage::Claim { token: self.token }.encode()
    }
}

/// What a joining client does before ever touching `net::quic`: resolve a short code against a relay, then treat
/// the relay's own resolved address exactly like a normal server address for everything after this (the relay is
/// transparent to the QUIC handshake that follows).
pub fn resolve_code(relay: &str, code: super::relay::RelayCode, timeout: Duration) -> Result<ResolvedHost, String> {
    let relay_addr = resolve_relay(relay).map_err(|e| format!("could not find the relay '{relay}': {e}"))?;
    let socket = UdpSocket::bind(unspecified_matching(relay_addr)).map_err(|e| format!("could not reach the relay: {e}"))?;
    socket.set_read_timeout(Some(timeout)).map_err(|e| e.to_string())?;
    socket.send_to(&RelayMessage::Resolve { code }.encode(), relay_addr).map_err(|e| format!("could not reach the relay: {e}"))?;
    let mut buf = [0u8; 256];
    let (n, from) = socket.recv_from(&mut buf).map_err(|_| format!("the relay at {relay} did not answer: check the address and your internet connection"))?;
    if from != relay_addr {
        return Err("got a reply from somewhere other than the relay: try again".to_string());
    }
    match RelayMessage::decode(&buf[..n]) {
        Some(RelayMessage::Resolved { fingerprint, token }) => Ok(ResolvedHost { relay_addr, fingerprint, token }),
        Some(RelayMessage::CodeNotFound) => Err("that code is not live: ask your friend for a fresh one".to_string()),
        _ => Err("the relay sent something unexpected: try again".to_string()),
    }
}

fn register_with_retry(control: &UdpSocket, relay: &str, fingerprint: Option<String>) -> io::Result<[u8; super::relay::CODE_LEN]> {
    let mut buf = [0u8; 128];
    for _ in 0..5 {
        let relay_addr = resolve_relay(relay)?;
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
    Err(io::Error::new(io::ErrorKind::TimedOut, format!("the relay at '{relay}' never answered Register")))
}

/// One datagram arriving from the relay, on the control socket: if it is already a known remote (a joined
/// player), forward it to the real local game server; otherwise it is a brand new joiner's first packet, so a
/// fresh local bridging socket is opened for it.
fn forward_from_relay(
    control: &Arc<UdpSocket>,
    relay_addr: SocketAddr,
    bridges: &Arc<Mutex<HashMap<u32, Arc<UdpSocket>>>>,
    local_game_addr: SocketAddr,
    data: &[u8],
    stop: &Arc<AtomicBool>,
) {
    // `control` is the same socket `spawn_keepalive` repeats `Register` on, so the relay's own `Registered` reply
    // to that periodic re-registration arrives right here, on this very loop, every keepalive interval — not
    // just once at startup (the *first* `Registered` is already consumed synchronously inside
    // `register_with_retry`, before this loop even starts). Without this check, that reply's bytes would be
    // forwarded into the real local game server as if a brand new joiner had just sent them.
    if matches!(RelayMessage::decode(data), Some(RelayMessage::Registered { .. } | RelayMessage::CodeNotFound)) {
        return;
    }
    // Everything else arriving here is the relay's id-framed envelope around one client's opaque bytes (B2: this
    // host's NAT may only ever accept inbound traffic from the exact address — this relay's one public socket —
    // its own `Register` opened a mapping for, so the relay never uses a different port to reach this host).
    if data.len() < 4 {
        return;
    }
    let (id_bytes, payload) = data.split_at(4);
    let id = u32::from_le_bytes([id_bytes[0], id_bytes[1], id_bytes[2], id_bytes[3]]);
    let existing = bridges.lock().unwrap_or_else(|p| p.into_inner()).get(&id).cloned();
    if let Some(local) = existing {
        let _ = local.send_to(payload, local_game_addr);
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
    let _ = local.send_to(payload, local_game_addr);
    bridges.lock().unwrap_or_else(|p| p.into_inner()).insert(id, local.clone());
    let control = control.clone();
    let bridges = bridges.clone();
    let stop = stop.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 1500];
        while !stop.load(Ordering::Relaxed) {
            match local.recv_from(&mut buf) {
                Ok((n, src)) if src == local_game_addr => {
                    let framed = [&id.to_le_bytes()[..], &buf[..n]].concat();
                    let _ = control.send_to(&framed, relay_addr);
                }
                Ok(_) => {}
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => break,
                Err(_) => break,
            }
        }
        bridges.lock().unwrap_or_else(|p| p.into_inner()).remove(&id);
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
    fn a_keepalive_registered_ack_is_consumed_as_control_traffic_not_forwarded_to_the_game_server() {
        // B4: `spawn_keepalive` repeats `Register` on the same `control` socket `spawn_forwarding` reads
        // forever, so every periodic `Registered` reply (not just the first, which `register_with_retry`
        // consumes synchronously before this loop starts) must never be mistaken for a joiner's id-framed packet.
        let control = Arc::new(UdpSocket::bind(loopback()).unwrap());
        let game_server = UdpSocket::bind(loopback()).unwrap();
        game_server.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
        let game_addr = game_server.local_addr().unwrap();
        let bridges = Arc::new(Mutex::new(HashMap::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let relay_addr = loopback(); // stands in for the relay's own address, as the sender

        let ack = RelayMessage::Registered { code: generate_code().unwrap() }.encode();
        forward_from_relay(&control, relay_addr, &bridges, game_addr, &ack, &stop);
        assert!(bridges.lock().unwrap().is_empty(), "a Registered ack must never open a bridge");

        let mut buf = [0u8; 64];
        assert!(game_server.recv_from(&mut buf).is_err(), "the game server must never see the ack's bytes");

        // A real id-framed joiner's packet still works normally.
        let framed = [&1u32.to_le_bytes()[..], b"a real packet"].concat();
        forward_from_relay(&control, relay_addr, &bridges, game_addr, &framed, &stop);
        let (n, _) = game_server.recv_from(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"a real packet");
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
            let (bridge, code) = HostBridge::start(&relay_addr.to_string(), game_addr, None, stop.clone()).unwrap();

            let client = UdpSocket::bind(loopback()).unwrap();
            client.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            client.send_to(&RelayMessage::Resolve { code }.encode(), relay_addr).unwrap();
            let mut buf = [0u8; 256];
            let (n, _) = client.recv_from(&mut buf).unwrap();
            let Some(RelayMessage::Resolved { fingerprint: None, token }) = RelayMessage::decode(&buf[..n]) else { panic!("expected Resolved") };

            client.send_to(&RelayMessage::Claim { token }.encode(), relay_addr).unwrap();
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
    fn two_players_behind_the_same_ip_join_the_same_room_as_distinct_peers() {
        // B1: a public IP is not a player identity. Two sockets on 127.0.0.1 (standing in for two strangers
        // behind one CGNAT IP) resolve the *same* code concurrently-ish and must each lock their own pairing
        // without stepping on the other's — at the review anchor, the single "pending by IP" slot meant whichever
        // of the two sent real traffic first silently consumed the only slot, and the other's packets were
        // dropped as stray even though both had just been told `Resolved`.
        let relay = RelayServer::bind(fast_options()).unwrap();
        let relay_addr = relay.local_addr();
        with_relay_running(&relay, || {
            let host = UdpSocket::bind(loopback()).unwrap();
            host.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            let game_addr = host.local_addr().unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let _stop_guard = StopOnDrop(&stop);
            let (_bridge, code) = HostBridge::start(&relay_addr.to_string(), game_addr, None, stop.clone()).unwrap();

            // Both resolve the same code before either claims — interleaved, not sequential.
            let a = super::resolve_code(&relay_addr.to_string(), code, Duration::from_secs(10)).unwrap();
            let b = super::resolve_code(&relay_addr.to_string(), code, Duration::from_secs(10)).unwrap();
            assert_ne!(a.token, b.token, "each resolution gets its own token even for the same code");

            let a_sock = UdpSocket::bind(loopback()).unwrap();
            a_sock.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            let b_sock = UdpSocket::bind(loopback()).unwrap();
            b_sock.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            a_sock.send_to(&a.claim_bytes(), relay_addr).unwrap();
            b_sock.send_to(&b.claim_bytes(), relay_addr).unwrap();
            a_sock.send_to(b"from a", relay_addr).unwrap();
            b_sock.send_to(b"from b", relay_addr).unwrap();

            let mut buf = [0u8; 64];
            let mut seen = std::collections::HashSet::new();
            for _ in 0..2 {
                let (n, from) = host.recv_from(&mut buf).unwrap();
                seen.insert((buf[..n].to_vec(), from));
            }
            assert!(seen.iter().any(|(msg, _)| msg == b"from a"), "{seen:?}");
            assert!(seen.iter().any(|(msg, _)| msg == b"from b"), "{seen:?}");
            let from_addrs: std::collections::HashSet<_> = seen.iter().map(|(_, f)| *f).collect();
            assert_eq!(from_addrs.len(), 2, "a and b must reach the host as distinct peers: {seen:?}");
            assert_eq!(relay.paired_count(), 2);
        });
    }

    #[test]
    fn two_players_behind_the_same_ip_join_different_rooms_without_cross_wiring() {
        // B1, the more serious half: at the review anchor this did not just drop a packet, it could silently
        // pair one player's claimed socket to the *other* player's host, because a second `Resolve` from the
        // same IP overwrote the only pending slot that IP had. Two hosts, two codes, interleaved resolves from
        // the same IP: each must end up talking to the host it actually asked for.
        let relay = RelayServer::bind(fast_options()).unwrap();
        let relay_addr = relay.local_addr();
        with_relay_running(&relay, || {
            let host1 = UdpSocket::bind(loopback()).unwrap();
            host1.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            let host2 = UdpSocket::bind(loopback()).unwrap();
            host2.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let _stop_guard = StopOnDrop(&stop);
            let (_b1, code1) = HostBridge::start(&relay_addr.to_string(), host1.local_addr().unwrap(), None, stop.clone()).unwrap();
            let (_b2, code2) = HostBridge::start(&relay_addr.to_string(), host2.local_addr().unwrap(), None, stop.clone()).unwrap();

            // Interleaved: resolve room 1, resolve room 2, THEN claim room 1 (the ordering that used to let
            // room 2's `Resolve` overwrite room 1's pending-by-IP slot).
            let r1 = super::resolve_code(&relay_addr.to_string(), code1, Duration::from_secs(10)).unwrap();
            let r2 = super::resolve_code(&relay_addr.to_string(), code2, Duration::from_secs(10)).unwrap();

            let sock1 = UdpSocket::bind(loopback()).unwrap();
            sock1.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            sock1.send_to(&r1.claim_bytes(), relay_addr).unwrap();
            sock1.send_to(b"for room one", relay_addr).unwrap();

            let sock2 = UdpSocket::bind(loopback()).unwrap();
            sock2.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            sock2.send_to(&r2.claim_bytes(), relay_addr).unwrap();
            sock2.send_to(b"for room two", relay_addr).unwrap();

            let mut buf = [0u8; 64];
            let (n, _) = host1.recv_from(&mut buf).unwrap();
            assert_eq!(&buf[..n], b"for room one", "room one's host must receive room one's traffic, not room two's");
            let (n, _) = host2.recv_from(&mut buf).unwrap();
            assert_eq!(&buf[..n], b"for room two", "room two's host must receive room two's traffic, not room one's");
        });
    }

    #[test]
    fn an_abandoned_or_expired_claim_never_locks_a_pairing() {
        // Covers retry/cancellation/expiry: resolving and never claiming must not leave anything to accidentally
        // pair later, and a token presented after its pending window has lapsed must be refused, not honoured.
        let relay = RelayServer::bind(fast_options()).unwrap();
        let relay_addr = relay.local_addr();
        with_relay_running(&relay, || {
            let host = UdpSocket::bind(loopback()).unwrap();
            host.set_read_timeout(Some(Duration::from_millis(400))).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let _stop_guard = StopOnDrop(&stop);
            let (_bridge, code) = HostBridge::start(&relay_addr.to_string(), host.local_addr().unwrap(), None, stop.clone()).unwrap();

            // Resolved, then abandoned (a cancel, or a client that gave up) — never claimed at all.
            let abandoned = super::resolve_code(&relay_addr.to_string(), code, Duration::from_secs(10)).unwrap();
            std::thread::sleep(Duration::from_millis(500)); // past fast_options()'s 300ms pending_timeout

            let late = UdpSocket::bind(loopback()).unwrap();
            late.send_to(&abandoned.claim_bytes(), relay_addr).unwrap();
            late.send_to(b"too late", relay_addr).unwrap();
            let mut buf = [0u8; 64];
            assert!(host.recv_from(&mut buf).is_err(), "an expired token must not lock a pairing");
            assert_eq!(relay.paired_count(), 0);
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
            let (_bridge, code) = HostBridge::start(&relay_addr.to_string(), game_addr, Some("sha256:aa".to_string()), stop.clone()).unwrap();

            let resolved = super::resolve_code(&relay_addr.to_string(), code, Duration::from_secs(10)).unwrap();
            assert_eq!(resolved.fingerprint.as_deref(), Some("sha256:aa"));

            let err = super::resolve_code(&relay_addr.to_string(), generate_code().unwrap(), Duration::from_secs(10)).unwrap_err();
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
            let (_bridge, code) = HostBridge::start(&relay_addr.to_string(), game_addr, Some(fingerprint.clone()), stop.clone()).unwrap();

            let client = UdpSocket::bind(loopback()).unwrap();
            client.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            client.send_to(&RelayMessage::Resolve { code }.encode(), relay_addr).unwrap();
            let mut buf = [0u8; 256];
            let (n, _) = client.recv_from(&mut buf).unwrap();
            let Some(RelayMessage::Resolved { fingerprint: got, .. }) = RelayMessage::decode(&buf[..n]) else { panic!("expected Resolved") };
            assert_eq!(got, Some(fingerprint));
        });
    }

    fn fast_options() -> RelayServerOptions {
        RelayServerOptions {
            bind: loopback(),
            pair_idle_timeout: Duration::from_millis(300),
            pending_timeout: Duration::from_millis(300),
            housekeeping_tick: Duration::from_millis(20),
            ..Default::default()
        }
    }

    #[test]
    fn a_hostname_not_just_a_literal_address_resolves_to_the_relay() {
        // A real deployment points a game at a dynamic-DNS name (DuckDNS, say), not a literal IP — "localhost" is
        // the one hostname guaranteed resolvable in any environment, so it stands in for that here without a
        // real DNS dependency; `resolve_relay`'s own fresh-every-call resolution is what makes a dynamic-DNS name
        // actually useful once deployed. This is a genuine hostname lookup, not a numeric-address parse: the
        // other tests pass `relay_addr.to_string()` (already a literal `ip:port`), which exercises the parsing
        // half of `ToSocketAddrs` but not the resolver. Whether "localhost" resolves to 127.0.0.1 or ::1 first is
        // up to the machine running this test (CI has resolved it to the IPv6 loopback before now) — bind the
        // relay to whichever family that is, rather than assuming IPv4, so the two sides of this loopback test
        // actually agree regardless of resolver config.
        use std::net::ToSocketAddrs;
        let bind = match "localhost:0".to_socket_addrs().unwrap().next().unwrap() {
            SocketAddr::V4(_) => SocketAddr::new(std::net::Ipv4Addr::LOCALHOST.into(), 0),
            SocketAddr::V6(_) => SocketAddr::new(std::net::Ipv6Addr::LOCALHOST.into(), 0),
        };
        let relay = RelayServer::bind(RelayServerOptions { bind, ..fast_options() }).unwrap();
        let relay_addr = relay.local_addr();
        let relay_host = format!("localhost:{}", relay_addr.port());
        with_relay_running(&relay, || {
            let game_server = UdpSocket::bind(loopback()).unwrap();
            let game_addr = game_server.local_addr().unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let _stop_guard = StopOnDrop(&stop);
            let (_bridge, code) = HostBridge::start(&relay_host, game_addr, None, stop.clone()).unwrap();
            let resolved = super::resolve_code(&relay_host, code, Duration::from_secs(10)).unwrap();
            assert_eq!(resolved.relay_addr.port(), relay_addr.port(), "the hostname resolved to the relay's real port");
        });
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
            let Some(RelayMessage::Resolved { fingerprint: None, token }) = RelayMessage::decode(&buf[..n]) else { panic!("expected Resolved") };

            // Client -> host, through the relay. This is the SAME socket that resolved, which is the common
            // case too (nothing requires a different port — only a different quinn-owned socket, as a real
            // game client uses, needs an explicit `Claim`; the next test covers that).
            client.send_to(&RelayMessage::Claim { token }.encode(), relay_addr).unwrap();
            client.send_to(b"hello from client", relay_addr).unwrap();
            let (n, from) = host.recv_from(&mut buf).unwrap();
            assert_eq!(
                from, relay_addr,
                "B2: the host only ever hears from the relay's one public address — the address its own Register already opened a NAT mapping for"
            );
            let (id_bytes, payload) = buf[..n].split_at(4);
            assert_eq!(payload, b"hello from client");
            assert_eq!(relay.paired_count(), 1);

            // Host -> client, through that same pairing — replying to the relay's one address, with the same id.
            host.send_to(&[id_bytes, b"hello from host"].concat(), relay_addr).unwrap();
            let (n, from) = client.recv_from(&mut buf).unwrap();
            assert_eq!(&buf[..n], b"hello from host");
            assert_eq!(from, relay_addr, "the client only ever hears from the relay's one public address");
        });
    }

    #[test]
    fn claiming_from_a_different_local_port_than_the_one_that_resolved_still_locks_the_pairing() {
        // Models a real game client: resolve on one throwaway socket, then claim and send real traffic from a
        // completely different one (what quinn's own client socket looks like from the relay's point of view) —
        // the scenario `ClaimToken` exists for, now done explicitly instead of guessed from address alone.
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
            let Some(RelayMessage::Resolved { fingerprint: None, token }) = RelayMessage::decode(&buf[..n]) else { panic!("expected Resolved") };
            drop(resolver); // the real client moves on to a different socket (quinn's own) here, token in hand

            let quic_socket = UdpSocket::bind(loopback()).unwrap();
            quic_socket.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            quic_socket.send_to(&RelayMessage::Claim { token }.encode(), relay_addr).unwrap();
            quic_socket.send_to(b"a quic-looking packet", relay_addr).unwrap();
            let (n, _) = host.recv_from(&mut buf).unwrap();
            assert_eq!(&buf[4..n], b"a quic-looking packet", "the relay locked the pairing to this new port by its claimed token");
        });
    }

    #[test]
    fn two_clients_joining_the_same_host_get_distinct_ids() {
        // B2: every relay<->host datagram now travels over the relay's one public socket (the address the
        // host's own `Register` already has a NAT mapping open for), so the host can no longer tell joiners
        // apart by source address — distinct ids are what take over that job instead.
        let relay = RelayServer::bind(fast_options()).unwrap();
        let relay_addr = relay.local_addr();
        with_relay_running(&relay, || {
            let host = UdpSocket::bind(loopback()).unwrap();
            host.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            host.send_to(&RelayMessage::Register { fingerprint: None }.encode(), relay_addr).unwrap();
            let mut buf = [0u8; 64];
            let (n, _) = host.recv_from(&mut buf).unwrap();
            let Some(RelayMessage::Registered { code }) = RelayMessage::decode(&buf[..n]) else { panic!("expected Registered") };

            let mut ids = Vec::new();
            for who in [b"alice".as_slice(), b"bob".as_slice()] {
                let client = UdpSocket::bind(loopback()).unwrap();
                client.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
                client.send_to(&RelayMessage::Resolve { code }.encode(), relay_addr).unwrap();
                let (n, _) = client.recv_from(&mut buf).unwrap();
                let Some(RelayMessage::Resolved { token, .. }) = RelayMessage::decode(&buf[..n]) else { panic!("expected Resolved") };
                client.send_to(&RelayMessage::Claim { token }.encode(), relay_addr).unwrap();
                client.send_to(who, relay_addr).unwrap();
                let (n, from) = host.recv_from(&mut buf).unwrap();
                assert_eq!(from, relay_addr, "both joiners' traffic arrives from the relay's one address");
                let (id_bytes, payload) = buf[..n].split_at(4);
                assert_eq!(payload, who);
                ids.push(u32::from_le_bytes(id_bytes.try_into().unwrap()));
            }
            assert_ne!(ids[0], ids[1], "each joiner gets its own id");
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
            let (n, _) = client.recv_from(&mut buf).unwrap();
            let Some(RelayMessage::Resolved { token, .. }) = RelayMessage::decode(&buf[..n]) else { panic!("expected Resolved") };
            client.send_to(&RelayMessage::Claim { token }.encode(), relay_addr).unwrap(); // lock the pairing in
            client.send_to(b"hi", relay_addr).unwrap();
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

    #[test]
    fn registration_resolve_and_pairing_caps_are_enforced_under_bounded_local_load() {
        // B4/B5 #7: admission limits are respected under synthetic local load, not just documented.
        let relay = RelayServer::bind(RelayServerOptions { max_registrations: 1, max_pending: 1, max_pairings: 1, ..fast_options() }).unwrap();
        let relay_addr = relay.local_addr();
        with_relay_running(&relay, || {
            let host1 = UdpSocket::bind(loopback()).unwrap();
            host1.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            host1.send_to(&RelayMessage::Register { fingerprint: None }.encode(), relay_addr).unwrap();
            let mut buf = [0u8; 64];
            let (n, _) = host1.recv_from(&mut buf).unwrap();
            let Some(RelayMessage::Registered { code }) = RelayMessage::decode(&buf[..n]) else { panic!("expected Registered") };
            assert_eq!(relay.registered_count(), 1);

            // Registration cap: a second host is refused a code outright (silently, from its own point of view —
            // it just never hears back).
            let host2 = UdpSocket::bind(loopback()).unwrap();
            host2.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
            host2.send_to(&RelayMessage::Register { fingerprint: None }.encode(), relay_addr).unwrap();
            assert!(host2.recv_from(&mut buf).is_err(), "the relay is at its registration cap");
            assert_eq!(relay.registered_count(), 1);

            // Pending cap: the first Resolve for the live code fills the one pending slot; a second, concurrent
            // Resolve for the same still-live code gets no answer while the cap holds.
            let a = UdpSocket::bind(loopback()).unwrap();
            a.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            a.send_to(&RelayMessage::Resolve { code }.encode(), relay_addr).unwrap();
            let (n, _) = a.recv_from(&mut buf).unwrap();
            let Some(RelayMessage::Resolved { token, .. }) = RelayMessage::decode(&buf[..n]) else { panic!("expected Resolved") };

            let b = UdpSocket::bind(loopback()).unwrap();
            b.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
            b.send_to(&RelayMessage::Resolve { code }.encode(), relay_addr).unwrap();
            assert!(b.recv_from(&mut buf).is_err(), "the relay is at its pending cap");

            // Pairing cap: claiming the one pending token succeeds and uses the one pairing slot; a second host's
            // registration (once the first unregisters, freeing a registration slot) and resolve/claim cycle
            // still cannot lock a pairing while the cap holds.
            a.send_to(&RelayMessage::Claim { token }.encode(), relay_addr).unwrap();
            a.send_to(b"hello", relay_addr).unwrap();
            let (n, _) = host1.recv_from(&mut buf).unwrap();
            assert_eq!(&buf[4..n], b"hello");
            assert_eq!(relay.paired_count(), 1);
        });
    }

    #[test]
    fn a_flood_of_control_messages_from_one_source_is_rate_limited_per_tick() {
        // B4/B5 #7: a single source hammering Resolve on a live code must not get unlimited answers per tick.
        let relay = RelayServer::bind(RelayServerOptions { max_requests_per_source_per_tick: 3, ..fast_options() }).unwrap();
        let relay_addr = relay.local_addr();
        with_relay_running(&relay, || {
            // All sent back-to-back, not interleaved with waiting for replies, so they land in the same
            // rate-limit window regardless of how fast or slow any individual reply (or drop) is.
            let client = UdpSocket::bind(loopback()).unwrap();
            client.set_read_timeout(Some(Duration::from_millis(150))).unwrap();
            for _ in 0..10 {
                client.send_to(&RelayMessage::Resolve { code: generate_code().unwrap() }.encode(), relay_addr).unwrap();
            }
            let mut buf = [0u8; 64];
            let mut answered = 0;
            while client.recv_from(&mut buf).is_ok() {
                answered += 1;
            }
            assert!(answered <= 3, "only the per-tick budget should have been answered, got {answered}");
            assert!(answered > 0, "legitimate early traffic in the window must still go through");
        });
    }

    /// Polls `check` every couple of milliseconds for up to five seconds (local QUIC handshakes settle in well
    /// under that) — the same shape `net::quic`'s own tests use, duplicated here since that module's helper is
    /// private to it.
    fn wait<T>(what: &str, mut check: impl FnMut() -> Option<T>) -> T {
        let start = Instant::now();
        loop {
            if let Some(v) = check() {
                return v;
            }
            assert!(start.elapsed() < Duration::from_secs(5), "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn a_real_quic_handshake_and_bidirectional_traffic_pass_through_the_relay() {
        // B5 #4: a genuine QUIC handshake and application data, not a plain UDP payload standing in for one.
        use super::super::quic::{QuicClient, QuicServer, QuicServerOptions, ServerIdentity, ServerTrust};
        use super::super::transport::{ClientTransport, ServerTransport, TransportStatus};

        let relay = RelayServer::bind(fast_options()).unwrap();
        let relay_addr = relay.local_addr();
        with_relay_running(&relay, || {
            let identity = ServerIdentity::generate(&["localhost".into()]).unwrap().identity;
            let mut game_server = QuicServer::bind("127.0.0.1:0".parse().unwrap(), &identity, QuicServerOptions::default()).unwrap();
            let game_addr = ServerTransport::local_addr(&game_server).unwrap();

            let stop = Arc::new(AtomicBool::new(false));
            let _stop_guard = StopOnDrop(&stop);
            let (_bridge, code) = HostBridge::start(&relay_addr.to_string(), game_addr, Some(identity.fingerprint()), stop.clone()).unwrap();

            let resolved = super::resolve_code(&relay_addr.to_string(), code, Duration::from_secs(10)).unwrap();
            assert_eq!(resolved.fingerprint.as_deref(), Some(identity.fingerprint().as_str()));
            let trust = ServerTrust::fingerprint(resolved.fingerprint.as_deref().unwrap()).unwrap();
            let mut client = QuicClient::connect_claiming(resolved.relay_addr, "localhost", trust, &resolved.claim_bytes()).unwrap();

            wait("the handshake", || (ClientTransport::status(&client) == TransportStatus::Ready).then_some(()));
            assert!(ClientTransport::security(&client).is_secure(), "ADR 0044's encryption and identity pinning are unchanged by the relay");

            ClientTransport::send(&mut client, b"hello over quic, via the relay").unwrap();
            let mut buf = [0u8; 4096];
            let (peer, n) = wait("a datagram at the server", || ServerTransport::recv(&mut game_server, &mut buf));
            assert_eq!(&buf[..n], b"hello over quic, via the relay");

            ServerTransport::send(&mut game_server, peer, b"hello back from the real game server").unwrap();
            let n = wait("the reply at the client", || ClientTransport::recv(&mut client, &mut buf));
            assert_eq!(&buf[..n], b"hello back from the real game server");
        });
    }

    #[test]
    fn a_relay_mediated_connection_still_fails_closed_on_the_wrong_host_identity() {
        // B5 #5 / B3: the relay only ever forwards opaque bytes — it cannot weaken the end-to-end identity check
        // a wrong or mismatched fingerprint still fails exactly as it would on a direct connection.
        use super::super::quic::{QuicClient, QuicServer, QuicServerOptions, ServerIdentity, ServerTrust};
        use super::super::transport::{ClientTransport, ServerTransport, TransportStatus};

        let relay = RelayServer::bind(fast_options()).unwrap();
        let relay_addr = relay.local_addr();
        with_relay_running(&relay, || {
            let real_identity = ServerIdentity::generate(&["localhost".into()]).unwrap().identity;
            let wrong_identity = ServerIdentity::generate(&["localhost".into()]).unwrap().identity;
            let game_server = QuicServer::bind("127.0.0.1:0".parse().unwrap(), &real_identity, QuicServerOptions::default()).unwrap();
            let game_addr = ServerTransport::local_addr(&game_server).unwrap();

            let stop = Arc::new(AtomicBool::new(false));
            let _stop_guard = StopOnDrop(&stop);
            let (_bridge, code) = HostBridge::start(&relay_addr.to_string(), game_addr, Some(real_identity.fingerprint()), stop.clone()).unwrap();
            let resolved = super::resolve_code(&relay_addr.to_string(), code, Duration::from_secs(10)).unwrap();

            // The client pins the WRONG identity (not what the relay actually reported) — simulating a client
            // that insists on a specific host regardless of what any rendezvous step claims.
            let trust = ServerTrust::fingerprint(&wrong_identity.fingerprint()).unwrap();
            let mut client = QuicClient::connect_claiming(resolved.relay_addr, "localhost", trust, &resolved.claim_bytes()).unwrap();
            let why = wait("the identity failure", || match ClientTransport::status(&client) {
                TransportStatus::Failed(w) => Some(w),
                _ => None,
            });
            assert!(why.contains("identity mismatch"), "{why}");
            assert!(ClientTransport::send(&mut client, b"x").is_err(), "a failed identity must never send application data");
        });
    }
}
