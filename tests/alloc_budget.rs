//! Heap allocations per simulation tick, counted with a global allocator (objective, noise-free
//! feedback for the tick-scoped scratch buffers — ADR 0014).
//!
//! Run `cargo test --release --test alloc_budget -- --nocapture` to see the numbers.

use glam::Vec3;
use red_engine2::physics::PropWorld;
use std::alloc::{GlobalAlloc, Layout, System};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

struct Counting;
static ALLOCS: AtomicU64 = AtomicU64::new(0);

// SAFETY: forwards to the system allocator unchanged; only counts calls.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.realloc(p, l, n)
    }
}

#[global_allocator]
static A: Counting = Counting;

fn allocs<R>(f: impl FnOnce() -> R) -> (u64, R) {
    let before = ALLOCS.load(Ordering::Relaxed);
    let r = f();
    (ALLOCS.load(Ordering::Relaxed) - before, r)
}

fn world(map: &str) -> PropWorld {
    let scene = red_engine2::load_scene(Path::new(map)).expect("scene");
    PropWorld::new(&scene, None)
}

/// One test (not several) so the global counter is not shared with parallel tests.
///
/// Budgets, not exact numbers: our own code must add none; whatever is left is inside rapier/parry
/// (CCD solver, EPA contact generation) and only exists while props are awake. See ADR 0014 for the
/// measured before/after (idle 5.03 -> 0.03 allocations per tick).
#[test]
fn per_tick_allocations() {
    let mut w = world("examples/store.json");
    let feet = Vec3::new(0.0, 0.0, 6.0);
    w.set_player(feet, 0.35, 1.8);
    for _ in 0..30 {
        w.step(); // settle: first ticks may grow internal buffers
    }
    let (idle, _) = allocs(|| {
        for _ in 0..120 {
            w.step();
        }
    });
    println!("idle store, 120 ticks: {idle} allocations ({:.2}/tick)", idle as f64 / 120.0);
    assert!(idle <= 12, "an idle map must not allocate per tick: {idle} allocations in 120 ticks");

    // Something is moving: knock a prop about so bodies are awake and the cascade runs.
    let target = (0..w.props().len()).find(|&i| w.mass(i) < 3.0).expect("a light prop");
    w.strike_impulse(target, Vec3::X, w.prop_pose(target).transform_point3(Vec3::ZERO), 8.0);
    for _ in 0..10 {
        w.step(); // warm-up of the scratch buffers for this scenario
    }
    let grows_before = w.scratch_grows();
    let (busy, _) = allocs(|| {
        for _ in 0..60 {
            w.step();
        }
    });
    println!("one prop knocked, 60 ticks: {busy} allocations ({:.2}/tick), awake now: {}", busy as f64 / 60.0, w.awake_count());
    assert_eq!(w.scratch_grows(), grows_before, "our scratch buffers must not grow in steady state");
    assert!(busy as f64 / 60.0 <= 40.0, "awake-prop ticks regressed: {:.1} allocations/tick (was ~21, all inside rapier)", busy as f64 / 60.0);
}
