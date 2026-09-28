//! Plays a headless match between bots and prints what happened: kills, accuracy, how far they travelled, how long they stood still.
//! No window, no socket, faster than real time; it is how bot behaviour and arena balance are *measured* rather than guessed.
//!
//! ```text
//! cargo run --release --example bot_match -- examples/test_lab.json [--bots 8] [--secs 300] [--skill normal] [--seed 1] [--quiet]
//! ```
//!
//! The scene's own `bots` block supplies the roster; `--bots N` overrides how many fight and `--skill` the level of generated ones.

use red_engine2::sim::ai::skill::level_from_name;
use red_engine2::sim::ai::BotsConfig;
use red_engine2::sim::clock::TICK_RATE_HZ;
use red_engine2::sim::match_sim::{MatchSim, MAX_PLAYERS};
use red_engine2::sim::spawns::parse_spawns;
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut path = String::from("examples/test_lab.json");
    let (mut bots, mut secs, mut skill, mut quiet) = (None::<usize>, 300.0f32, None::<f32>, false);
    let (mut trace, mut weapon) = (None::<usize>, None::<String>);
    let mut first = true;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--bots" => bots = args.next().and_then(|v| v.parse().ok()),
            "--secs" => secs = args.next().and_then(|v| v.parse().ok()).unwrap_or(secs),
            "--skill" => skill = args.next().and_then(|v| level_from_name(&v)),
            "--quiet" => quiet = true,
            "--trace" => trace = args.next().and_then(|v| v.parse().ok()),
            "--weapon" => weapon = args.next(),
            other if first && !other.starts_with("--") => path = other.to_string(),
            other => {
                eprintln!("unknown argument {other}");
                std::process::exit(2);
            }
        }
        first = false;
    }
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("{path}: {e}");
        std::process::exit(1);
    });
    let text = match &weapon {
        Some(w) => {
            let mut v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
            v["weapons"]["starting"] = serde_json::Value::String(w.clone());
            v.to_string()
        }
        None => text,
    };
    let scene = red_engine2::schema::parse_scene(&text).unwrap_or_else(|e| {
        eprintln!("{path}: {}", e.join("; "));
        std::process::exit(1);
    });
    let spawns = parse_spawns(&text).unwrap_or_else(|e| {
        eprintln!("{path}: {e}");
        std::process::exit(1);
    });
    let mut sim = MatchSim::new(&scene, spawns);
    let mut cfg: BotsConfig = sim.bots_config().clone();
    if let Some(l) = skill {
        cfg.level = l;
        for b in &mut cfg.roster {
            b.level = l;
        }
    }
    let n = bots.unwrap_or(if cfg.fill > 0 { cfg.fill } else { 8 }).min(MAX_PLAYERS);
    for i in 0..n {
        if sim.add_bot(&cfg.spec(i)).is_none() {
            eprintln!("no room for bot {i}");
        }
    }
    let ticks = (secs * TICK_RATE_HZ as f32) as u64;
    let mut moved = [0.0f32; MAX_PLAYERS];
    let mut still = [0u64; MAX_PLAYERS];
    let mut alive = [0u64; MAX_PLAYERS];
    let mut last: [Option<glam::Vec2>; MAX_PLAYERS] = [None; MAX_PLAYERS];
    let mut engaged = [0u64; MAX_PLAYERS];
    let mut in_air = [0u64; MAX_PLAYERS];
    let started = Instant::now();
    let mut first_kill = None;
    for _ in 0..ticks {
        sim.tick_once();
        if let Some(slot) = trace {
            if sim.tick() % 30 == 0 {
                if let (Some(p), Some(b)) = (sim.player(slot), sim.brain(slot)) {
                    let others: Vec<String> = sim
                        .players()
                        .filter(|(s, _)| *s != slot)
                        .map(|(s, q)| format!("{s}:({:.1},{:.1}){}", q.state.pos.x, q.state.pos.y, if q.combat.is_dead() { "x" } else { "" }))
                        .collect();
                    println!("   others {}", others.join(" "));
                    println!(
                        "t={:>6.1}s pos ({:>6.1},{:>6.1}) y {:.1} hp {} | {}",
                        sim.tick() as f32 / 60.0,
                        p.state.pos.x,
                        p.state.pos.y,
                        p.state.foot_y,
                        p.combat.hp,
                        b.debug_line(sim.tick())
                    );
                }
            }
        }
        for (slot, p) in sim.players() {
            if p.combat.is_dead() {
                last[slot] = None;
                continue;
            }
            alive[slot] += 1;
            if let Some(prev) = last[slot] {
                let d: f32 = (p.state.pos - prev).length();
                moved[slot] += d;
                if d / red_engine2::player::FIXED_DT < 0.5 {
                    still[slot] += 1;
                }
            }
            last[slot] = Some(p.state.pos);
            if sim.brain(slot).and_then(|b| b.target()).is_some() {
                engaged[slot] += 1;
            }
            if p.state.vy.abs() > 0.5 {
                in_air[slot] += 1;
            }
        }
        if first_kill.is_none() && sim.players().any(|(_, p)| p.combat.kills > 0) {
            first_kill = Some(sim.tick());
        }
    }
    let wall = started.elapsed().as_secs_f32();
    println!("{path}: {n} bots, {secs:.0} s simulated in {wall:.2} s ({:.0}x real time)", secs / wall.max(0.001));
    let (mut kills, mut shots, mut hits) = (0u32, 0u32, 0u32);
    for (slot, p) in sim.players() {
        let c = &p.combat;
        let name = sim.bot_name(slot).unwrap_or("human");
        let spec = sim.brain(slot).map(|b| b.spec().clone());
        if !quiet {
            println!(
                "  {slot} {name:<9} {:<8} skill {:.2} {:<9} | kills {:>2} deaths {:>2} | shots {:>4} hits {:>4} acc {:>3.0}% | moved {:>5.0} m still {:>3.0}% engaged {:>3.0}% airborne {:>3.0}% | {}",
                format!("{:?}", p.state.character),
                spec.as_ref().map_or(0.0, |s| s.level),
                spec.as_ref().map_or_else(String::new, |s| format!("{:?}", s.style)),
                c.kills,
                c.deaths,
                c.shots,
                c.hits,
                if c.shots > 0 { 100.0 * c.hits as f32 / c.shots as f32 } else { 0.0 },
                moved[slot],
                100.0 * still[slot] as f32 / alive[slot].max(1) as f32,
                100.0 * engaged[slot] as f32 / alive[slot].max(1) as f32,
                100.0 * in_air[slot] as f32 / alive[slot].max(1) as f32,
                c.weapon.name(),
            );
        }
        kills += c.kills;
        shots += c.shots;
        hits += c.hits;
    }
    let minutes = secs / 60.0;
    println!(
        "  total: {kills} kills ({:.1}/min, {:.1}/min per bot), {shots} shots, accuracy {:.0}%, first kill at {}",
        kills as f32 / minutes,
        kills as f32 / minutes / n.max(1) as f32,
        if shots > 0 { 100.0 * hits as f32 / shots as f32 } else { 0.0 },
        first_kill.map_or("never".to_string(), |t| format!("{:.1} s", t as f32 / TICK_RATE_HZ as f32)),
    );
}
