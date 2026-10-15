//! Heap allocations per steady-state tick of the match with Workshop as its game, counted with a global allocator (the engine's
//! `tests/alloc_budget.rs` pattern). The same scene without its `workshop` block is the engine's own share; Workshop's share is the
//! difference, and its budget is zero.
//!
//! `cargo test -p workshop_server --release --test alloc_budget -- --nocapture` prints the numbers.

mod common;

use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::spawns::parse_spawns;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

struct Counting;
static ALLOCS: AtomicU64 = AtomicU64::new(0);

// SAFETY: forwards to the system allocator unchanged; only counts calls.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Counting = Counting;

/// Allocations per tick over `ticks` ticks after `warm` warm-up ticks.
fn per_tick(text: &str, dir: &std::path::Path, warm: u32, ticks: u32) -> f64 {
    let scene = red_engine2::scene_blocks::parse_scene_in(text, Some(dir)).unwrap();
    let mut sim = MatchSim::try_new(&scene, parse_spawns(text).unwrap()).unwrap();
    for _ in 0..warm {
        sim.tick_once();
    }
    let before = ALLOCS.load(Ordering::Relaxed);
    for _ in 0..ticks {
        sim.tick_once();
    }
    (ALLOCS.load(Ordering::Relaxed) - before) as f64 / ticks as f64
}

/// One test (not several) so the global counter is not shared with parallel tests.
#[test]
fn per_tick_allocations() {
    workshop_server::register();
    for n in [10, 100, 1000] {
        let (text, dir) = common::workshop(n);
        let mut bare: serde_json::Value = serde_json::from_str(&text).unwrap();
        bare.as_object_mut().unwrap().remove("workshop");
        let engine = per_tick(&bare.to_string(), &dir, 3000, 600);
        let with = per_tick(&text, &dir, 3000, 600);
        let _ = std::fs::remove_dir_all(&dir);
        let workshop = with - engine;
        println!("N = {n}: {with:.4} allocations per tick, the engine alone {engine:.4}, Workshop {workshop:.4}");
        if std::env::var_os("WORKSHOP_ALLOC_REPORT").is_none() {
            assert!(workshop <= 0.0, "N = {n}: Workshop allocates {workshop:.4} times per steady-state tick (budget 0)");
        }
    }
}
