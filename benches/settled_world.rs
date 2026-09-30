//! Long-session scaling: what a tick, a scene sync and a per-client prop selection cost once every prop has been disturbed and has
//! settled again, against an untouched map and against a few props still moving. Plain timing (percentiles), not gated.
//! Run: `cargo bench --bench settled_world`. Headless: no GPU, no sockets (`net::props_to_send` is the server's own selection function).
//!
//! Phases per N loose props (`benches/common`): `untouched` (nothing promoted), `settled` (all promoted, all asleep, poses published),
//! `16 moving` (settled world with 16 props kept awake). For each: `step` (one physics tick), `sync_scene`, `awake_count`, and
//! `props_to_send` for a client that has confirmed everything (steady state) and for one that has confirmed nothing (late join).

mod common;

use common::synthetic_scene;
use glam::Vec3;
use red_engine2::net::props_to_send;
use red_engine2::sim::change::Generation;
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::spawns::Spawn;
use std::time::Instant;

fn pct(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() as f64 - 1.0) * p).round() as usize]
}

fn time(label: &str, n: usize, phase: &str, samples: usize, mut f: impl FnMut()) {
    for _ in 0..5 {
        f();
    }
    let mut us: Vec<f64> = (0..samples)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed().as_secs_f64() * 1e6
        })
        .collect();
    println!(
        "N={n:<5} {phase:<9} {label:<28} samples={samples:<4} p50={:>9.1}us p95={:>9.1}us p99={:>9.1}us",
        pct(&mut us, 0.5),
        pct(&mut us, 0.95),
        pct(&mut us, 0.99)
    );
}

fn measure(sim: &mut MatchSim, scene: &mut red_engine2::schema::Scene, n: usize, phase: &str, moving: &[usize]) {
    let step = |sim: &mut MatchSim| {
        for &p in moving {
            sim.props_mut().strike_impulse(p, Vec3::Y, Vec3::ZERO, 0.05);
        }
        sim.props_mut().step();
    };
    time("PropWorld::step", n, phase, 300, || step(sim));
    time("sync_scene", n, phase, 300, || sim.props_mut().sync_scene(scene));
    time("awake_count", n, phase, 300, || {
        std::hint::black_box(sim.props().awake_count());
    });
    let slots = sim.props().entities().len();
    let (mut scratch, mut out, mut sent) = (Vec::new(), Vec::new(), Vec::new());
    // A client that has confirmed everything up to the newest generation: the steady state of every settled session.
    let all_known = vec![Generation(u64::MAX); slots];
    time("props_to_send (all confirmed)", n, phase, 300, || {
        props_to_send(sim, None, None, &all_known, &mut scratch, &mut out, &mut sent, 60);
        std::hint::black_box(&out);
    });
    let none_known = vec![Generation(0); slots];
    time("props_to_send (nothing known)", n, phase, 100, || {
        props_to_send(sim, None, None, &none_known, &mut scratch, &mut out, &mut sent, 60);
        std::hint::black_box(&out);
    });
    println!("        -> promoted={} awake={} entities={}", sim.props().dynamic_count(), sim.props().awake_count(), slots);
}

fn main() {
    for n in [1000usize, 4000] {
        let mut scene = synthetic_scene(n);
        let mut sim = MatchSim::new(&scene, vec![Spawn { id: "s".into(), position: [900.0, 0.0, 900.0], yaw_deg: 0.0, group: String::new() }]);
        for _ in 0..10 {
            sim.tick_once();
        }
        measure(&mut sim, &mut scene, n, "untouched", &[]);
        // Disturb every prop once, then run until everything sleeps.
        for p in 0..sim.props().props().len() {
            sim.props_mut().strike_impulse(p, Vec3::Y, Vec3::ZERO, 0.25);
        }
        let mut ticks = 0;
        while sim.props().awake_count() > 0 && ticks < 3000 {
            sim.tick_once();
            ticks += 1;
        }
        println!("N={n}: all props disturbed, settled after {ticks} ticks (awake now {})", sim.props().awake_count());
        for _ in 0..5 {
            sim.tick_once();
        }
        measure(&mut sim, &mut scene, n, "settled", &[]);
        let moving: Vec<usize> = (0..n).step_by((n / 16).max(1)).take(16).collect();
        measure(&mut sim, &mut scene, n, "16 moving", &moving);
    }
}
