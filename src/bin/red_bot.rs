//! `red_bot` — a headless scripted Red client. Real networking, prediction and interpolation, no
//! window: it joins a server, walks a script, and prints JSON lines describing what it sees (other
//! players, props) so multiplayer can be verified from a separate process.
//!
//! ```text
//! red_bot --server 127.0.0.1:27015 [--map examples/test_lab.json] [--as human|rat]
//!         [--behavior idle | forward:YAW | circle:DEG_PER_SEC | route:x,z;x,z;...] [--sprint]
//!         [--duration 5] [--report-every 0.5] [--token N] [--leave-after SECS] [--rejoin-after SECS]
//! ```
//!
//! Output (stdout), one JSON object per line: `{"event":...}` for connects and disconnects,
//! `{"report":...}` periodically, and a final `{"summary":...}`.

use red_engine2::net::bot::{Behavior, Bot, BotFrame, ClientWorld};
use red_engine2::player::Character;
use serde_json::json;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn usage() -> ! {
    eprintln!("usage: red_bot --server HOST:PORT [--map FILE] [--as human|rat] [--behavior idle|forward:YAW|circle:DPS|route:x,z;x,z] [--sprint] [--duration S] [--report-every S] [--token N] [--leave-after S] [--rejoin-after S]");
    std::process::exit(2);
}

fn behavior(spec: &str, sprint: bool) -> Behavior {
    match spec.split_once(':') {
        None if spec == "idle" => Behavior::Idle,
        Some(("forward", yaw)) => Behavior::Forward { yaw_deg: yaw.parse().unwrap_or_else(|_| usage()), sprint },
        Some(("circle", dps)) => Behavior::Circle { turn_deg_per_sec: dps.parse().unwrap_or_else(|_| usage()) },
        Some(("route", pts)) => Behavior::Waypoints {
            points: pts
                .split(';')
                .map(|p| {
                    let (x, z) = p.split_once(',').unwrap_or_else(|| usage());
                    (x.trim().parse().unwrap_or_else(|_| usage()), z.trim().parse().unwrap_or_else(|_| usage()))
                })
                .collect(),
            sprint,
        },
        _ => usage(),
    }
}

fn report(f: &BotFrame) -> serde_json::Value {
    json!({"report": {
        "t": (f.t * 100.0).round() / 100.0,
        "conn": format!("{:?}", f.conn),
        "me": f.me_state.map(|s| [s.pos.x, s.pos.y]),
        "remote": f.remote.iter().map(|(id, p)| json!({"id": id, "x": p.pos.x, "z": p.pos.z, "speed": p.speed})).collect::<Vec<_>>(),
        "props": f.props.iter().map(|(id, p)| json!({"id": id, "x": p.pos.x, "y": p.pos.y, "z": p.pos.z})).collect::<Vec<_>>(),
        "rtt_ms": f.rtt_ms,
    }})
}

fn main() {
    let (mut server, mut map, mut who, mut spec, mut sprint) =
        (None::<SocketAddr>, PathBuf::from("examples/test_lab.json"), Character::Human, "idle".to_string(), false);
    let (mut duration, mut every, mut token, mut leave_after, mut rejoin_after) = (5.0f64, 0.5f64, 0u64, None::<f64>, None::<f64>);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().unwrap_or_else(|| usage());
        match a.as_str() {
            "--server" => server = Some(val().to_socket_addrs().ok().and_then(|mut i| i.next()).unwrap_or_else(|| usage())),
            "--map" => map = PathBuf::from(val()),
            "--as" => who = Character::parse(&val()).unwrap_or_else(|| usage()),
            "--behavior" => spec = val(),
            "--sprint" => sprint = true,
            "--duration" => duration = val().parse().unwrap_or_else(|_| usage()),
            "--report-every" => every = val().parse().unwrap_or_else(|_| usage()),
            "--token" => token = val().parse().unwrap_or_else(|_| usage()),
            "--leave-after" => leave_after = Some(val().parse().unwrap_or_else(|_| usage())),
            "--rejoin-after" => rejoin_after = Some(val().parse().unwrap_or_else(|_| usage())),
            _ => usage(),
        }
    }
    let server = server.unwrap_or_else(|| usage());
    let mk = |token: u64| {
        let (_scene, world) = ClientWorld::load(&map).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(1);
        });
        Bot::new(server, who, world, behavior(&spec, sprint), token).unwrap_or_else(|e| {
            eprintln!("cannot open a socket: {e}");
            std::process::exit(1);
        })
    };
    let mut bot = mk(token);
    let started = Instant::now();
    let mut next_report = 0.0;
    let mut last_event = 0usize;
    let (mut frames, mut max_remote, mut worst_rtt) = (0u64, 0usize, 0.0f32);
    let mut left = false;
    let mut rejoined = false;
    loop {
        let t = started.elapsed().as_secs_f64();
        if t >= duration {
            break;
        }
        if !left && leave_after.is_some_and(|s| t >= s) {
            left = true;
            token = bot.client.token();
            bot.client.disconnect();
            println!("{}", json!({"event": {"t": t, "what": "left (Bye sent)", "token": token}}));
            if rejoin_after.is_none() {
                break;
            }
        }
        if left && !rejoined {
            if rejoin_after.is_some_and(|s| t >= s) {
                rejoined = true;
                bot = mk(token);
                last_event = 0; // a fresh Bot has a fresh event list
                println!("{}", json!({"event": {"t": t, "what": "rejoining with token", "token": token}}));
            } else {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
        }
        let now = Instant::now();
        bot.pump(now);
        let f = bot.frame(now);
        frames += 1;
        max_remote = max_remote.max(f.remote.len());
        worst_rtt = worst_rtt.max(f.rtt_ms);
        while last_event < bot.events.len() {
            let (et, what) = &bot.events[last_event];
            println!("{}", json!({"event": {"t": et, "what": what}}));
            last_event += 1;
        }
        if t >= next_report {
            println!("{}", report(&f));
            next_report = t + every;
        }
        std::thread::sleep(Duration::from_millis(3));
    }
    let f = bot.frame(Instant::now());
    let stats = bot.client.stats().clone();
    println!(
        "{}",
        json!({"summary": {
            "id": bot.client.my_id(),
            "token": bot.client.token(),
            "conn": format!("{:?}", f.conn),
            "me": f.me_state.map(|s| [s.pos.x, s.pos.y]),
            "remote": f.remote.iter().map(|(id, p)| json!({"id": id, "x": p.pos.x, "z": p.pos.z})).collect::<Vec<_>>(),
            "props": f.props.iter().map(|(id, p)| json!({"id": id, "x": p.pos.x, "y": p.pos.y, "z": p.pos.z})).collect::<Vec<_>>(),
            "max_remote_players_seen": max_remote,
            "frames": frames,
            "snapshots": stats.snapshots,
            "snapshots_missed": stats.snapshots_missed,
            "connects": stats.connects,
            "rtt_ms": stats.rtt_ms,
            "worst_rtt_ms": worst_rtt,
            "worst_prediction_correction_m": bot.predictor.as_ref().map_or(0.0, |p| p.worst_correction),
        }})
    );
    bot.client.disconnect();
}
