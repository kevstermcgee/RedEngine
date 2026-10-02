//! `red_relay` — a small, always-on rendezvous relay for players who cannot otherwise reach each other (a home
//! network behind carrier-grade NAT, where no UPnP mapping or port forward can ever be reached from outside;
//! see `docs/adr/0031-home-hosting-with-upnp.md`). A hosting `red_server`/`re2 --host --relay` registers and gets
//! a short code back; a joining player types that code instead of an address.
//!
//! ```text
//! red_relay [--port 28016] [--bind 0.0.0.0] [--stats-secs 30] [--run-for SECS]
//! ```
//!
//! It never touches QUIC/TLS content: it only ever forwards opaque datagrams between two addresses it has
//! paired by code (ADR 0044's end-to-end security between a host and its players is completely unchanged — the
//! relay sees ciphertext, exactly like any NAT box on the path already does). The actual socket work is
//! `net::relay_server::RelayServer` (tested in-process on loopback, without needing this binary at all); this is
//! the thin CLI/signal-handling shell around it, the same split `red_server` keeps with `net::server::Server`.
//!
//! SIGTERM (`docker stop`, systemd) and Ctrl-C both stop it cleanly.

use red_engine2::net::relay_server::{RelayServer, RelayServerOptions};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The relay's own default port (deliberately not [`red_engine2::net::DEFAULT_PORT`]: a relay and a game server
/// can run on the same machine at once).
const DEFAULT_RELAY_PORT: u16 = 28016;

fn usage() -> ! {
    eprintln!("usage: red_relay [--port N] [--bind IP] [--stats-secs N] [--run-for SECS]");
    std::process::exit(2);
}

fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

fn env<T: std::str::FromStr>(name: &str) -> Option<T> {
    let raw = std::env::var(name).ok().filter(|v| !v.is_empty())?;
    match raw.parse() {
        Ok(v) => Some(v),
        Err(_) => {
            eprintln!("{name}='{raw}' is not a valid value");
            std::process::exit(2);
        }
    }
}

fn main() {
    let mut port: u16 = env("RED_RELAY_PORT").unwrap_or(DEFAULT_RELAY_PORT);
    // Unlike `red_server`, a relay is reachable from anywhere by design: there is no "loopback by default" to opt out of.
    let mut bind_ip: IpAddr = env("RED_RELAY_BIND").unwrap_or(IpAddr::from([0, 0, 0, 0]));
    let mut stats_secs: u64 = env("RED_RELAY_STATS_SECS").unwrap_or(30);
    let mut run_for: Option<f64> = env("RED_RELAY_RUN_FOR");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().unwrap_or_else(|| usage());
        match a.as_str() {
            "--port" => port = val().parse().unwrap_or_else(|_| usage()),
            "--bind" => bind_ip = val().parse().unwrap_or_else(|_| usage()),
            "--stats-secs" => stats_secs = val().parse().unwrap_or_else(|_| usage()),
            "--run-for" => run_for = Some(val().parse().unwrap_or_else(|_| usage())),
            "-h" | "--help" => usage(),
            _ => usage(),
        }
    }

    let options = RelayServerOptions { bind: SocketAddr::new(bind_ip, port), ..Default::default() };
    let relay = match RelayServer::bind(options) {
        Ok(r) => Arc::new(r),
        Err(e) => fail(&format!("could not bind {bind_ip}:{port}: {e}")),
    };
    let local = relay.local_addr();
    println!("LISTENING {local}");
    println!("a host registers with --relay {local}; a friend joins with the short code it is given instead of an address");

    let stop = Arc::new(AtomicBool::new(false));
    if let Some(secs) = run_for {
        let s = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs_f64(secs));
            s.store(true, Ordering::Relaxed);
        });
    }
    {
        let s = stop.clone();
        if let Err(e) = ctrlc::set_handler(move || s.store(true, Ordering::Relaxed)) {
            eprintln!("note: Ctrl-C will not stop the relay gracefully ({e})");
        }
    }

    // The networking loop runs on its own thread so the main thread is free to print periodic stats by polling
    // the relay's own counters (`net::relay_server` keeps the server itself single-purpose: it never prints).
    let relay_thread = {
        let relay = relay.clone();
        let stop = stop.clone();
        std::thread::spawn(move || relay.run(&stop))
    };
    let mut last_stats = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_secs(1));
        if last_stats.elapsed() >= Duration::from_secs(stats_secs) {
            last_stats = Instant::now();
            println!("stats: {} host(s) registered, {} client(s) relaying", relay.registered_count(), relay.paired_count());
        }
    }
    let _ = relay_thread.join();
    println!("relay stopping");
}
