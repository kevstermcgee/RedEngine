//! The live renderer's per-frame object preparation (`object_staging::SceneStaging`): the cached path must produce exactly the bytes the
//! old rebuild-everything path did, after every kind of scene mutation, while rewriting only what changed and allocating nothing in
//! steady state. CPU-only (no GPU): what the GPU does with the uploads is not measured here.
//!
//! One test drives the counting allocator (a global counter is not shared safely with parallel tests), the others compare bytes.

#![cfg(feature = "gfx")]

use glam::Vec3;
use red_engine2::object_staging::{local_bounds, uncached_uniforms, SceneStaging};
use red_engine2::render::wrap_offsets;
use red_engine2::schema::{Object, ObjectKind, Scene};
use red_engine2::track::Track;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::path::Path;

struct Counting;
// Per thread, so the other tests in this binary (they run in parallel) cannot add to a measurement.
thread_local! {
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
}
fn allocs_now() -> u64 {
    ALLOCS.with(|c| c.get())
}
fn count() {
    let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
}

// SAFETY: forwards to the system allocator unchanged; only counts calls.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        count();
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Counting = Counting;

const STRIDE: u64 = 256;

fn load(map: &str) -> Scene {
    red_engine2::load_scene(Path::new(map)).expect("scene")
}

fn make(scene: &Scene) -> (SceneStaging, Vec<(Vec3, Vec3)>, Vec<Vec3>) {
    let bounds = local_bounds(scene);
    let offsets = wrap_offsets(scene);
    (SceneStaging::new(bounds.len(), offsets.len(), 0, STRIDE), bounds, offsets)
}

/// Every object (groups included), depth first.
fn all_objects_mut(objects: &mut [Object], out: &mut Vec<*mut Object>) {
    for o in objects.iter_mut() {
        out.push(o as *mut Object);
        if let ObjectKind::Group(children) = &mut o.kind {
            all_objects_mut(children, out);
        }
    }
}

fn objects_of(scene: &mut Scene) -> Vec<&mut Object> {
    let mut ptrs = Vec::new();
    all_objects_mut(&mut scene.objects, &mut ptrs);
    // SAFETY: each pointer is a distinct object of the scene (a group and its children are different objects), and `scene` stays
    // mutably borrowed for the lifetime of the returned references.
    ptrs.into_iter().map(|p| unsafe { &mut *p }).collect()
}

fn first_leaf(objects: &mut [Object]) -> &mut Object {
    let first = objects.first_mut().expect("a non-empty object list");
    match first.kind {
        ObjectKind::Group(_) => match &mut first.kind {
            ObjectKind::Group(c) => first_leaf(c),
            _ => unreachable!(),
        },
        _ => first,
    }
}

/// After `update_scene`, the cached bytes for every scene slot equal a from-scratch rebuild.
fn assert_matches_reference(st: &SceneStaging, scene: &Scene, t: f32, offsets: &[Vec3], slots: usize, what: &str) {
    let reference = uncached_uniforms(scene, t, offsets, STRIDE, slots);
    assert_eq!(st.bytes(), &reference[..], "cached uniforms diverged from the full rebuild after: {what}");
}

#[test]
fn cached_bytes_match_the_full_rebuild_after_every_kind_of_mutation() {
    let mut scene = load("examples/test_lab.json");
    let (mut st, bounds, offsets) = make(&scene);
    let slots = bounds.len() * offsets.len();
    let n = st.update_scene(&scene, 0.0, &offsets, &bounds);
    assert_eq!(n, slots, "the first frame stages everything");
    assert_matches_reference(&st, &scene, 0.0, &offsets, slots, "first frame");
    st.dirty_ranges();

    // Nothing changed: nothing rewritten, nothing to upload (the camera is not part of this data).
    assert_eq!(st.update_scene(&scene, 0.0, &offsets, &bounds), 0);
    assert!(st.dirty_ranges().is_empty(), "an unchanged scene must upload nothing");

    let count = objects_of(&mut scene).len();
    assert!(count > 20, "the lab has many objects");
    // Each mutation kind, applied to a different object each round, checked against the reference every time.
    for round in 0..count.min(60) {
        let mut objs = objects_of(&mut scene);
        let o = &mut *objs[round];
        match round % 6 {
            0 => o.position = Track::constant(Vec3::new(round as f32, 1.0, 2.0)), // physics / network pose write
            1 => o.rotation = Track::constant(Vec3::new(0.0, 10.0 + round as f32, 0.0)),
            2 => o.scale = Track::constant(Vec3::splat(0.001)), // a pooled object switched off
            3 => {
                // material: colour of a prim, or of whatever the kind carries
                if let Some(m) = o.material.as_mut() {
                    m.color = Track::constant(Vec3::new(0.9, 0.1, 0.2));
                    m.metallic = 0.5;
                }
            }
            4 => o.position = Track::constant(Vec3::new(-3.0, round as f32 * 0.1, 4.0)), // a parent moving: descendants follow
            _ => o.scale = Track::constant(Vec3::ONE),                                   // shown again
        }
        drop(objs);
        st.update_scene(&scene, 0.0, &offsets, &bounds);
        assert_matches_reference(&st, &scene, 0.0, &offsets, slots, &format!("mutation round {round}"));
        st.dirty_ranges();
    }
}

#[test]
fn a_parent_move_dirties_every_descendant_and_a_leaf_move_only_itself() {
    let mut scene = load("examples/test_lab.json");
    let (mut st, bounds, offsets) = make(&scene);
    st.update_scene(&scene, 0.0, &offsets, &bounds);
    // The biggest group: moving it must restage at least all its leaves.
    fn leaves(o: &Object) -> usize {
        match &o.kind {
            ObjectKind::Group(c) => c.iter().map(leaves).sum(),
            ObjectKind::Prim(_) => 1,
            _ => 1,
        }
    }
    let (gi, glen) = scene
        .objects
        .iter()
        .enumerate()
        .filter(|(_, o)| matches!(o.kind, ObjectKind::Group(_)))
        .map(|(i, o)| (i, leaves(o)))
        .max_by_key(|x| x.1)
        .expect("a group");
    scene.objects[gi].position = Track::constant(Vec3::new(50.0, 0.0, 50.0));
    let rewritten = st.update_scene(&scene, 0.0, &offsets, &bounds);
    assert!(rewritten >= glen, "moving a group restaged {rewritten} slots, but it holds {glen} leaves");
    assert!(rewritten < bounds.len(), "a group move must not restage the whole scene ({rewritten} of {})", bounds.len());
}

#[test]
fn animated_tracks_restage_only_while_they_move_and_wrapped_worlds_stay_correct() {
    let mut scene = load("examples/endless_shore.json");
    let (mut st, bounds, offsets) = make(&scene);
    assert_eq!(offsets.len(), 3, "the shore is a looping world: three images");
    let slots = bounds.len() * offsets.len();
    st.update_scene(&scene, 0.0, &offsets, &bounds);
    assert_matches_reference(&st, &scene, 0.0, &offsets, slots, "wrapped first frame");
    // Time passing on a scene without keyframes must not restage anything.
    assert_eq!(st.update_scene(&scene, 5.0, &offsets, &bounds), 0);
    // A moved object is restaged in all three images.
    let mut objs = objects_of(&mut scene);
    let moved = objs.iter_mut().find(|o| !matches!(o.kind, ObjectKind::Group(_))).expect("a leaf object");
    moved.position = Track::constant(Vec3::new(1.0, 2.0, 3.0));
    drop(objs);
    let n = st.update_scene(&scene, 5.0, &offsets, &bounds);
    assert!(n >= 3 && n % 3 == 0, "a moved leaf restages one slot per world image, got {n}");
    assert_matches_reference(&st, &scene, 5.0, &offsets, slots, "wrapped mutation");
}

#[test]
fn dirty_ranges_are_coalesced_and_cover_exactly_the_changed_slots() {
    let mut scene = load("examples/test_lab.json");
    let (mut st, bounds, offsets) = make(&scene);
    st.update_scene(&scene, 0.0, &offsets, &bounds);
    let full = st.dirty_ranges();
    assert_eq!(full.len(), 1, "the first frame is one contiguous upload");
    // Move a scattering of objects; the number of upload calls must stay far below the number of changed slots.
    let mut gpu_mirror = st.bytes().to_vec();
    let mut objs = objects_of(&mut scene);
    let len = objs.len();
    for (i, o) in objs.iter_mut().enumerate() {
        if i % 3 == 0 && i < len {
            o.position = Track::constant(Vec3::new(i as f32 * 0.37, 0.5, -(i as f32)));
        }
    }
    drop(objs);
    let changed = st.update_scene(&scene, 0.0, &offsets, &bounds);
    let ranges = st.dirty_ranges();
    assert!(changed > 0 && ranges.len() <= changed, "{} upload ranges for {changed} changed slots", ranges.len());
    // Applying only those ranges to a mirror of the GPU buffer reproduces the staging bytes: nothing changed was left out.
    for r in &ranges {
        gpu_mirror[r.clone()].copy_from_slice(&st.bytes()[r.clone()]);
    }
    assert_eq!(gpu_mirror, st.bytes(), "the coalesced ranges missed a changed slot");
}

/// Steady-state frames allocate nothing of their own: not in a static scene, not while one prop moves.
#[test]
fn preparation_allocations_are_flat_after_warmup() {
    let mut scene = load("examples/test_lab.json");
    let (mut st, bounds, offsets) = make(&scene);
    let mut ranges = Vec::new();
    for _ in 0..3 {
        st.update_scene(&scene, 0.0, &offsets, &bounds);
        st.drain_dirty_into(&mut ranges);
    }
    let before = allocs_now();
    for f in 0..200 {
        st.update_scene(&scene, f as f32 * 0.016, &offsets, &bounds);
        st.drain_dirty_into(&mut ranges);
    }
    let per_frame = (allocs_now() - before) as f64 / 200.0;
    println!("static lab, cached preparation: {per_frame:.2} allocations/frame");
    assert!(per_frame <= PREP_STATIC_ALLOC_BUDGET, "static-scene preparation allocates {per_frame:.1}/frame (budget {PREP_STATIC_ALLOC_BUDGET})");

    // One object moves every frame (a rolling prop).
    for f in 0..3 {
        first_leaf(&mut scene.objects).position = Track::constant(Vec3::new(f as f32, 1.0, 0.0));
        st.update_scene(&scene, 0.0, &offsets, &bounds);
        st.drain_dirty_into(&mut ranges);
    }
    let before = allocs_now();
    for f in 0..200 {
        first_leaf(&mut scene.objects).position = Track::constant(Vec3::new(f as f32 * 0.01, 1.0, 0.0));
        st.update_scene(&scene, 0.0, &offsets, &bounds);
        st.drain_dirty_into(&mut ranges);
    }
    let per_frame = (allocs_now() - before) as f64 / 200.0;
    println!("static lab + one moving object, cached preparation: {per_frame:.2} allocations/frame");
    assert!(per_frame <= PREP_STATIC_ALLOC_BUDGET, "moving-object preparation allocates {per_frame:.1}/frame");
}

/// Heap allocations per frame the preparation itself may make once warm. Measured after the change: see the test output.
const PREP_STATIC_ALLOC_BUDGET: f64 = 2.0;

/// Edits to what a character or a stairs is *made from* (not just where it is) reach the uniforms: the compiled part lists are keyed by
/// value, so a changed height, look, pose track or tread dimension is recomputed rather than served stale.
#[test]
fn edits_to_characters_and_stairs_are_never_stale() {
    use red_engine2::track::Track as T;
    let mut scene = load("examples/test_lab.json");
    let (mut st, bounds, offsets) = make(&scene);
    let slots = bounds.len() * offsets.len();
    st.update_scene(&scene, 0.0, &offsets, &bounds);
    let (mut humans, mut stairs) = (0, 0);
    for round in 0..4 {
        for o in objects_of(&mut scene) {
            match &mut o.kind {
                ObjectKind::Humanoid(h) => {
                    humans += 1;
                    match round {
                        0 => h.height *= 1.2,
                        1 => h.look.skin = Vec3::new(0.2, 0.3, 0.4),
                        2 => h.pose.l_shoulder = T::constant(Vec3::new(40.0, 0.0, 10.0)),
                        _ => h.build *= 0.9,
                    }
                }
                ObjectKind::Stairs(s) => {
                    stairs += 1;
                    match round {
                        0 => s.rise *= 1.5,
                        1 => s.run += 0.5,
                        2 => s.width += 0.25,
                        _ => s.material.color = T::constant(Vec3::new(0.1, 0.9, 0.1)),
                    }
                }
                _ => {}
            }
        }
        st.update_scene(&scene, 0.0, &offsets, &bounds);
        assert_matches_reference(&st, &scene, 0.0, &offsets, slots, &format!("character/stairs edit round {round}"));
    }
    assert!(humans > 0 && stairs > 0, "the lab must contain characters and stairs for this test to mean anything ({humans} humans, {stairs} stairs)");
}

/// `material.opacity` < 1 marks a leaf as see-through: it leaves the solid and shadow draws and is listed by `blended` far to near.
#[test]
fn blended_leaves_are_listed_far_to_near_and_skipped_by_the_solid_draws() {
    use glam::Vec4;
    let json = r##"{"camera":{"position":[0,2,8],"target":[0,1,0]},"objects":[
        {"id":"near","type":"box","position":[0,1,2],"size":[1,1,1],"material":{"color":"#88ccff","opacity":0.4}},
        {"id":"solid","type":"box","position":[0,1,0],"size":[1,1,1],"material":{"color":"#888888"}},
        {"id":"far","type":"box","position":[0,1,-6],"size":[1,1,1],"material":{"color":"#88ccff","opacity":0.5}}]}"##;
    let scene = red_engine2::schema::parse_scene(json).expect("scene");
    let (mut st, bounds, offsets) = make(&scene);
    st.update_scene(&scene, 0.0, &offsets, &bounds);
    let everything = [Vec4::new(0.0, 0.0, 0.0, 1.0); 6]; // every plane passes: nothing is culled
    st.cull(&everything, Some(&everything), &[false; 3]);
    assert!(st.is_blended(0) && !st.is_blended(1) && st.is_blended(2));
    assert!(!st.main_visible(0) && st.main_visible(1) && !st.main_visible(2), "solid draw skips blended slots");
    assert!(!st.shadow_visible(0) && st.shadow_visible(1) && !st.shadow_visible(2), "blended slots cast no shadow");
    assert_eq!(st.blended(Vec3::new(0.0, 2.0, 8.0)), &[2, 0], "the far pane is drawn first, the near one last");
    assert_eq!(st.blended(Vec3::new(0.0, 2.0, -9.0)), &[0, 2], "the order follows the eye");
}

#[test]
fn opacity_defaults_to_solid_reaches_the_uniform_and_an_out_of_range_value_is_an_error_not_a_clamp() {
    let json = r##"{"camera":{"position":[0,2,8],"target":[0,1,0]},"objects":[
        {"id":"a","type":"box","material":{"color":"#ffffff"}},
        {"id":"b","type":"box","material":{"color":"#ffffff","opacity":1}},
        {"id":"c","type":"box","material":{"color":"#ffffff","opacity":0.25}}]}"##;
    let scene = red_engine2::schema::parse_scene(json).expect("scene");
    let (mut st, bounds, offsets) = make(&scene);
    st.update_scene(&scene, 0.0, &offsets, &bounds);
    let alpha = |slot: usize| f32::from_le_bytes(st.bytes()[slot * STRIDE as usize + 128 + 12..slot * STRIDE as usize + 128 + 16].try_into().unwrap());
    assert_eq!((alpha(0), alpha(1), alpha(2)), (1.0, 1.0, 0.25));
    // 7 used to be clamped to 1 without a word; now the author is told (ADR 2026-10-09-reject-bad-input)
    let bad = json.replace("\"opacity\":1}", "\"opacity\":7}");
    let Err(errors) = red_engine2::schema::parse_scene(&bad) else { panic!("an opacity of 7 is rejected") };
    assert!(errors.iter().any(|e| e.starts_with("b.material.opacity: must be between 0 and 1 (got 7)")), "{errors:?}");
}
