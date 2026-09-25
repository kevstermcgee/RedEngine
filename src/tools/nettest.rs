//! `red_engine2 net-test`: what does the game feel like on a bad connection? A real server and real clients (prediction, interpolation,
//! reconnect and all) run in-process with a seeded, bursty-lossy, laggy UDP proxy between them (`net::netsim`), and the run is judged on
//! what a player would notice: did anyone get disconnected, did the local player end up where the server says, did other players
//! glide or teleport on screen, how large were the corrections, how much bandwidth did it take.
//!
//! It is deterministic in the network (same seed, same losses) but not in wall-clock timing, so the verdicts have margin built in.

use crate::net::bot::{Behavior, Bot, BotFrame, ClientWorld};
use crate::net::client::ConnState;
use crate::net::netsim::{LinkProfile, LossyProxy, ProxyReport};
use crate::net::server::{Server, ServerConfig};
use crate::player::Character;
use crate::sim::match_sim::{MatchSim, MAX_PLAYERS};
use crate::sim::spawns::parse_spawns;
use glam::Vec2;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// What to run.
#[derive(Debug, Clone)]
pub struct Options {
    /// Link conditions to try, one run each.
    pub profiles: Vec<LinkProfile>,
    /// Players (each behind its own proxy), 1..=8.
    pub players: usize,
    /// Seconds of play per profile (plus a short settle).
    pub secs: f64,
    /// Seed for the proxies' randomness.
    pub seed: u64,
}

/// One pass/fail line of a report.
#[derive(Debug, Clone)]
pub struct Check {
    /// What was checked.
    pub name: String,
    /// Whether it held.
    pub ok: bool,
    /// The numbers.
    pub detail: String,
}

/// What one client experienced.
#[derive(Debug, Clone)]
pub struct ClientReport {
    /// Player id.
    pub id: u8,
    /// Smoothed round-trip time, ms.
    pub rtt_ms: f32,
    /// Snapshots received.
    pub snapshots: u64,
    /// Snapshots that never arrived (lost or reordered away).
    pub missed: u64,
    /// Times it (re)connected (1 = it never lost the server).
    pub connects: u32,
    /// Largest prediction correction, metres.
    pub worst_correction_m: f32,
    /// Distance between where it thinks it is and where the server says, after settling, metres.
    pub final_error_m: f32,
    /// Largest movement of another player's drawn position beyond what the catch-up speed allows for the time elapsed, metres (0 = perfectly smooth).
    pub worst_remote_step_m: f32,
    /// Whether it ended connected.
    pub connected: bool,
}

/// The result for one link profile.
#[derive(Debug, Clone)]
pub struct ProfileReport {
    /// The link.
    pub profile: LinkProfile,
    /// Per client.
    pub clients: Vec<ClientReport>,
    /// What the proxies did, summed over all clients.
    pub proxy: ProxyReport,
    /// Server bytes sent per client per second.
    pub server_bytes_per_client_sec: f64,
    /// The verdicts.
    pub checks: Vec<Check>,
}

impl ProfileReport {
    /// Whether every check held.
    pub fn ok(&self) -> bool {
        self.checks.iter().all(|c| c.ok)
    }
}

/// Runs every profile in turn.
pub fn run(scene_path: &Path, opts: &Options) -> Result<Vec<ProfileReport>, String> {
    if !(1..=MAX_PLAYERS).contains(&opts.players) {
        return Err(format!("--players must be 1..={MAX_PLAYERS}"));
    }
    let text = std::fs::read_to_string(scene_path).map_err(|e| format!("{}: {e}", scene_path.display()))?;
    let mut out = Vec::new();
    for (i, p) in opts.profiles.iter().enumerate() {
        out.push(run_one(scene_path, &text, *p, opts, opts.seed.wrapping_add(i as u64 * 7919))?);
    }
    Ok(out)
}

fn run_one(scene_path: &Path, text: &str, profile: LinkProfile, opts: &Options, seed: u64) -> Result<ProfileReport, String> {
    let scene = crate::schema::parse_scene(text).map_err(|e| e.join("; "))?;
    let spawns = parse_spawns(text)?;
    let sim = MatchSim::try_new(&scene, spawns)?;
    let mut cfg = ServerConfig::new("127.0.0.1:0".parse().map_err(|e| format!("{e}"))?, crate::net::map_hash(text));
    cfg.client_timeout = Duration::from_millis(3000);
    let mut server = Server::bind(cfg, sim).map_err(|e| format!("cannot bind: {e}"))?;
    server.set_logger(|_| {});
    let server_addr = SocketAddr::new("127.0.0.1".parse().map_err(|e| format!("{e}"))?, server.local_addr().map_err(|e| e.to_string())?.port());
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let server_thread = std::thread::spawn(move || {
        server.run(&stop2);
        server
    });

    let mut proxies = Vec::new();
    let mut bots: Vec<Bot> = Vec::new();
    let mut failure = None;
    for k in 0..opts.players {
        let proxy = LossyProxy::start(server_addr, profile, seed.wrapping_add(k as u64 * 104_729)).map_err(|e| e.to_string())?;
        let (_scene, world) = ClientWorld::load(scene_path)?;
        let who = if k % 2 == 0 { Character::Human } else { Character::Rat };
        let mut bot = Bot::new(proxy.addr, who, world, Behavior::Circle { turn_deg_per_sec: 40.0 + 15.0 * k as f32 }, 0).map_err(|e| e.to_string())?;
        // Join one at a time so spawn order is stable and a slow handshake is attributed to its link.
        bot.run(Duration::from_secs(8), |f| f.conn != ConnState::Connected || f.me_state.is_none());
        if bot.client.state() != ConnState::Connected {
            failure = Some(format!("player {k} could not join through the '{}' link within 8 s", profile.name));
        }
        proxies.push(proxy);
        bots.push(bot);
        if failure.is_some() {
            break;
        }
    }

    let mut frames: Vec<Vec<BotFrame>> = (0..bots.len()).map(|_| Vec::new()).collect();
    if failure.is_none() {
        std::thread::scope(|s| {
            let handles: Vec<_> = bots
                .iter_mut()
                .zip(frames.iter_mut())
                .map(|(bot, out)| {
                    s.spawn(move || {
                        let mut last = -1.0;
                        bot.run(Duration::from_secs_f64(opts.secs), |f| {
                            if f.t - last >= 0.010 {
                                out.push(f.clone());
                                last = f.t;
                            }
                            true
                        });
                        // Stand still so prediction and the server can agree, then compare.
                        bot.behavior = Behavior::Idle;
                        bot.run(Duration::from_millis(1800), |_| true);
                    })
                })
                .collect();
            for h in handles {
                let _ = h.join();
            }
        });
    }

    let ids: Vec<Option<u8>> = bots.iter().map(|b| b.client.my_id()).collect();
    let stats: Vec<_> = bots.iter().map(|b| b.client.stats().clone()).collect();
    let states: Vec<ConnState> = bots.iter().map(|b| b.client.state()).collect();
    let predicted: Vec<Option<Vec2>> = bots.iter().map(|b| b.predictor.as_ref().map(|p| p.state.pos)).collect();
    let worst_corrections: Vec<f32> = bots.iter().map(|b| b.predictor.as_ref().map_or(0.0, |p| p.worst_correction)).collect();
    drop(bots);
    let proxy_report = proxies.into_iter().map(LossyProxy::finish).fold(ProxyReport::default(), |mut a, r| {
        for d in 0..2 {
            a.received[d] += r.received[d];
            a.dropped[d] += r.dropped[d];
            a.duplicated[d] += r.duplicated[d];
        }
        a
    });
    stop.store(true, Ordering::Relaxed);
    let server = server_thread.join().map_err(|_| "the server thread panicked".to_string())?;
    if let Some(f) = failure {
        return Err(f);
    }
    let secs_total = opts.secs + 1.8;
    let server_bytes_per_client_sec = server.stats().bytes_out as f64 / secs_total / opts.players as f64;

    let clients: Vec<ClientReport> = (0..ids.len())
        .map(|k| {
            let id = ids[k].unwrap_or(0);
            let truth = server.sim().player(id as usize).map(|p| p.state.pos);
            let final_error = match (predicted[k], truth) {
                (Some(a), Some(b)) => a.distance(b),
                _ => f32::INFINITY,
            };
            // How far any other player's drawn position moved between two samples of the same player, *beyond what CATCH_UP_SPEED allows in the
            // time that really passed between them*. Judging by elapsed time (not "per 10 ms") means a stalled test thread on a loaded machine is
            // not mistaken for a teleport; a snap of a metre shows as a metre.
            let mut worst_step = 0.0f32;
            let mut last: std::collections::HashMap<u8, (f64, Vec2)> = std::collections::HashMap::new();
            for f in &frames[k] {
                for (rid, p) in &f.remote {
                    let now = Vec2::new(p.pos.x, p.pos.z);
                    if let Some((t0, prev)) = last.insert(*rid, (f.t, now)) {
                        let allowed = crate::net::interp::CATCH_UP_SPEED * (f.t - t0).max(0.0) as f32;
                        worst_step = worst_step.max((prev.distance(now) - allowed).max(0.0));
                    }
                }
            }
            ClientReport {
                id,
                rtt_ms: stats[k].rtt_ms,
                snapshots: stats[k].snapshots,
                missed: stats[k].snapshots_missed,
                connects: stats[k].connects,
                worst_correction_m: worst_corrections[k],
                final_error_m: final_error,
                worst_remote_step_m: worst_step,
                connected: states[k] == ConnState::Connected,
            }
        })
        .collect();

    // ---- the verdict, in terms a player would notice ----
    let mut checks = Vec::new();
    let mut add = |name: &str, ok: bool, detail: String| checks.push(Check { name: name.to_string(), ok, detail });
    add(
        "nobody was disconnected",
        clients.iter().all(|c| c.connected && c.connects == 1),
        format!("connects per client: {:?}", clients.iter().map(|c| c.connects).collect::<Vec<_>>()),
    );
    let worst_final = clients.iter().map(|c| c.final_error_m).fold(0.0f32, f32::max);
    add("prediction ends on the server's position", worst_final <= 0.10, format!("worst final error {worst_final:.3} m (limit 0.10)"));
    let worst_corr = clients.iter().map(|c| c.worst_correction_m).fold(0.0f32, f32::max);
    add("corrections stay small", worst_corr <= 0.6, format!("worst correction {worst_corr:.3} m (limit 0.6)"));
    if opts.players > 1 {
        let worst_step = clients.iter().map(|c| c.worst_remote_step_m).fold(0.0f32, f32::max);
        add(
            "other players glide, they do not teleport",
            worst_step <= 0.15,
            format!("worst drawn jump {worst_step:.3} m beyond {} m/s (limit 0.15)", crate::net::interp::CATCH_UP_SPEED),
        );
    }
    add("bandwidth stays modest", server_bytes_per_client_sec <= 24_000.0, format!("{server_bytes_per_client_sec:.0} B/s per client (limit 24000)"));
    if profile.loss > 0.0 {
        add(
            "the simulated link really was lossy",
            proxy_report.dropped[0] + proxy_report.dropped[1] > 0,
            format!(
                "{} of {} datagrams dropped ({:.1}%)",
                proxy_report.dropped[0] + proxy_report.dropped[1],
                proxy_report.received[0] + proxy_report.received[1],
                proxy_report.loss_fraction() * 100.0
            ),
        );
    }
    Ok(ProfileReport { profile, clients, proxy: proxy_report, server_bytes_per_client_sec, checks })
}

/// The reports as JSON.
pub fn to_json(reports: &[ProfileReport]) -> Value {
    json!({
        "ok": reports.iter().all(ProfileReport::ok),
        "profiles": reports.iter().map(|r| json!({
            "profile": r.profile.name,
            "link": {"loss": r.profile.loss, "burst": r.profile.burst, "delay_ms": r.profile.delay_ms, "jitter_ms": r.profile.jitter_ms, "duplicate": r.profile.duplicate},
            "ok": r.ok(),
            "server_bytes_per_client_sec": r.server_bytes_per_client_sec.round(),
            "proxy": {"received": r.proxy.received, "dropped": r.proxy.dropped, "duplicated": r.proxy.duplicated},
            "clients": r.clients.iter().map(|c| json!({
                "id": c.id, "rtt_ms": c.rtt_ms.round(), "snapshots": c.snapshots, "missed": c.missed, "connects": c.connects,
                "worst_correction_m": c.worst_correction_m, "final_error_m": c.final_error_m, "worst_remote_step_m": c.worst_remote_step_m, "connected": c.connected,
            })).collect::<Vec<_>>(),
            "checks": r.checks.iter().map(|c| json!({"name": c.name, "ok": c.ok, "detail": c.detail})).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

/// The reports as text.
pub fn render(reports: &[ProfileReport]) -> String {
    let mut s = String::new();
    for r in reports {
        let p = r.profile;
        s.push_str(&format!(
            "{} link '{}': {:.0}% loss in bursts of {:.1}, {:.0} ms +- {:.0} ms each way{}\n",
            if r.ok() { "PASS" } else { "FAIL" },
            p.name,
            p.loss * 100.0,
            p.burst,
            p.delay_ms,
            p.jitter_ms,
            if p.duplicate > 0.0 { format!(", {:.0}% duplicated", p.duplicate * 100.0) } else { String::new() }
        ));
        for c in &r.clients {
            let seen = if c.snapshots + c.missed > 0 { c.missed as f64 / (c.snapshots + c.missed) as f64 * 100.0 } else { 0.0 };
            s.push_str(&format!(
                "    player {}: rtt {:.0} ms, {} snapshots ({:.1}% never arrived), worst correction {:.3} m, final error {:.3} m, worst remote jump {:.3} m\n",
                c.id, c.rtt_ms, c.snapshots, seen, c.worst_correction_m, c.final_error_m, c.worst_remote_step_m
            ));
        }
        for c in &r.checks {
            s.push_str(&format!("    {} {}: {}\n", if c.ok { "ok  " } else { "FAIL" }, c.name, c.detail));
        }
    }
    s
}
