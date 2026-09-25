//! `red_server` — the authoritative, headless Red multiplayer server.
//!
//! ```text
//! red_server [--map examples/test_lab.json] [--port 27015] [--bind 0.0.0.0]
//!            [--spawn-group NAME] [--demo-kick OBJECT_ID] [--snapshot-every 2]
//!            [--timeout-ms 3000] [--stats-secs 5] [--run-for SECS] [--record TRACE.json] [--record-every 6] [--no-interest]
//! ```
//!
//! It loads the map, simulates every player and every physics prop at 60 Hz, and sends 30 Hz
//! snapshots over UDP to whoever joins (`re2 --connect HOST:PORT`, or `red_bot`). No window, no GPU
//! is used. The first output line is `LISTENING <addr>` so scripts can find the port (`--port 0`
//! picks a free one).

use red_engine2::net::server::{raise_timer_resolution, Server, ServerConfig};
use red_engine2::net::{map_hash, DEFAULT_PORT};
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::spawns::parse_spawns;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn usage() -> ! {
    eprintln!(
        "usage: red_server [--map FILE] [--port N] [--bind IP] [--spawn-group NAME] [--demo-kick OBJECT_ID]\n                  [--snapshot-every N] [--timeout-ms N] [--stats-secs N] [--run-for SECS]
                  [--record TRACE.json] [--record-every N] [--no-interest]"
    );
    std::process::exit(2);
}

fn main() {
    let mut map = PathBuf::from("examples/test_lab.json");
    let (mut port, mut bind): (u16, IpAddr) = (DEFAULT_PORT, "0.0.0.0".parse().unwrap());
    let (mut group, mut kick, mut every, mut timeout_ms, mut stats_secs, mut run_for) = (String::new(), None::<String>, 2u8, 3000u64, 5u64, None::<f64>);
    let (mut record, mut record_every, mut no_interest) = (None::<PathBuf>, 6u32, false);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().unwrap_or_else(|| usage());
        match a.as_str() {
            "--map" => map = PathBuf::from(val()),
            "--port" => port = val().parse().unwrap_or_else(|_| usage()),
            "--bind" => bind = val().parse().unwrap_or_else(|_| usage()),
            "--spawn-group" => group = val(),
            "--demo-kick" => kick = Some(val()),
            "--snapshot-every" => every = val().parse().unwrap_or_else(|_| usage()),
            "--timeout-ms" => timeout_ms = val().parse().unwrap_or_else(|_| usage()),
            "--stats-secs" => stats_secs = val().parse().unwrap_or_else(|_| usage()),
            "--run-for" => run_for = Some(val().parse().unwrap_or_else(|_| usage())),
            "--no-interest" => no_interest = true,
            "--record" => record = Some(PathBuf::from(val())),
            "--record-every" => record_every = val().parse().unwrap_or_else(|_| usage()),
            _ => usage(),
        }
    }

    let text = std::fs::read_to_string(&map).unwrap_or_else(|e| {
        eprintln!("cannot read {}: {e}", map.display());
        std::process::exit(1);
    });
    let scene = red_engine2::schema::parse_scene(&text).unwrap_or_else(|errs| {
        eprintln!("{} is not a valid scene:\n  {}", map.display(), errs.join("\n  "));
        std::process::exit(1);
    });
    let mut spawns = parse_spawns(&text).unwrap_or_else(|e| {
        eprintln!("spawns: {e}");
        std::process::exit(1);
    });
    if !group.is_empty() {
        spawns.retain(|s| s.group == group);
        if spawns.is_empty() {
            eprintln!("no spawn points in group '{group}'");
            std::process::exit(1);
        }
    }
    let sim = MatchSim::try_new(&scene, spawns).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });
    let kick_prop = kick.as_ref().map(|id| {
        let obj = scene.objects.iter().position(|o| &o.id == id).unwrap_or_else(|| {
            eprintln!("--demo-kick: no object '{id}'");
            std::process::exit(1);
        });
        sim.props().prop_of_object(obj).unwrap_or_else(|| {
            eprintln!("--demo-kick: '{id}' is not a loose (movable) prop");
            std::process::exit(1);
        })
    });

    let mut cfg = ServerConfig::new(SocketAddr::new(bind, port), map_hash(&text));
    cfg.snapshot_every = every.max(1);
    cfg.client_timeout = Duration::from_millis(timeout_ms);
    cfg.stats_every = (stats_secs > 0).then(|| Duration::from_secs(stats_secs));
    let mut server = Server::bind(cfg, sim).unwrap_or_else(|e| {
        eprintln!("cannot listen on {bind}:{port}: {e}");
        std::process::exit(1);
    });
    if let Some(p) = kick_prop {
        server.set_demo_kick(p);
    }
    if !no_interest {
        match red_engine2::sim::interest::InterestMap::parse(&text) {
            Ok(Some(map)) => {
                println!("interest management: {} rooms, {} portal hop(s)", map.rooms.len(), map.hops);
                server.set_interest(Some(map));
            }
            Ok(None) => println!("interest management: off (the map has no zones)"),
            Err(e) => {
                eprintln!("interest: {e}");
                std::process::exit(1);
            }
        }
    }
    if record.is_some() {
        let mut header = red_engine2::sim::trace::Header::new(map_hash(&text), 0, &group, record_every, 60);
        header.scene = map.display().to_string();
        if let Err(e) = server.start_recording(header) {
            eprintln!("--record: {e}");
            std::process::exit(1);
        }
    }
    raise_timer_resolution();
    let local = server.local_addr().expect("bound");
    println!("LISTENING {local}");
    println!(
        "map {} ({} objects, hash {:08x}), {} loose props, 60 Hz tick, snapshots every {} ticks",
        map.display(),
        scene.objects.len(),
        map_hash(&text),
        server.sim().props().props().len(),
        every.max(1)
    );
    println!("join with:  re2 --connect 127.0.0.1:{} {}", local.port(), map.display());

    let stop = Arc::new(AtomicBool::new(false));
    if let Some(secs) = run_for {
        let s = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs_f64(secs));
            s.store(true, Ordering::Relaxed);
        });
    }
    {
        // Ctrl-C stops the server cleanly (clients get a Bye, a --record trace is written).
        let s = stop.clone();
        if let Err(e) = ctrlc::set_handler(move || s.store(true, Ordering::Relaxed)) {
            eprintln!("note: Ctrl-C will not stop the server gracefully ({e})");
        }
    }
    server.run(&stop);
    if let (Some(path), Some(trace)) = (&record, server.take_trace()) {
        match std::fs::write(path, serde_json::to_string(&trace.to_json()).unwrap_or_default()) {
            Ok(()) => println!(
                "recorded {} ticks ({} inputs) to {}: replay with `red_engine2 replay {}`",
                trace.final_tick,
                trace.entries.len(),
                path.display(),
                path.display()
            ),
            Err(e) => eprintln!("cannot write {}: {e}", path.display()),
        }
    }
    let s = server.stats();
    println!(
        "stopped: {} ticks, {} snapshots, {} joins, {} resumes, {} leaves ({} timeouts), {} bad packets",
        s.ticks, s.snapshots_sent, s.joins, s.resumes, s.leaves, s.timeouts, s.bad_packets
    );
}
