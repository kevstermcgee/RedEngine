//! A bad-network simulator: a UDP proxy that sits between a client and the server and makes the link *bursty-lossy, laggy, jittery and
//! duplicating*, the same on every run (it is seeded). `red_engine2 net-test` puts real clients and a real server on either side of it and
//! reports what the player would feel; `tests/net_e2e.rs` uses it too.
//!
//! Loss comes in bursts (a two-state Gilbert-Elliott model), because real Wi-Fi and mobile links drop several datagrams in a row, which
//! is what defeats a scheme that only survives isolated losses. `loss` is the long-run fraction dropped; `burst` is how many datagrams a
//! loss run lasts on average (1 = independent losses).

use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// A named set of link conditions, applied in each direction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkProfile {
    /// Its name (`wifi`, `4g`, `bad`, ...).
    pub name: &'static str,
    /// Long-run fraction of datagrams dropped, `0.0..1.0`.
    pub loss: f64,
    /// Mean length of a run of consecutive losses, in datagrams (`>= 1`; `1` = independent).
    pub burst: f64,
    /// One-way delay, milliseconds.
    pub delay_ms: f64,
    /// Delay varies uniformly by up to +- this, milliseconds (so datagrams can arrive out of order).
    pub jitter_ms: f64,
    /// Fraction of datagrams delivered twice.
    pub duplicate: f64,
}

/// The built-in profiles, best link first.
pub const PROFILES: &[LinkProfile] = &[
    LinkProfile { name: "lan", loss: 0.0, burst: 1.0, delay_ms: 0.5, jitter_ms: 0.3, duplicate: 0.0 },
    LinkProfile { name: "wifi", loss: 0.01, burst: 1.5, delay_ms: 6.0, jitter_ms: 4.0, duplicate: 0.0 },
    LinkProfile { name: "4g", loss: 0.02, burst: 2.0, delay_ms: 25.0, jitter_ms: 15.0, duplicate: 0.0 },
    // 50 ms one way (100 ms round trip) and 5% loss in bursts, with the odd duplicate: the conditions a game should survive.
    LinkProfile { name: "bad", loss: 0.05, burst: 3.0, delay_ms: 50.0, jitter_ms: 25.0, duplicate: 0.01 },
    LinkProfile { name: "awful", loss: 0.15, burst: 4.0, delay_ms: 90.0, jitter_ms: 50.0, duplicate: 0.02 },
];

/// The profile called `name`.
pub fn profile(name: &str) -> Option<LinkProfile> {
    PROFILES.iter().find(|p| p.name == name).copied()
}

/// Datagram counts, per direction, of one proxy.
#[derive(Debug, Default)]
pub struct ProxyCounters {
    /// Datagrams the proxy received going to the server / going to the client.
    pub received: [AtomicU64; 2],
    /// Of those, dropped.
    pub dropped: [AtomicU64; 2],
    /// Extra copies delivered.
    pub duplicated: [AtomicU64; 2],
}

/// What a finished proxy saw. Index 0 is client to server, 1 is server to client.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ProxyReport {
    /// Datagrams received.
    pub received: [u64; 2],
    /// Datagrams dropped.
    pub dropped: [u64; 2],
    /// Datagrams delivered a second time.
    pub duplicated: [u64; 2],
}

impl ProxyReport {
    /// The fraction dropped, both directions together.
    pub fn loss_fraction(&self) -> f64 {
        let (r, d) = (self.received[0] + self.received[1], self.dropped[0] + self.dropped[1]);
        if r == 0 {
            0.0
        } else {
            d as f64 / r as f64
        }
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// One direction's Gilbert-Elliott loss state.
struct Loss {
    bad: bool,
    p_enter: f64,
    p_leave: f64,
}

impl Loss {
    fn new(profile: &LinkProfile) -> Loss {
        let p_leave = 1.0 / profile.burst.max(1.0);
        let loss = profile.loss.clamp(0.0, 0.95);
        // Stationary loss = p_enter / (p_enter + p_leave)  =>  p_enter = loss * p_leave / (1 - loss).
        Loss { bad: false, p_enter: loss * p_leave / (1.0 - loss), p_leave }
    }

    fn drops(&mut self, rng: &mut Rng) -> bool {
        let flip = rng.next();
        self.bad = if self.bad { flip >= self.p_leave } else { flip < self.p_enter };
        self.bad
    }
}

/// A running proxy. The client talks to [`LossyProxy::addr`]; the server sees the proxy's other socket as that client.
pub struct LossyProxy {
    /// Where the client should send its datagrams.
    pub addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    counters: Arc<ProxyCounters>,
}

impl LossyProxy {
    /// Starts a proxy to `server` with `profile`, drawing its randomness from `seed`.
    pub fn start(server: SocketAddr, profile: LinkProfile, seed: u64) -> io::Result<LossyProxy> {
        let front = UdpSocket::bind("127.0.0.1:0")?;
        let back = UdpSocket::bind("127.0.0.1:0")?;
        front.set_nonblocking(true)?;
        back.set_nonblocking(true)?;
        let addr = front.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let counters = Arc::new(ProxyCounters::default());
        let (stop2, counters2) = (stop.clone(), counters.clone());
        let handle = std::thread::spawn(move || {
            let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
            let mut loss = [Loss::new(&profile), Loss::new(&profile)];
            let mut client: Option<SocketAddr> = None;
            let mut queue: Vec<(Instant, usize, Vec<u8>)> = Vec::new();
            let mut buf = [0u8; 2048];
            while !stop2.load(Ordering::Relaxed) {
                for dir in 0..2 {
                    let sock = if dir == 0 { &front } else { &back };
                    while let Ok((n, from)) = sock.recv_from(&mut buf) {
                        if dir == 0 {
                            client = Some(from);
                        }
                        counters2.received[dir].fetch_add(1, Ordering::Relaxed);
                        if loss[dir].drops(&mut rng) {
                            counters2.dropped[dir].fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                        let copies = if rng.next() < profile.duplicate { 2 } else { 1 };
                        if copies == 2 {
                            counters2.duplicated[dir].fetch_add(1, Ordering::Relaxed);
                        }
                        for _ in 0..copies {
                            let d = (profile.delay_ms + (rng.next() - 0.5) * 2.0 * profile.jitter_ms).max(0.0);
                            queue.push((Instant::now() + Duration::from_micros((d * 1000.0) as u64), dir, buf[..n].to_vec()));
                        }
                    }
                }
                let now = Instant::now();
                let mut i = 0;
                while i < queue.len() {
                    if queue[i].0 <= now {
                        let (_, dir, bytes) = queue.swap_remove(i);
                        if dir == 0 {
                            let _ = back.send_to(&bytes, server);
                        } else if let Some(c) = client {
                            let _ = front.send_to(&bytes, c);
                        }
                    } else {
                        i += 1;
                    }
                }
                std::thread::sleep(Duration::from_micros(400));
            }
        });
        Ok(LossyProxy { addr, stop, handle: Some(handle), counters })
    }

    /// Stops the proxy and returns what it saw.
    pub fn finish(mut self) -> ProxyReport {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        let get = |a: &[AtomicU64; 2]| [a[0].load(Ordering::Relaxed), a[1].load(Ordering::Relaxed)];
        ProxyReport { received: get(&self.counters.received), dropped: get(&self.counters.dropped), duplicated: get(&self.counters.duplicated) }
    }
}

impl Drop for LossyProxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_burst_model_hits_its_average_loss_and_its_run_length() {
        for (loss, burst) in [(0.05, 1.0), (0.05, 3.0), (0.15, 4.0), (0.0, 1.0)] {
            let p = LinkProfile { name: "t", loss, burst, delay_ms: 0.0, jitter_ms: 0.0, duplicate: 0.0 };
            let (mut l, mut rng) = (Loss::new(&p), Rng(12345));
            let n = 400_000;
            let (mut dropped, mut runs, mut prev) = (0u32, 0u32, false);
            for _ in 0..n {
                let d = l.drops(&mut rng);
                dropped += d as u32;
                runs += (d && !prev) as u32;
                prev = d;
            }
            let frac = dropped as f64 / n as f64;
            assert!((frac - loss).abs() < 0.01, "loss {loss} burst {burst}: measured {frac}");
            if runs > 0 {
                let mean_run = dropped as f64 / runs as f64;
                assert!((mean_run - burst).abs() < burst * 0.25, "loss {loss}: mean run {mean_run}, wanted about {burst}");
            }
        }
    }

    #[test]
    fn profiles_are_ordered_from_kind_to_cruel_and_findable() {
        assert!(PROFILES.windows(2).all(|w| w[0].loss <= w[1].loss && w[0].delay_ms <= w[1].delay_ms));
        assert_eq!(profile("bad").map(|p| p.loss), Some(0.05));
        assert_eq!(profile("nonsense"), None);
    }

    #[test]
    fn a_proxy_forwards_delays_drops_and_counts() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        server.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
        let p = LinkProfile { name: "t", loss: 0.5, burst: 1.0, delay_ms: 30.0, jitter_ms: 0.0, duplicate: 0.0 };
        let proxy = LossyProxy::start(server.local_addr().unwrap(), p, 7).unwrap();
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        let t0 = Instant::now();
        for i in 0..200u8 {
            client.send_to(&[i], proxy.addr).unwrap();
        }
        let mut got = 0;
        let mut first_at = None;
        let mut buf = [0u8; 16];
        while server.recv_from(&mut buf).is_ok() {
            first_at.get_or_insert(t0.elapsed());
            got += 1;
        }
        let report = proxy.finish();
        assert_eq!(report.received[0], 200);
        assert!(report.dropped[0] > 60 && report.dropped[0] < 140, "about half dropped: {}", report.dropped[0]);
        assert_eq!(got as u64, 200 - report.dropped[0], "everything not dropped was delivered");
        assert!(first_at.is_some_and(|t| t >= Duration::from_millis(25)), "the delay was applied: {first_at:?}");
    }
}
