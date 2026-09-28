//! Hosting a match from inside another program (ADR 0054): how `re2 --host MAP` plays against bots on your own machine with no second process,
//! no console window to close and nothing left running afterwards. It is the core of `red_server` for one map: the same [`Server`], the match flow
//! and the bots that the map's own `match` and `bots` blocks ask for, listening on the loopback by default, on a thread that stops when the
//! [`LocalHost`] is dropped.

use super::map_hash;
use super::server::{raise_timer_resolution, Server, ServerConfig};
use crate::sim::flow::MatchSettings;
use crate::sim::interest::InterestMap;
use crate::sim::match_sim::MatchSim;
use crate::sim::spawns::parse_spawns;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

/// What to host.
#[derive(Debug, Clone)]
pub struct HostOptions {
    /// The address to listen on: the loopback unless other machines should be able to join.
    pub bind: IpAddr,
    /// The UDP port (`0` picks a free one).
    pub port: u16,
    /// Players (humans included) the match aims for; empty slots are filled with bots. `None` = the map's `bots.fill`, `Some(0)` = no bots.
    pub fill: Option<usize>,
    /// The bots' level, overriding the map's: `rookie`, `easy`, `normal`, `hard`, `nightmare`, or a number from 0 to 1.
    pub bot_skill: Option<String>,
    /// Only use spawn points of this group (`""` = all).
    pub spawn_group: String,
}

impl Default for HostOptions {
    fn default() -> Self {
        HostOptions { bind: IpAddr::from([127, 0, 0, 1]), port: 0, fill: None, bot_skill: None, spawn_group: String::new() }
    }
}

/// A server running on a background thread of this process.
pub struct LocalHost {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl LocalHost {
    /// Loads `map` and starts serving it. The error says what is wrong with the map or the options.
    pub fn start(map: &Path, opts: &HostOptions) -> Result<LocalHost, String> {
        let text = std::fs::read_to_string(map).map_err(|e| format!("cannot read {}: {e}", map.display()))?;
        let scene = crate::schema::parse_scene(&text).map_err(|e| format!("{} is not a valid scene: {}", map.display(), e.join("; ")))?;
        let mut spawns = parse_spawns(&text)?;
        if !opts.spawn_group.is_empty() {
            spawns.retain(|s| s.group == opts.spawn_group);
            if spawns.is_empty() {
                return Err(format!("no spawn points in group '{}'", opts.spawn_group));
            }
        }
        let sim = MatchSim::try_new(&scene, spawns.clone())?;
        let mut cfg = ServerConfig::new(SocketAddr::new(opts.bind, opts.port), map_hash(&text));
        cfg.bot_fill = opts.fill;
        cfg.bot_level = opts
            .bot_skill
            .as_deref()
            .map(|s| {
                crate::sim::ai::skill::level_from_name(s)
                    .ok_or_else(|| format!("'{s}' is not a bot level (rookie, easy, normal, hard, nightmare, or a number from 0 to 1)"))
            })
            .transpose()?;
        let mut server = Server::bind(cfg, sim).map_err(|e| format!("cannot listen on {}:{}: {e}", opts.bind, opts.port))?;
        if let Some(interest) = InterestMap::parse(&text)? {
            server.set_interest(Some(interest));
        }
        if let Some(settings) = MatchSettings::from_scene_text(&text)? {
            // The scene is parsed afresh for every round: a rematch starts from the authored map.
            let (text2, spawns2) = (text.clone(), spawns);
            server.enable_flow(settings, move || {
                let scene = crate::schema::parse_scene(&text2).map_err(|e| e.join("; "))?;
                MatchSim::try_new(&scene, spawns2.clone())
            })?;
        }
        let bound = server.local_addr().map_err(|e| format!("cannot read the server address: {e}"))?;
        // A server bound to every interface is still reached from this machine by the loopback.
        let addr = SocketAddr::new(if opts.bind.is_unspecified() { IpAddr::from([127, 0, 0, 1]) } else { opts.bind }, bound.port());
        raise_timer_resolution();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread =
            std::thread::Builder::new().name("red-host".into()).spawn(move || server.run(&flag)).map_err(|e| format!("cannot start the server thread: {e}"))?;
        Ok(LocalHost { addr, stop, thread: Some(thread) })
    }

    /// The address a client on this machine joins.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

impl Drop for LocalHost {
    /// Stops the server (its clients are told) and waits for its thread.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            // The server stops within a tick or two; if it somehow cannot (a blocked write), give up on it rather than hold the program open.
            for _ in 0..300 {
                if t.is_finished() {
                    let _ = t.join();
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::bot::{Behavior, Bot, ClientWorld};
    use crate::net::protocol::ROSTER_BOT;
    use crate::player::Character;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    fn lab() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json")
    }

    #[test]
    fn a_local_host_serves_the_map_fills_it_with_bots_and_stops_when_dropped() {
        let host = LocalHost::start(&lab(), &HostOptions { fill: Some(4), spawn_group: "duel".into(), ..Default::default() }).unwrap();
        assert!(host.addr().ip().is_loopback() && host.addr().port() != 0);
        let (_scene, world) = ClientWorld::load(&lab()).unwrap();
        let mut me = Bot::new(host.addr(), Character::Human, world, Behavior::Idle, 0).unwrap();
        let end = Instant::now() + Duration::from_secs(10);
        let mut bots = 0;
        while Instant::now() < end && bots < 3 {
            me.pump(Instant::now());
            bots = me.client.status().map_or(0, |s| s.roster.iter().filter(|e| e.flags & ROSTER_BOT != 0).count());
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(bots, 3, "one human and three bots make four players");
        let started = Instant::now();
        drop(host);
        assert!(started.elapsed() < Duration::from_secs(3), "dropping the host stops its thread promptly: {:?}", started.elapsed());
    }

    #[test]
    fn a_bad_map_or_option_is_an_error_that_says_what_is_wrong() {
        let missing = LocalHost::start(Path::new("no/such/map.json"), &HostOptions::default()).err().unwrap();
        assert!(missing.contains("no/such/map.json"), "{missing}");
        let skill = LocalHost::start(&lab(), &HostOptions { bot_skill: Some("godlike".into()), ..Default::default() }).err().unwrap();
        assert!(skill.contains("godlike") && skill.contains("nightmare"), "{skill}");
        let group = LocalHost::start(&lab(), &HostOptions { spawn_group: "nowhere".into(), ..Default::default() }).err().unwrap();
        assert!(group.contains("nowhere"), "{group}");
    }
}
