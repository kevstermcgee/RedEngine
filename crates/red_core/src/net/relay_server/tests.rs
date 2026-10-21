//! Tests of the relay's socket I/O: the relay itself, the host's bridge to it, and a joining client resolving and claiming through it.

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
    let shared = HostShared { code: Mutex::new(generate_code().unwrap()), changes: AtomicU32::new(0), relay_addr: Mutex::new(relay_addr) };

    let ack = RelayMessage::Registered { code: generate_code().unwrap() }.encode();
    forward_from_relay(&control, relay_addr, &bridges, game_addr, &ack, &stop, &shared);
    assert!(bridges.lock().unwrap().is_empty(), "a Registered ack must never open a bridge");

    let mut buf = [0u8; 64];
    assert!(game_server.recv_from(&mut buf).is_err(), "the game server must never see the ack's bytes");

    // A real id-framed joiner's packet still works normally.
    let framed = [&1u32.to_le_bytes()[..], b"a real packet"].concat();
    forward_from_relay(&control, relay_addr, &bridges, game_addr, &framed, &stop, &shared);
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
        assert_eq!(err, ResolveError::CodeNotLive);
        assert!(err.to_string().contains("not live"), "{err}");
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
fn a_hosts_keepalive_keeps_its_code_live_past_the_lease_and_silence_does_not() {
    // The keepalive `Register` comes from the registered host's own address, which the relay used to read as framed joiner traffic and
    // drop: every code died at the lease however long the host stayed up. A short lease makes that visible in a fraction of a second.
    let relay = RelayServer::bind(RelayServerOptions { registration_lease: Duration::from_millis(400), ..fast_options() }).unwrap();
    let relay_addr = relay.local_addr().to_string();
    with_relay_running(&relay, || {
        let game = loopback_game();
        let stop = Arc::new(AtomicBool::new(false));
        let _stop_guard = StopOnDrop(&stop);
        let (_alive, kept) = HostBridge::start_with_keepalive(&relay_addr, game, None, stop.clone(), Duration::from_millis(100)).unwrap();
        let (_silent, forgotten) = HostBridge::start_with_keepalive(&relay_addr, game, None, stop.clone(), Duration::from_secs(3600)).unwrap();
        std::thread::sleep(Duration::from_millis(1200)); // three leases
        assert!(super::resolve_code(&relay_addr, kept, Duration::from_secs(5)).is_ok(), "a host that keeps asking keeps its code");
        assert_eq!(
            super::resolve_code(&relay_addr, forgotten, Duration::from_secs(5)).unwrap_err(),
            ResolveError::CodeNotLive,
            "a host that went quiet loses it when the lease runs out"
        );
    });
}

#[test]
fn a_host_that_leaves_frees_its_code_and_its_pairings_at_once() {
    let relay = RelayServer::bind(fast_options()).unwrap();
    let relay_addr = relay.local_addr();
    with_relay_running(&relay, || {
        let host = UdpSocket::bind(loopback()).unwrap();
        host.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let _stop_guard = StopOnDrop(&stop);
        let (bridge, code) = HostBridge::start(&relay_addr.to_string(), host.local_addr().unwrap(), None, stop.clone()).unwrap();
        let resolved = super::resolve_code(&relay_addr.to_string(), code, Duration::from_secs(5)).unwrap();
        let joiner = UdpSocket::bind(loopback()).unwrap();
        joiner.send_to(&resolved.claim_bytes(), relay_addr).unwrap();
        joiner.send_to(b"hello", relay_addr).unwrap();
        let mut buf = [0u8; 64];
        host.recv_from(&mut buf).unwrap();
        assert_eq!((relay.registered_count(), relay.paired_count()), (1, 1));

        drop(bridge); // the host leaves: its goodbye reaches the relay
        wait("the code to stop resolving", || {
            matches!(super::resolve_code(&relay_addr.to_string(), code, Duration::from_secs(5)), Err(ResolveError::CodeNotLive)).then_some(())
        });
        wait("the pairing to be torn down", || (relay.registered_count() == 0 && relay.paired_count() == 0).then_some(()));
    });
}

#[test]
fn only_the_registered_address_can_unregister_a_host() {
    let relay = RelayServer::bind(fast_options()).unwrap();
    let relay_addr = relay.local_addr();
    with_relay_running(&relay, || {
        let stop = Arc::new(AtomicBool::new(false));
        let _stop_guard = StopOnDrop(&stop);
        let (_bridge, code) = HostBridge::start(&relay_addr.to_string(), loopback_game(), None, stop.clone()).unwrap();
        let stranger = UdpSocket::bind(loopback()).unwrap();
        stranger.send_to(&RelayMessage::Unregister.encode(), relay_addr).unwrap();
        std::thread::sleep(Duration::from_millis(150));
        assert!(super::resolve_code(&relay_addr.to_string(), code, Duration::from_secs(5)).is_ok(), "someone else's goodbye changes nothing");
    });
}

fn loopback_game() -> SocketAddr {
    // Where a game server would be; the tests above only need an address to bridge to.
    SocketAddr::from(([127, 0, 0, 1], 9))
}

// Needs the QUIC transport (red_core's `quic`; the engine always has it).
#[cfg(feature = "quic")]
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

// Needs the QUIC transport (red_core's `quic`; the engine always has it).
#[cfg(feature = "quic")]
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

// ---- a join that survives a lost packet, a relay that is not there, and a relay that restarts ----

/// A relay that is only a socket: it records every datagram it gets and answers (through `answer`) the ones it is told to.
struct ScriptedRelay {
    addr: SocketAddr,
    seen: mpsc::Receiver<Vec<Vec<u8>>>,
}

impl ScriptedRelay {
    /// Listens until it has seen `until` datagrams, answering the last of them with `reply` (no answer when `None`), or until five seconds pass.
    fn start(until: usize, reply: Option<RelayMessage>) -> ScriptedRelay {
        let socket = UdpSocket::bind(loopback()).unwrap();
        socket.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
        let addr = socket.local_addr().unwrap();
        let (tx, seen) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut got, mut buf) = (Vec::new(), [0u8; 128]);
            let end = Instant::now() + Duration::from_secs(5);
            while Instant::now() < end && got.len() < until {
                if let Ok((n, from)) = socket.recv_from(&mut buf) {
                    got.push(buf[..n].to_vec());
                    if got.len() == until {
                        if let Some(r) = &reply {
                            let _ = socket.send_to(&r.encode(), from);
                        }
                    }
                }
            }
            let _ = tx.send(got);
        });
        ScriptedRelay { addr, seen }
    }

    fn seen(&self) -> Vec<Vec<u8>> {
        self.seen.recv_timeout(Duration::from_secs(8)).unwrap()
    }
}

#[test]
fn resolve_asks_again_when_a_datagram_is_lost_and_takes_the_first_answer() {
    // A relay that does not answer the first two questions (they were lost) and answers the third: one lost datagram costs a fraction of a second,
    // not the whole wait, and every question is the same seven bytes the deployed relay has always been asked.
    let code = generate_code().unwrap();
    let relay = ScriptedRelay::start(3, Some(RelayMessage::CodeNotFound));
    let started = Instant::now();
    let err = super::resolve_code(&relay.addr.to_string(), code, Duration::from_secs(2)).unwrap_err();
    assert_eq!(err, ResolveError::CodeNotLive, "the third question was answered");
    assert!(started.elapsed() < Duration::from_millis(1500), "answered at the third try, not after the whole 2 s wait: {:?}", started.elapsed());
    let asked = relay.seen();
    assert_eq!(asked.len(), 3);
    assert!(asked.iter().all(|d| *d == RelayMessage::Resolve { code }.encode() && d.len() == 7 && d[0] == 3), "{asked:?}");
}

#[test]
fn a_silent_relay_is_asked_every_try_inside_the_time_and_then_called_silent() {
    let code = generate_code().unwrap();
    let relay = ScriptedRelay::start(usize::MAX, None);
    let started = Instant::now();
    let err = super::resolve_code(&relay.addr.to_string(), code, Duration::from_millis(800)).unwrap_err();
    let waited = started.elapsed();
    assert_eq!(err, ResolveError::NoAnswer(relay.addr.to_string()));
    assert!(waited >= Duration::from_millis(790) && waited < Duration::from_millis(1500), "{waited:?}");
    let text = err.to_string();
    assert!(text.contains("did not answer") && text.contains("blocks UDP") && text.contains("long join code"), "the cause and what to do next: {text}");
    drop(relay); // stops listening once the five seconds are up; nothing waits for it
}

#[test]
fn a_relay_whose_port_is_closed_is_called_switched_off_and_quickly() {
    // The machine is there and says nothing listens on the port: that is "the relay is down", not "your network drops UDP".
    let closed = {
        let socket = UdpSocket::bind(loopback()).unwrap();
        socket.local_addr().unwrap()
    };
    let started = Instant::now();
    let err = super::resolve_code(&closed.to_string(), generate_code().unwrap(), Duration::from_secs(5)).unwrap_err();
    assert_eq!(err, ResolveError::RelayDown(closed.to_string()));
    assert!(started.elapsed() < Duration::from_secs(2), "no waiting out the five seconds for a refusal: {:?}", started.elapsed());
    let text = err.to_string();
    assert!(text.contains("switched off") && text.contains("Ask whoever runs it") && text.contains("long join code"), "{text}");
}

#[test]
fn a_name_that_does_not_resolve_is_a_lookup_failure_that_names_the_name() {
    let err = super::resolve_code("relay.invalid:28016", generate_code().unwrap(), Duration::from_secs(1)).unwrap_err();
    let ResolveError::RelayNotFound(text) = &err else { panic!("{err:?}") };
    assert!(text.contains("relay.invalid") && text.contains("Check that you are online") && text.contains("another network"), "{text}");
}

#[test]
fn a_dead_code_is_told_so_with_what_to_do() {
    let relay = RelayServer::bind(fast_options()).unwrap();
    let addr = relay.local_addr().to_string();
    with_relay_running(&relay, || {
        let err = super::resolve_code(&addr, generate_code().unwrap(), Duration::from_secs(5)).unwrap_err();
        assert_eq!(err, ResolveError::CodeNotLive);
        let text = err.to_string();
        assert!(text.contains("not live") && text.contains("host has left or restarted") && text.contains("ask your friend"), "{text}");
    });
}

#[test]
fn a_relay_asked_twice_gives_two_working_claims_and_one_pairing() {
    // Why asking again is safe: every Resolve of a live code mints its own token, and a token is used once however many Claims name it. The
    // second Claim of a pair finds nothing to consume; it reaches the host (an address already paired is forwarded) as bytes no game server reads.
    let relay = RelayServer::bind(fast_options()).unwrap();
    let addr = relay.local_addr();
    with_relay_running(&relay, || {
        let game = UdpSocket::bind(loopback()).unwrap();
        game.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let _guard = StopOnDrop(&stop);
        let (_bridge, code) = HostBridge::start(&addr.to_string(), game.local_addr().unwrap(), None, stop.clone()).unwrap();
        let first = super::resolve_code(&addr.to_string(), code, Duration::from_secs(5)).unwrap();
        let second = super::resolve_code(&addr.to_string(), code, Duration::from_secs(5)).unwrap();
        assert_ne!(first.token, second.token, "each question gets a token of its own");
        let client = UdpSocket::bind(loopback()).unwrap();
        for _ in 0..3 {
            client.send_to(&first.claim_bytes(), addr).unwrap();
        }
        client.send_to(b"real", addr).unwrap();
        wait("the pairing", || (relay.paired_count() == 1).then_some(()));
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(relay.paired_count(), 1, "three copies of one claim are one pairing");
        // The host's game port got the id-framed claim copies and then the real bytes, each as the datagram it was sent as.
        let mut seen = Vec::new();
        let mut buf = [0u8; 64];
        while let Ok((n, _)) = game.recv_from(&mut buf) {
            seen.push(buf[..n].to_vec());
        }
        assert_eq!(seen.last().map(Vec::as_slice), Some(&b"real"[..]), "{seen:?}");
        assert!(seen.iter().filter(|d| d.len() == 9 && d[0] == 6).count() == 2, "the two repeats arrive as opaque 9-byte datagrams: {seen:?}");
    });
}

#[test]
fn the_hosts_code_follows_the_relay_when_it_is_restarted() {
    // A relay that restarts has forgotten every host and hands the next keepalive a new code. The host must show the new one: the old one resolves to
    // nobody. (The relay is a socket on a fixed port here, so the second one can take the first one's place.)
    let first = Arc::new(RelayServer::bind(fast_options()).unwrap());
    let addr = first.local_addr();
    let stop_first = Arc::new(AtomicBool::new(false));
    let first_thread = {
        let (relay, stop) = (first.clone(), stop_first.clone());
        std::thread::spawn(move || relay.run(&stop))
    };
    let stop = Arc::new(AtomicBool::new(false));
    let _guard = StopOnDrop(&stop);
    let (bridge, original) = HostBridge::start_with_keepalive(&addr.to_string(), loopback_game(), None, stop.clone(), Duration::from_millis(100)).unwrap();
    assert_eq!((bridge.code(), bridge.code_changes()), (original, 0));

    stop_first.store(true, Ordering::Relaxed);
    first_thread.join().unwrap();
    drop(first);
    let second = RelayServer::bind(RelayServerOptions { bind: addr, ..fast_options() }).unwrap();
    with_relay_running(&second, || {
        let renewed = wait("the host to learn its new code", || (bridge.code() != original).then(|| bridge.code()));
        assert_eq!(bridge.code_changes(), 1);
        assert!(super::resolve_code(&addr.to_string(), renewed, Duration::from_secs(5)).is_ok(), "the new code is live on the new relay");
        assert_eq!(super::resolve_code(&addr.to_string(), original, Duration::from_secs(5)).unwrap_err(), ResolveError::CodeNotLive, "the old one is not");
        // Asked again, the same relay says the same code: no further change.
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(bridge.code_changes(), 1);
    });
}

#[test]
fn only_the_relays_own_address_can_change_the_code_a_host_shows() {
    let control = Arc::new(UdpSocket::bind(loopback()).unwrap());
    let game_addr = loopback_game();
    let bridges = Arc::new(Mutex::new(HashMap::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let (relay, stranger) = (SocketAddr::from(([127, 0, 0, 1], 28016)), SocketAddr::from(([127, 0, 0, 1], 4000)));
    let original = generate_code().unwrap();
    let shared = HostShared { code: Mutex::new(original), changes: AtomicU32::new(0), relay_addr: Mutex::new(relay) };
    let announce =
        |from: SocketAddr, code| forward_from_relay(&control, from, &bridges, game_addr, &RelayMessage::Registered { code }.encode(), &stop, &shared);

    let other = loop {
        let c = generate_code().unwrap();
        if c != original {
            break c;
        }
    };
    announce(stranger, other);
    assert_eq!((*shared.code.lock().unwrap(), shared.changes.load(Ordering::Relaxed)), (original, 0), "a datagram from anywhere else is not the relay");
    announce(relay, original);
    assert_eq!(shared.changes.load(Ordering::Relaxed), 0, "the same code again is not a change");
    announce(relay, other);
    assert_eq!((*shared.code.lock().unwrap(), shared.changes.load(Ordering::Relaxed)), (other, 1));
    assert!(bridges.lock().unwrap().is_empty(), "an announcement never opens a bridge to the game");
}

#[test]
fn a_joiners_envelope_that_looks_like_registered_never_changes_the_code_a_host_shows() {
    // Every joiner's datagram reaches the host from the relay's own address, framed as `[id: u32 LE][payload]`. Pairing 2 with a three-byte payload is
    // seven bytes that start with `Registered`'s tag: anyone who knows the six characters (not the key) could make the host drop its online code. A
    // relay's code is always six characters of its alphabet; this one is the id's zero bytes and the payload.
    let control = Arc::new(UdpSocket::bind(loopback()).unwrap());
    let game_server = UdpSocket::bind(loopback()).unwrap();
    game_server.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    let game_addr = game_server.local_addr().unwrap();
    let bridges = Arc::new(Mutex::new(HashMap::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let relay = SocketAddr::from(([127, 0, 0, 1], 28016));
    let original = generate_code().unwrap();
    let shared = HostShared { code: Mutex::new(original), changes: AtomicU32::new(0), relay_addr: Mutex::new(relay) };
    let forged = [&2u32.to_le_bytes()[..], b"ABC"].concat();
    assert!(matches!(RelayMessage::decode(&forged), Some(RelayMessage::Registered { .. })), "the shape this test is about");
    forward_from_relay(&control, relay, &bridges, game_addr, &forged, &stop, &shared);
    assert_eq!((*shared.code.lock().unwrap(), shared.changes.load(Ordering::Relaxed)), (original, 0), "the code the host shows is unchanged");
    // A code with a byte outside the alphabet (lower case, a digit the alphabet leaves out) is no code either.
    for bad in [*b"abcdef", *b"H3PQX0", *b"H3PQX\0"] {
        forward_from_relay(&control, relay, &bridges, game_addr, &RelayMessage::Registered { code: bad }.encode(), &stop, &shared);
        assert_eq!(shared.changes.load(Ordering::Relaxed), 0, "{bad:?}");
    }
    assert!(bridges.lock().unwrap().is_empty());
    let mut buf = [0u8; 64];
    assert!(game_server.recv_from(&mut buf).is_err(), "dropped, as before: the game never sees three bytes it could not use");
    // The relay's real answer still gets through.
    let renewed = loop {
        let c = generate_code().unwrap();
        if c != original {
            break c;
        }
    };
    forward_from_relay(&control, relay, &bridges, game_addr, &RelayMessage::Registered { code: renewed }.encode(), &stop, &shared);
    assert_eq!((*shared.code.lock().unwrap(), shared.changes.load(Ordering::Relaxed)), (renewed, 1));
}

#[test]
fn a_signal_interrupting_a_read_is_a_pause_not_the_end_of_the_hosts_link() {
    // On Linux a process that is stopped and continued (Ctrl-Z then `fg`, a debugger, `kill -STOP`/`-CONT`) gets `EINTR` from every socket read
    // that has a timeout (signal(7)). The host's forwarding thread used to end on it, leaving a code the relay still resolved and no host behind it:
    // every joiner then got "the host did not answer". Reproduced on a throwaway server built from f7d78d3.
    for kind in [io::ErrorKind::Interrupted, io::ErrorKind::WouldBlock, io::ErrorKind::TimedOut] {
        assert!(read_paused(&io::Error::from(kind)), "{kind:?} is a pause: the socket reads on");
    }
    for kind in [io::ErrorKind::NotConnected, io::ErrorKind::InvalidInput, io::ErrorKind::ConnectionReset] {
        assert!(!read_paused(&io::Error::from(kind)), "{kind:?} is not a timeout (a bounce has its own arm)");
    }
}

#[test]
fn a_host_that_cannot_reach_the_relay_says_so_after_its_tries_and_does_not_hang() {
    let silent = UdpSocket::bind(loopback()).unwrap();
    let started = Instant::now();
    let timing = RegisterTiming { wait: Duration::from_millis(150), tries: 2 };
    let err = HostBridge::start_with(
        &silent.local_addr().unwrap().to_string(),
        loopback_game(),
        None,
        Arc::new(AtomicBool::new(false)),
        DEFAULT_HOST_KEEPALIVE,
        timing,
    )
    .err()
    .expect("nobody answers");
    assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    assert!(started.elapsed() >= Duration::from_millis(290) && started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
}
