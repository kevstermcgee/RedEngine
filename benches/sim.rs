//! Simulation benchmarks (criterion). `cargo bench --bench sim`, then
//! `cargo run --release --bin bench_check` to compare against the stored baseline (see
//! `benches/README.md`). Every id here is stable: the baseline file is keyed by it.

mod common;

use common::synthetic_scene;
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use glam::Vec3;
use std::hint::black_box;
use red_engine2::physics::PropWorld;
use red_engine2::sim::change::{ChangeCursor, GenClock};
use red_engine2::sim::components::Transform;
use red_engine2::sim::entities::Entities;
use red_engine2::sim::snapshot::{encode_delta, encode_full};
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

fn config() -> Criterion {
    Criterion::default().warm_up_time(Duration::from_millis(500)).measurement_time(Duration::from_secs(2)).sample_size(30)
}

criterion_group! { name = benches; config = config(); targets = tick_untouched, tick_active, frame_sync, promotion, build_world, snapshot }
criterion_main!(benches);
