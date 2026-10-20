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
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a paired forwarding session may go without any traffic before it is torn down (counted from the last datagram either way).
/// This is a friend's window to come back after their network drops: the claim that locked the pairing is single-use, so once the relay
/// forgets it a reconnecting client's packets are strays and it can only JOIN again. The host gives up on a silent player after seconds
/// and parks their place, so the relay's patience is what decides whether an automatic reconnect (a laptop that slept, a Wi-Fi hand-over)
/// still works: ten minutes covers those, and a pairing is a few dozen bytes.
pub const DEFAULT_PAIR_IDLE_TIMEOUT: Duration = Duration::from_secs(600);
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
    /// How long a host's code stays live without a keepalive (`net::relay::REGISTRATION_TIMEOUT` by default).
    pub registration_lease: Duration,
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
            registration_lease: super::relay::REGISTRATION_TIMEOUT,
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
            let expired = self.table.lock().unwrap_or_else(|p| p.into_inner()).expire_after(now, self.options.registration_lease);
            for host in expired {
                self.drop_pairings_of(host);
            }
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
            self.handle_from_host(from, data);
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
            // A goodbye from an address that never registered (or whose lease already ran out): nothing to drop.
            Some(RelayMessage::Registered { .. } | RelayMessage::Resolved { .. } | RelayMessage::CodeNotFound | RelayMessage::Unregister) => {}
        }
    }

    /// A datagram from a registered host's own address: its keepalive (`Register` again), its goodbye (`Unregister`), or the
    /// id-framed envelope around one joiner's bytes. The first two are told apart from the third by shape alone, which is safe
    /// because an envelope starts with a four-byte pairing id: `Unregister` is one byte, a `Register` without a fingerprint two,
    /// and one with a fingerprint has `sha256:` at offset 2, where an envelope has the top half of an id that would have to be
    /// above a billion. Without this a host's keepalive was swallowed as framing and its code expired five minutes in, however
    /// busy the match was.
    fn handle_from_host(&self, host: SocketAddr, data: &[u8]) {
        match RelayMessage::decode(data) {
            Some(RelayMessage::Unregister) => {
                self.table.lock().unwrap_or_else(|p| p.into_inner()).unregister(host);
                self.drop_pairings_of(host);
            }
            Some(RelayMessage::Register { fingerprint }) if data.len() < 4 || fingerprint.as_deref().is_some_and(|f| f.starts_with("sha256:")) => {
                let renewed = self.table.lock().unwrap_or_else(|p| p.into_inner()).register(host, fingerprint, Instant::now(), self.options.max_registrations);
                if let Ok(code) = renewed {
                    let _ = self.socket.send_to(&RelayMessage::Registered { code }.encode(), host);
                }
            }
            _ => self.forward_from_host(host, data),
        }
    }

    /// Forgets every pairing that leads to `host` (it left or its lease ran out), so its joiners stop being forwarded to nobody.
    fn drop_pairings_of(&self, host: SocketAddr) {
        let mut pairs = self.pairs.lock().unwrap_or_else(|p| p.into_inner());
        let mut by_id = self.by_id.lock().unwrap_or_else(|p| p.into_inner());
        pairs.retain(|_, p| {
            let keep = p.host != host;
            if !keep {
                by_id.remove(&p.id);
            }
            keep
        });
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
    /// What the relay last said this host's code is (see [`HostBridge::code`]).
    shared: Arc<HostShared>,
}

/// What the keepalive thread and the forwarding thread of a [`HostBridge`] share with whoever shows the host its code.
struct HostShared {
    /// The code the relay last answered `Register` with.
    code: Mutex<super::relay::RelayCode>,
    /// How many times that answer differed from the one before it (a relay that restarted, a NAT that rebound).
    changes: AtomicU32,
    /// The relay's address as of the last `Register` (a name that resolves to a new address moves it): only an answer from there counts.
    relay_addr: Mutex<SocketAddr>,
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
        Self::start_with_keepalive(relay, local_game_addr, fingerprint, stop, DEFAULT_HOST_KEEPALIVE)
    }

    /// [`start`](Self::start) with a keepalive interval other than [`DEFAULT_HOST_KEEPALIVE`] (a relay run with a short lease, in tests).
    pub fn start_with_keepalive(
        relay: &str,
        local_game_addr: SocketAddr,
        fingerprint: Option<String>,
        stop: Arc<AtomicBool>,
        keepalive: Duration,
    ) -> io::Result<(HostBridge, [u8; super::relay::CODE_LEN])> {
        Self::start_with(relay, local_game_addr, fingerprint, stop, keepalive, RegisterTiming::default())
    }

    /// [`start_with_keepalive`](Self::start_with_keepalive) with the patience of the first registration set too (a test that must not wait nine seconds
    /// for a relay that is not there).
    pub fn start_with(
        relay: &str,
        local_game_addr: SocketAddr,
        fingerprint: Option<String>,
        stop: Arc<AtomicBool>,
        keepalive: Duration,
        timing: RegisterTiming,
    ) -> io::Result<(HostBridge, [u8; super::relay::CODE_LEN])> {
        let relay_addr = resolve_relay_within(relay, DNS_BUDGET)?;
        let control = Arc::new(UdpSocket::bind(unspecified_matching(relay_addr))?);
        control.set_read_timeout(Some(timing.wait))?;
        let code = register_with_retry(&control, relay, fingerprint.clone(), timing)?;
        let bridge = HostBridge {
            control: control.clone(),
            relay: relay.to_string(),
            local_game_addr,
            fingerprint,
            bridges: Arc::new(Mutex::new(HashMap::new())),
            shared: Arc::new(HostShared { code: Mutex::new(code), changes: AtomicU32::new(0), relay_addr: Mutex::new(relay_addr) }),
        };
        bridge.spawn_keepalive(stop.clone(), keepalive);
        bridge.spawn_forwarding(stop);
        Ok((bridge, code))
    }

    /// The code the relay currently holds this host under. It is the one [`start`](Self::start) returned until the relay says otherwise: every
    /// keepalive is answered with the code, and a relay that was restarted (its table is empty) or a NAT that rebound (the host now has another
    /// address) hands out a new one, which this follows so the screen can show it.
    pub fn code(&self) -> super::relay::RelayCode {
        *self.shared.code.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// How many times [`code`](Self::code) has changed since registering (`0` = never).
    pub fn code_changes(&self) -> u32 {
        self.shared.changes.load(Ordering::Relaxed)
    }

    fn spawn_keepalive(&self, stop: Arc<AtomicBool>, interval: Duration) {
        let control = self.control.clone();
        let relay = self.relay.clone();
        let fingerprint = self.fingerprint.clone();
        let shared = self.shared.clone();
        std::thread::spawn(move || {
            let mut last = Instant::now();
            // Wakes often to look at `stop`: a host that has been told to stop must not send one more `Register` after its goodbye.
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(50).min(interval));
                if last.elapsed() < interval || stop.load(Ordering::Relaxed) {
                    continue;
                }
                last = Instant::now();
                if let Ok(relay_addr) = resolve_relay_within(&relay, DNS_BUDGET) {
                    *shared.relay_addr.lock().unwrap_or_else(|p| p.into_inner()) = relay_addr;
                    let _ = control.send_to(&RelayMessage::Register { fingerprint: fingerprint.clone() }.encode(), relay_addr);
                }
            }
        });
    }

    fn spawn_forwarding(&self, stop: Arc<AtomicBool>) {
        let control = self.control.clone();
        let local_game_addr = self.local_game_addr;
        let bridges = self.bridges.clone();
        let shared = self.shared.clone();
        std::thread::spawn(move || {
            control.set_read_timeout(Some(Duration::from_millis(500))).ok();
            let mut buf = [0u8; 1500];
            while !stop.load(Ordering::Relaxed) {
                match control.recv_from(&mut buf) {
                    Ok((n, from)) => forward_from_relay(&control, from, &bridges, local_game_addr, &buf[..n], &stop, &shared),
                    Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
                    // Windows reports the ICMP "port unreachable" a restarting relay causes as an error on the next read (`WSAECONNRESET`): the
                    // socket is fine and the relay is coming back, so this thread must not give up the host's only way to be reached.
                    Err(e) if is_bounced(&e) => std::thread::sleep(Duration::from_millis(50)),
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

impl Drop for HostBridge {
    /// Tells the relay this host is gone, so its code stops resolving now instead of lingering for the rest of its lease (a friend
    /// handed that code would otherwise wait out a connection to nobody). Best effort: a relay that cannot be reached, or one that
    /// predates `Unregister`, still forgets the code when the lease runs out.
    fn drop(&mut self) {
        if let Ok(relay_addr) = resolve_relay(&self.relay) {
            let _ = self.control.send_to(&RelayMessage::Unregister.encode(), relay_addr);
        }
    }
}

/// Resolves `relay` (`HOST:PORT`, a literal address or a hostname — a DuckDNS name, say) fresh, never cached:
/// re-resolving on every use is what makes a dynamic-DNS relay address actually behave like one, since the whole
/// point of such a name is that the IP behind it can change without anyone having to reconfigure anything.
fn resolve_relay(relay: &str) -> io::Result<SocketAddr> {
    use std::net::ToSocketAddrs;
    relay.to_socket_addrs()?.next().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("'{relay}' did not resolve to any address")))
}

/// How long a name lookup may take before it counts as failed. The system resolver can take far longer than this on a network that drops DNS, and a window that
/// waits that long looks frozen; it is not a limit on the relay's answer.
pub const DNS_BUDGET: Duration = Duration::from_secs(6);
/// How long a host waits for the relay's answer to one `Register` (it asks [`REGISTER_TRIES`] times).
pub const REGISTER_WAIT: Duration = Duration::from_secs(3);
/// How many times a host asks the relay for a code before it reports the relay as not answering.
pub const REGISTER_TRIES: usize = 3;

/// How patiently a host asks a relay for its first code: it sends `Register` up to `tries` times, waiting `wait` for each answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegisterTiming {
    /// How long one `Register` waits for the relay's answer.
    pub wait: Duration,
    /// How many `Register`s are sent before the relay counts as not answering.
    pub tries: usize,
}

impl Default for RegisterTiming {
    fn default() -> Self {
        RegisterTiming { wait: REGISTER_WAIT, tries: REGISTER_TRIES }
    }
}

/// [`resolve_relay`] that gives up after `budget`: the lookup runs on a thread of its own, which finishes (and is forgotten) whenever the system's
/// resolver does. A literal `IP:PORT` needs no lookup at all.
fn resolve_relay_within(relay: &str, budget: Duration) -> io::Result<SocketAddr> {
    if let Ok(addr) = relay.parse::<SocketAddr>() {
        return Ok(addr);
    }
    let (tx, rx) = mpsc::channel();
    let name = relay.to_string();
    std::thread::Builder::new().name("red-dns".into()).spawn(move || {
        let _ = tx.send(resolve_relay(&name));
    })?;
    rx.recv_timeout(budget)
        .unwrap_or_else(|_| Err(io::Error::new(io::ErrorKind::TimedOut, format!("the name lookup for '{relay}' took longer than {} s", budget.as_secs()))))
}

/// Whether `e` is the operating system telling a UDP socket that an earlier datagram of its bounced (an ICMP "port unreachable"): Windows raises
/// it on the next `recv_from` of an unconnected socket, Linux only on a connected one. It says the other end is not listening *now*, never that
/// this socket is broken.
fn is_bounced(e: &io::Error) -> bool {
    matches!(e.kind(), io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionRefused)
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
    /// The `Claim` datagram to send from the exact socket about to carry real traffic (`QuicClient::connect_claiming` sends it, and repeats it while
    /// the handshake runs: a repeat is harmless, see `quic::CLAIM_REPEATS_MS`).
    pub fn claim_bytes(&self) -> Vec<u8> {
        RelayMessage::Claim { token: self.token }.encode()
    }
}

/// How many times [`resolve_code`] asks within its time budget (the asks are spread out: see [`RESOLVE_SCHEDULE`]).
pub const RESOLVE_SENDS: usize = 4;
/// When each `Resolve` is sent, as a fraction of the time budget: the first at once, the others after the relay has had a fair chance to answer. A lost
/// datagram (a bad Wi-Fi moment, a busy router) then costs a fraction of a second, not the whole wait.
///
/// Asking again is safe: the relay answers every `Resolve` of a live code with a *fresh* claim token and keeps each for [`DEFAULT_PENDING_TIMEOUT`], so a
/// repeat only means one more unused token waiting to expire (the relay caps them, and a source may ask [`DEFAULT_MAX_REQUESTS_PER_SOURCE_PER_TICK`] times
/// a tick). The first answer wins; the later ones arrive at a socket nobody reads any more.
pub const RESOLVE_SCHEDULE: [f64; RESOLVE_SENDS] = [0.0, 0.12, 0.32, 0.6];

/// What a joining client does before ever touching `net::quic`: resolve a short code against a relay, then treat
/// the relay's own resolved address exactly like a normal server address for everything after this (the relay is
/// transparent to the QUIC handshake that follows).
///
/// `timeout` is how long the relay gets to answer, counted from the first `Resolve`; the datagram is sent [`RESOLVE_SENDS`] times inside it
/// ([`RESOLVE_SCHEDULE`]) because UDP is allowed to lose it. Looking the relay's name up has a budget of its own ([`DNS_BUDGET`]). A host that is not
/// listening at all (the relay is switched off) is told apart from a relay that is merely silent: the first bounces our datagram back, the second does not.
pub fn resolve_code(relay: &str, code: super::relay::RelayCode, timeout: Duration) -> Result<ResolvedHost, ResolveError> {
    let relay_addr = resolve_relay_within(relay, DNS_BUDGET).map_err(|e| ResolveError::RelayNotFound(relay_not_found(relay, &e)))?;
    // A connected socket: the system then drops anything that is not from the relay, and reports a bounced datagram instead of hiding it.
    let socket = UdpSocket::bind(unspecified_matching(relay_addr)).map_err(|e| ResolveError::Unreachable(cannot_send(&e)))?;
    socket.connect(relay_addr).map_err(|e| ResolveError::Unreachable(cannot_send(&e)))?;
    let ask = RelayMessage::Resolve { code }.encode();
    let (start, mut sent, mut bounced) = (Instant::now(), 0usize, 0u32);
    let deadline = start + timeout;
    let mut buf = [0u8; 256];
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Err(if bounced > 0 { ResolveError::RelayDown(relay.to_string()) } else { ResolveError::NoAnswer(relay.to_string()) });
        }
        while sent < RESOLVE_SENDS && now >= start + timeout.mul_f64(RESOLVE_SCHEDULE[sent]) {
            sent += 1;
            match socket.send(&ask) {
                Ok(_) => {}
                Err(e) if is_bounced(&e) => bounced += 1,
                Err(e) => return Err(ResolveError::Unreachable(cannot_send(&e))),
            }
        }
        if bounced >= 2 {
            return Err(ResolveError::RelayDown(relay.to_string()));
        }
        let next = if sent < RESOLVE_SENDS { start + timeout.mul_f64(RESOLVE_SCHEDULE[sent]) } else { deadline };
        let wait = next.min(deadline).saturating_duration_since(Instant::now()).max(Duration::from_millis(1));
        socket.set_read_timeout(Some(wait)).map_err(|e| ResolveError::Unreachable(e.to_string()))?;
        match socket.recv(&mut buf) {
            Ok(n) => match RelayMessage::decode(&buf[..n]) {
                Some(RelayMessage::Resolved { fingerprint, token }) => return Ok(ResolvedHost { relay_addr, fingerprint, token }),
                Some(RelayMessage::CodeNotFound) => return Err(ResolveError::CodeNotLive),
                // Something that is not an answer to us (a stray datagram to our port): not worth giving up the wait for.
                _ => {}
            },
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
            Err(e) if is_bounced(&e) => bounced += 1,
            Err(e) => return Err(ResolveError::Unreachable(cannot_send(&e))),
        }
    }
}

fn relay_not_found(relay: &str, why: &io::Error) -> String {
    let name = relay.rsplit_once(':').map_or(relay, |(n, _)| n);
    format!(
        "Cannot look up the relay's name, {name} ({why}). Check that you are online. Some networks block this name: try another network, or use the long join code."
    )
}

fn cannot_send(why: &io::Error) -> String {
    format!("This PC could not send to the relay ({why}). Check your network connection and that a firewall is not blocking the game.")
}

/// Why a code did not resolve. Kept apart so a screen can say the useful thing: a code nobody hosts any more (ask for a new one) is not
/// a relay that is down (try again, or use another network).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// The relay's name did not resolve to an address (carries the whole sentence for the screen).
    RelayNotFound(String),
    /// This machine could not send to the relay (carries the whole sentence for the screen).
    Unreachable(String),
    /// The relay did not answer in time (carries its `HOST:PORT`): it may be down, or the network drops UDP.
    NoAnswer(String),
    /// The relay's machine answered that nothing listens on the relay's port (carries its `HOST:PORT`): the relay is not running.
    RelayDown(String),
    /// The relay answered that no host holds this code: mistyped, never registered, or expired / left.
    CodeNotLive,
    /// The relay (or something pretending to be it) answered with something that is not a resolution.
    Unexpected(String),
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::RelayNotFound(m) | ResolveError::Unreachable(m) | ResolveError::Unexpected(m) => f.write_str(m),
            ResolveError::NoAnswer(relay) => write!(
                f,
                "The relay at {relay} did not answer after {RESOLVE_SENDS} tries. It may be down, or this network blocks UDP. Try again, try a phone hotspot, or ask for the long join code."
            ),
            ResolveError::RelayDown(relay) => write!(
                f,
                "The relay at {relay} is switched off: its machine answered that nothing is listening there. Ask whoever runs it to start it, or use the long join code, which needs no relay."
            ),
            ResolveError::CodeNotLive => f.write_str(
                "That code is not live: it is mistyped, or the host has left or restarted. Check it, or ask your friend for the code from their lobby again.",
            ),
        }
    }
}

impl std::error::Error for ResolveError {}

fn register_with_retry(control: &UdpSocket, relay: &str, fingerprint: Option<String>, timing: RegisterTiming) -> io::Result<[u8; super::relay::CODE_LEN]> {
    let mut buf = [0u8; 128];
    for _ in 0..timing.tries {
        let relay_addr = resolve_relay_within(relay, DNS_BUDGET)?;
        control.send_to(&RelayMessage::Register { fingerprint: fingerprint.clone() }.encode(), relay_addr)?;
        // Answers that are not this relay's `Registered` (a stray datagram) do not use up the try: the wait goes on until the socket's read timeout.
        let waited = Instant::now();
        while waited.elapsed() < timing.wait {
            match control.recv_from(&mut buf) {
                Ok((n, from)) if from == relay_addr => {
                    if let Some(RelayMessage::Registered { code }) = RelayMessage::decode(&buf[..n]) {
                        return Ok(code);
                    }
                }
                Ok(_) => {}
                Err(e) if is_bounced(&e) => break, // bounced: nothing is listening there; the next try may find it started
                Err(_) => break,
            }
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
    shared: &HostShared,
) {
    // `control` is the same socket `spawn_keepalive` repeats `Register` on, so the relay's own `Registered` reply
    // to that periodic re-registration arrives right here, on this very loop, every keepalive interval — not
    // just once at startup (the *first* `Registered` is already consumed synchronously inside
    // `register_with_retry`, before this loop even starts). Without this check, that reply's bytes would be
    // forwarded into the real local game server as if a brand new joiner had just sent them.
    match RelayMessage::decode(data) {
        // The keepalive's answer is also how a host learns its code changed: a relay that restarted has forgotten it and hands out a new one.
        // Only the relay's own address may say so (any host on the internet can send this socket a datagram).
        Some(RelayMessage::Registered { code }) => {
            if relay_addr == *shared.relay_addr.lock().unwrap_or_else(|p| p.into_inner()) {
                let mut current = shared.code.lock().unwrap_or_else(|p| p.into_inner());
                if *current != code {
                    *current = code;
                    shared.changes.fetch_add(1, Ordering::Relaxed);
                }
            }
            return;
        }
        Some(RelayMessage::CodeNotFound) => return,
        _ => {}
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
    // Woken twice a second to look at `stop`, and gives up on a joiner the game server has not answered for the relay's own patience.
    if local.set_read_timeout(Some(Duration::from_millis(500))).is_err() {
        return;
    }
    let _ = local.send_to(payload, local_game_addr);
    bridges.lock().unwrap_or_else(|p| p.into_inner()).insert(id, local.clone());
    let control = control.clone();
    let bridges = bridges.clone();
    let stop = stop.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 1500];
        let mut last_reply = Instant::now();
        while !stop.load(Ordering::Relaxed) {
            match local.recv_from(&mut buf) {
                Ok((n, src)) if src == local_game_addr => {
                    last_reply = Instant::now();
                    let framed = [&id.to_le_bytes()[..], &buf[..n]].concat();
                    let _ = control.send_to(&framed, relay_addr);
                }
                Ok(_) => {}
                // A bounced datagram (Windows reports the game server's ICMP "port unreachable" as an error here) is the same as silence: wait on.
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) || is_bounced(&e) => {
                    if last_reply.elapsed() > DEFAULT_PAIR_IDLE_TIMEOUT {
                        break;
                    }
                    if is_bounced(&e) {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                }
                Err(_) => break,
            }
        }
        bridges.lock().unwrap_or_else(|p| p.into_inner()).remove(&id);
    });
}

#[cfg(test)]
mod tests;
