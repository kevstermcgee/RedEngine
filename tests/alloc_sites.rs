//! Scratch diagnostic (not a regression test): prints where per-tick allocations come from.
//! `cargo test --release --test alloc_sites -- --nocapture --ignored`

use glam::Vec3;
use red_engine2::physics::PropWorld;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

struct Sites;
static ARMED: AtomicBool = AtomicBool::new(false);
static SITES: Mutex<BTreeMap<String, u64>> = Mutex::new(BTreeMap::new());
thread_local! { static BUSY: Cell<bool> = const { Cell::new(false) }; }

fn record() {
    if !ARMED.load(Ordering::Relaxed) {
        return;
    }
    BUSY.with(|b| {
        if b.replace(true) {
            return;
        }
        let bt = std::backtrace::Backtrace::force_capture().to_string();
        // The first frame outside the allocator/std plumbing that belongs to a crate we care about.
        let key = bt
            .lines()
            .map(str::trim)
            .filter(|l| !l.starts_with("at "))
            .filter(|l| l.contains("rapier") || l.contains("parry") || l.contains("red_engine2") || l.contains("glam"))
            .take(3)
            .map(|l| l.split_once(": ").map_or(l, |x| x.1).to_string())
            .collect::<Vec<_>>()
            .join("  <-  ");
        *SITES.lock().unwrap().entry(key).or_insert(0) += 1;
        b.set(false);
    });
}

// SAFETY: forwards to the system allocator; `record` guards against re-entrancy.
unsafe impl GlobalAlloc for Sites {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        record();
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        record();
        System.realloc(p, l, n)
    }
}

#[global_allocator]
static A: Sites = Sites;

#[test]
#[ignore]
fn where_do_allocations_come_from() {
    let scene = red_engine2::load_scene(Path::new("examples/store.json")).unwrap();
    let mut w = PropWorld::new(&scene, None);
    w.set_player(Vec3::new(0.0, 0.0, 6.0), 0.35, 1.8);
    for _ in 0..30 {
        w.step();
    }
    ARMED.store(true, Ordering::SeqCst);
    for _ in 0..30 {
        w.step();
    }
    let target = (0..w.props().len()).find(|&i| w.mass(i) < 3.0).unwrap();
    w.strike_impulse(target, Vec3::X, w.prop_pose(target).transform_point3(Vec3::ZERO), 8.0);
    for _ in 0..30 {
        w.step();
    }
    ARMED.store(false, Ordering::SeqCst);
    let mut v: Vec<_> = SITES.lock().unwrap().iter().map(|(k, n)| (*n, k.clone())).collect();
    v.sort_by(|a, b| b.cmp(a));
    for (n, k) in v.iter().take(12) {
        println!("{n:6}  {k}");
    }
}
