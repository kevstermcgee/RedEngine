//! Simulation benchmarks (criterion). `cargo bench --bench sim`, then
//! `cargo run --release --bin bench_check` to compare against the stored baseline (see
//! `benches/README.md`). Every id here is stable: the baseline file is keyed by it.

mod common;

use common::synthetic_scene;
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use glam::Vec3;
use red_engine2::physics::PropWorld;
use red_engine2::sim::change::{ChangeCursor, GenClock};
use red_engine2::sim::components::Transform;
use red_engine2::sim::entities::Entities;
use red_engine2::sim::snapshot::{encode_delta, encode_full};
use std::hint::black_box;
use std::time::{Duration, Instant};

const SIZES: [usize; 3] = [100, 1000, 4000];

fn far_player(w: &mut PropWorld) {
    w.set_player(Vec3::new(900.0, 0.0, 900.0), 0.35, 1.75);
}

/// One tick with N loose props that nobody has touched (the common case in Prop Hunt).
fn tick_untouched(c: &mut Criterion) {
    let mut g = c.benchmark_group("tick/untouched");
    for n in SIZES {
        let scene = synthetic_scene(n);
        let mut w = PropWorld::new(&scene, None);
        far_player(&mut w);
        for _ in 0..10 {
            w.step();
        }
        g.bench_function(BenchmarkId::from_parameter(n), |b| b.iter(|| w.step()));
    }
    g.finish();
}

/// One tick with N loose props of which 16 are kept awake (an impulse each tick).
fn tick_active(c: &mut Criterion) {
    let mut g = c.benchmark_group("tick/16_awake");
    for n in SIZES {
        let scene = synthetic_scene(n);
        let mut w = PropWorld::new(&scene, None);
        far_player(&mut w);
        let awake: Vec<usize> = (0..n).step_by((n / 16).max(1)).take(16).collect();
        g.bench_function(BenchmarkId::from_parameter(n), |b| {
            b.iter(|| {
                for &p in &awake {
                    w.strike_impulse(p, Vec3::Y, Vec3::ZERO, 0.05);
                }
                w.step();
            })
        });
    }
    g.finish();
}

/// The per-frame write of prop poses into the scene (what the renderer reads).
fn frame_sync(c: &mut Criterion) {
    let mut g = c.benchmark_group("frame/sync_scene");
    for n in SIZES {
        let mut scene = synthetic_scene(n);
        let mut w = PropWorld::new(&scene, None);
        far_player(&mut w);
        w.step();
        g.bench_function(BenchmarkId::from_parameter(n), |b| b.iter(|| w.sync_scene(black_box(&mut scene))));
    }
    g.finish();
}

/// Cost of promoting ONE untouched prop to a full dynamic one (activate: entity + body + colliders),
/// measured on a world of N. Props are spaced apart so no promotion cascades into neighbours.
fn promotion(c: &mut Criterion) {
    let mut g = c.benchmark_group("promote/one");
    for n in SIZES {
        let scene = synthetic_scene(n);
        g.bench_function(BenchmarkId::from_parameter(n), |b| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                let mut done = 0u64;
                while done < iters {
                    let mut w = PropWorld::new(&scene, None);
                    far_player(&mut w);
                    let batch = (iters - done).min(n as u64);
                    let t = Instant::now();
                    for p in 0..batch as usize {
                        w.activate(p);
                    }
                    total += t.elapsed();
                    done += batch;
                }
                total
            })
        });
    }
    g.finish();
}

/// Load-time cost: building the physics world for a map with N loose props.
fn build_world(c: &mut Criterion) {
    let mut g = c.benchmark_group("build/world");
    g.sample_size(10);
    for n in SIZES {
        let scene = synthetic_scene(n);
        g.bench_function(BenchmarkId::from_parameter(n), |b| b.iter_batched(|| (), |_| PropWorld::new(black_box(&scene), None), BatchSize::PerIteration));
    }
    g.finish();
}

/// Snapshot encoding of N dynamic entities: everything, a quiet world (nothing changed), and 10% changed.
fn snapshot(c: &mut Criterion) {
    let mut g = c.benchmark_group("snapshot");
    for n in [16usize, 256, 4096] {
        let mut clock = GenClock::default();
        let mut e = Entities::default();
        for i in 0..n {
            e.spawn(Transform::at(Vec3::new(i as f32, 0.0, 0.0)), clock.now());
        }
        let mut cursor = ChangeCursor::default();
        let mut buf = Vec::with_capacity(12 + n * 32);
        g.bench_function(BenchmarkId::new("full", n), |b| {
            b.iter(|| {
                buf.clear();
                encode_full(black_box(&e), 1, &mut buf)
            })
        });
        cursor.catch_up(&mut clock);
        g.bench_function(BenchmarkId::new("delta_quiet", n), |b| {
            b.iter(|| {
                buf.clear();
                encode_delta(black_box(&e), cursor.last(), 1, &mut buf)
            })
        });
        for slot in (0..n).step_by(10) {
            e.transforms.get_mut(slot, clock.now()).position.y = 1.0;
        }
        g.bench_function(BenchmarkId::new("delta_10pct", n), |b| {
            b.iter(|| {
                buf.clear();
                encode_delta(black_box(&e), cursor.last(), 1, &mut buf)
            })
        });
    }
    g.finish();
}

/// The authoritative match tick: N players walking around the Test Lab (movement, physics, rules and combat), one tick per iteration,
/// with fresh inputs pushed every tick like N connected clients. This is the server's whole per-tick simulation cost.
fn match_tick(c: &mut Criterion) {
    use red_engine2::player::Character;
    use red_engine2::sim::match_sim::MatchSim;
    use red_engine2::sim::player::PlayerInput;
    use red_engine2::sim::spawns::parse_spawns;
    let path = std::path::Path::new("examples/test_lab.json");
    let scene = red_engine2::load_scene(path).expect("lab");
    let spawns = parse_spawns(&std::fs::read_to_string(path).expect("lab text")).expect("spawns");
    let mut g = c.benchmark_group("match/tick");
    for n in [2usize, 8] {
        let mut sim = MatchSim::new(&scene, spawns.clone());
        for _ in 0..n {
            sim.add_player(Character::Human).expect("room");
        }
        let mut seq = 0u32;
        g.bench_function(BenchmarkId::from_parameter(n), |b| {
            b.iter(|| {
                seq += 1;
                for slot in 0..n {
                    sim.push_input(slot, PlayerInput { seq, forward: 1, yaw: seq as f32 * 0.02 + slot as f32, ..Default::default() });
                }
                sim.tick_once();
            })
        });
    }
    g.finish();
}

/// Per-client network cost: encoding one worst-case snapshot (8 players + 30 props), decoding one input packet, and the
/// authoritative-state checksum (what a trace records every tick).
fn net_codec(c: &mut Criterion) {
    use red_engine2::net::protocol::{
        ClientMsg, InputPacket, PlayerSnap, PropSnap, ServerMsg, Snapshot, MAX_PLAYERS_PER_SNAPSHOT, MAX_PROPS_PER_SNAPSHOT, NO_PROP,
    };
    use red_engine2::sim::player::PlayerInput;
    let snap = Snapshot {
        seq: 1,
        server_tick: 1,
        ack_input_seq: 1,
        echo_time_ms: 1,
        echo_hold_ms: 0,
        players: vec![
            PlayerSnap { id: 0, character: 0, flags: 0, pos: [1.0; 3], yaw: 0.1, pitch: 0.1, speed: 3.0, vy: 0.0, weapon: 0, held: NO_PROP, hp: 100 };
            MAX_PLAYERS_PER_SNAPSHOT
        ],
        props: vec![PropSnap { id: 0, pos: [1.0; 3], rot: [0.0, 0.0, 0.0, 1.0] }; MAX_PROPS_PER_SNAPSHOT],
    };
    let msg = ServerMsg::Snapshot(snap);
    let mut buf = Vec::with_capacity(1400);
    let mut encoded = Vec::new();
    msg.encode(&mut encoded);
    let mut input = Vec::new();
    ClientMsg::Input(InputPacket {
        snapshot_ack: 9,
        client_time_ms: 1,
        inputs: vec![PlayerInput { seq: 1, forward: 1, ..Default::default() }; 4],
        ..Default::default()
    })
    .encode(&mut input);
    let mut g = c.benchmark_group("net");
    g.bench_function("encode_worst_snapshot", |b| {
        b.iter(|| {
            buf.clear();
            black_box(&msg).encode(&mut buf);
        })
    });
    g.bench_function("decode_worst_snapshot", |b| b.iter(|| ServerMsg::decode(black_box(&encoded))));
    g.bench_function("decode_input_packet", |b| b.iter(|| ClientMsg::decode(black_box(&input))));
    // Authentication (ADR 0028): the tag on the worst snapshot the server sends to every client at 30 Hz, and the check on an input packet
    // the server receives from every client at 60 Hz.
    let key = red_engine2::net::auth::SessionKey::derive(b"bench-key", 1, 2);
    let mut signed_in = input.clone();
    key.sign(red_engine2::net::auth::Direction::ToServer, &mut signed_in);
    g.bench_function("sign_worst_snapshot", |b| {
        b.iter(|| {
            buf.clear();
            buf.extend_from_slice(black_box(&encoded));
            key.sign(red_engine2::net::auth::Direction::ToClient, &mut buf);
        })
    });
    g.bench_function("verify_input_packet", |b| b.iter(|| key.verify(red_engine2::net::auth::Direction::ToServer, black_box(&signed_in))));
    g.finish();
}

fn config() -> Criterion {
    Criterion::default().warm_up_time(Duration::from_millis(500)).measurement_time(Duration::from_secs(2)).sample_size(30)
}

criterion_group! { name = benches; config = config(); targets = tick_untouched, tick_active, frame_sync, promotion, build_world, snapshot, match_tick, net_codec }
criterion_main!(benches);
