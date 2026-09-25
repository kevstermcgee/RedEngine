//! `red_server` — the authoritative, headless Red multiplayer server.
//!
//! ```text
//! red_server [--map examples/test_lab.json] [--port 27015] [--bind 0.0.0.0]
//!            [--spawn-group NAME] [--demo-kick OBJECT_ID] [--snapshot-every 2]
//!            [--timeout-ms 3000] [--stats-secs 5] [--run-for SECS] [--record TRACE.json] [--record-every 6] [--no-interest]
//!            [--key JOIN_KEY|auto] [--lobby] [--upnp] [--min-players N] [--countdown-secs S] [--round-secs S] [--results-secs S] [--score-to-win N]
//! ```
//!
//! `--upnp` opens the UDP port on the home router (UPnP, see `red_engine2 portmap`), renews it while the server runs and removes it on exit.
//! `--key` makes joining need a key: clients prove they know it without sending it, and every datagram is authenticated (ADR 0028).
//! `--key auto` invents a random 128-bit key and prints it. `--lobby` turns on the match flow (lobby, ready-up, countdown, rounds,
//! results, rematch; ADR 0029) with default settings; a map's own `"match"` block turns it on with the map's settings; the `--min-players`,
//! `--countdown-secs`, `--round-secs`, `--results-secs` and `--score-to-win` flags (or `RED_MIN_PLAYERS` ...) override either and also turn
//! it on. With `--record`,
//! each round of a match flow is written to `TRACE.roundN.json`.
//!
//! It loads the map, simulates every player and every physics prop at 60 Hz, and sends 30 Hz
//! snapshots over UDP to whoever joins (`re2 --connect HOST:PORT`, or `red_bot`). No window, no GPU
//! is used. The first output line is `LISTENING <addr>` so scripts can find the port (`--port 0`
//! picks a free one).
//!
//! Every setting can also come from the environment (a flag wins over a variable), which is what containers and
//! process managers want: `RED_MAP`, `RED_PORT`, `RED_BIND`, `RED_SPAWN_GROUP`, `RED_SNAPSHOT_EVERY`, `RED_TIMEOUT_MS`,
//! `RED_STATS_SECS`, `RED_RUN_FOR`, `RED_KEY`, `RED_LOBBY` (1 = on), `RED_MIN_PLAYERS`, `RED_COUNTDOWN_SECS`, `RED_ROUND_SECS`, `RED_RESULTS_SECS`, `RED_SCORE_TO_WIN`. SIGTERM (`docker stop`, systemd) and Ctrl-C both stop it cleanly.

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
                  [--record TRACE.json] [--record-every N] [--no-interest] [--key K|auto] [--lobby] [--upnp]
                  [--min-players N] [--countdown-secs S] [--round-secs S] [--results-secs S] [--score-to-win N]"
    );
    std::process::exit(2);
}

/// A setting from the environment, parsed; a set-but-malformed value is an error rather than silently ignored.
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
    let mut map = env::<PathBuf>("RED_MAP").unwrap_or_else(|| PathBuf::from("examples/test_lab.json"));
    let (mut port, mut bind): (u16, IpAddr) = (env("RED_PORT").unwrap_or(DEFAULT_PORT), env("RED_BIND").unwrap_or_else(|| "0.0.0.0".parse().unwrap()));
    let (mut group, mut kick, mut every, mut timeout_ms, mut stats_secs, mut run_for) = (
        env::<String>("RED_SPAWN_GROUP").unwrap_or_default(),
        None::<String>,
        env("RED_SNAPSHOT_EVERY").unwrap_or(2u8),
        env("RED_TIMEOUT_MS").unwrap_or(3000u64),
        env("RED_STATS_SECS").unwrap_or(5u64),
        env::<f64>("RED_RUN_FOR"),
    );
    let (mut record, mut record_every, mut no_interest) = (None::<PathBuf>, 6u32, false);
    let (mut key, mut lobby) = (env::<String>("RED_KEY"), env::<u8>("RED_LOBBY").unwrap_or(0) != 0);
    let mut upnp = env::<u8>("RED_UPNP").unwrap_or(0) != 0;
    let (mut ov_min, mut ov_count, mut ov_round, mut ov_results, mut ov_score) = (
        env::<u8>("RED_MIN_PLAYERS"),
        env::<f32>("RED_COUNTDOWN_SECS"),
        env::<f32>("RED_ROUND_SECS"),
        env::<f32>("RED_RESULTS_SECS"),
        env::<u32>("RED_SCORE_TO_WIN"),
    );
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
            "--key" => key = Some(val()),
            "--lobby" => lobby = true,
            "--upnp" => upnp = true,
            "--min-players" => ov_min = Some(val().parse().unwrap_or_else(|_| usage())),
            "--countdown-secs" => ov_count = Some(val().parse().unwrap_or_else(|_| usage())),
            "--round-secs" => ov_round = Some(val().parse().unwrap_or_else(|_| usage())),
            "--results-secs" => ov_results = Some(val().parse().unwrap_or_else(|_| usage())),
            "--score-to-win" => ov_score = Some(val().parse().unwrap_or_else(|_| usage())),
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
    let sim = MatchSim::try_new(&scene, spawns.clone()).unwrap_or_else(|e| {
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

    let overridden = ov_min.is_some() || ov_count.is_some() || ov_round.is_some() || ov_results.is_some() || ov_score.is_some();
    let flow_settings = match red_engine2::sim::flow::MatchSettings::from_scene_text(&text) {
        Ok(Some(s)) => Some(s),
        Ok(None) => (lobby || overridden).then(red_engine2::sim::flow::MatchSettings::default),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
    .map(|mut s| {
        s.min_players = ov_min.unwrap_or(s.min_players);
        s.countdown_secs = ov_count.unwrap_or(s.countdown_secs);
        s.round_secs = ov_round.unwrap_or(s.round_secs);
        s.results_secs = ov_results.unwrap_or(s.results_secs);
        s.score_to_win = ov_score.unwrap_or(s.score_to_win);
        s
    });
    let mut cfg = ServerConfig::new(SocketAddr::new(bind, port), map_hash(&text));
    if key.as_deref() == Some("auto") {
        let entropy = std::collections::hash_map::RandomState::new();
        let mut n = 0u64;
        key = Some(red_engine2::net::auth::random_key(|| {
            n += 1;
            std::hash::BuildHasher::hash_one(&entropy, n)
        }));
        println!("join key (generated): {}", key.as_deref().unwrap_or(""));
    }
    cfg.join_key = key.clone().filter(|k| !k.is_empty());
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
    if let Some(settings) = flow_settings {
        // The scene is parsed afresh for every round: a rematch starts from the authored map (Scene is not Clone, and parsing is milliseconds).
        let (text2, spawns2) = (text.clone(), spawns.clone());
        let rebuild = move || {
            let scene = red_engine2::schema::parse_scene(&text2).map_err(|e| e.join("; "))?;
            MatchSim::try_new(&scene, spawns2.clone())
        };
        if let Err(e) = server.enable_flow(settings.clone(), rebuild) {
            eprintln!("match flow: {e}");
            std::process::exit(1);
        }
        println!(
            "match flow: on (min {} player(s), countdown {} s, round {} s, results {} s, score to win {}, join in progress {})",
            settings.min_players,
            settings.countdown_secs,
            if settings.round_secs > 0.0 { format!("{}", settings.round_secs) } else { "unlimited".to_string() },
            settings.results_secs,
            settings.score_to_win,
            settings.join_in_progress
        );
        if let Some(base) = &record {
            let mut header = red_engine2::sim::trace::Header::new(map_hash(&text), 0, &group, record_every, 60);
            header.scene = map.display().to_string();
            server.set_round_recording(header);
            let base = base.clone();
            server.set_round_hook(move |r| {
                let Some(trace) = r.trace else { return };
                let stem = base.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "trace".to_string());
                let path = base.with_file_name(format!("{stem}.round{}.json", r.round));
                match std::fs::write(&path, serde_json::to_string(&trace.to_json()).unwrap_or_default()) {
                    Ok(()) => println!("round {} recorded to {}: replay with `red_engine2 replay {}`", r.round, path.display(), path.display()),
                    Err(e) => eprintln!("cannot write {}: {e}", path.display()),
                }
            });
        }
    } else if record.is_some() {
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
    // UPnP: map the port on the home router, keep it alive from a small thread, remove it when the server stops.
    let upnp_thread = upnp.then(|| {
        let opts = red_engine2::tools::portmap::Options { port: local.port(), ..Default::default() };
        match red_engine2::tools::portmap::Keeper::start(&opts) {
            Ok((mut keeper, report)) => {
                print!("{report}");
                let s = stop.clone();
                Some(std::thread::spawn(move || {
                    while !s.load(Ordering::Relaxed) {
                        if let Some(e) = keeper.tick() {
                            eprintln!("{e}");
                        }
                        std::thread::sleep(Duration::from_millis(500));
                    }
                    println!("{}", keeper.stop());
                }))
            }
            Err(e) => {
                eprintln!("--upnp: {e}");
                eprintln!("the server keeps running on the local network; see docs/HOSTING.md for other ways to be reachable");
                None
            }
        }
    });
    if let Some(secs) = run_for {
        let s = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs_f64(secs));
            s.store(true, Ordering::Relaxed);
        });
    }
    {
        // Ctrl-C and SIGTERM (docker stop, systemd) stop the server cleanly: clients get a Bye, a --record trace is written.
        let s = stop.clone();
        if let Err(e) = ctrlc::set_handler(move || s.store(true, Ordering::Relaxed)) {
            eprintln!("note: Ctrl-C will not stop the server gracefully ({e})");
        }
    }
    server.run(&stop);
    if let Some(Some(t)) = upnp_thread {
        let _ = t.join(); // removes the router mapping before the process ends
    }
    let open_play = server.phase() == red_engine2::sim::flow::Phase::Playing && server.round() == 0;
    if let (true, Some(path), Some(trace)) = (open_play, &record, server.take_trace()) {
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
