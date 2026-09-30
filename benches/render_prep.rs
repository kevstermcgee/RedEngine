//! CPU cost of the live renderer's per-frame object preparation: the old rebuild-everything path against `object_staging::SceneStaging`.
//!
//! No GPU is involved (this measures what the CPU does before `queue.write_buffer`; GPU execution and presentation are not measured here).
//! Run: `cargo bench --bench render_prep` (optionally `-- <scene substring>`); numbers are printed, not gated (wall-clock is machine-specific;
//! the deterministic guards are `tests/render_prep.rs`).
//!
//! Per scene it reports, per frame: median / p95 / p99 microseconds, heap allocations, and bytes handed to `write_buffer`, for
//!   `old`          the pre-cache path (fresh vectors, zeroed staging, every uniform rebuilt, every bound recomputed, everything uploaded)
//!   `cam-only`     cached, nothing in the scene changes (the camera moves)
//!   `1% moving`    cached, 1% of leaf objects get a new pose every frame (rolling props)
//!   `all moving`   cached, every leaf object gets a new pose every frame (worst case: equals a full rebuild plus change detection)

use glam::{Mat4, Vec3};
use red_engine2::object_staging::{aabb_outside_frustum, frustum_planes, local_bounds, uncached_uniforms, world_aabb, SceneStaging};
use red_engine2::render::wrap_offsets;
use red_engine2::schema::{Object, ObjectKind, Scene};
use red_engine2::track::Track;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::path::Path;
use std::time::Instant;

struct Counting;
thread_local! {
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
}
fn allocs_now() -> u64 {
    ALLOCS.with(|c| c.get())
}
// SAFETY: forwards to the system allocator unchanged; only counts calls.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        unsafe { System.realloc(p, l, n) }
    }
}
#[global_allocator]
static A: Counting = Counting;

const STRIDE: u64 = 256;

fn leaves_mut<'a>(objects: &'a mut [Object], out: &mut Vec<&'a mut Object>) {
    for o in objects.iter_mut() {
        if matches!(o.kind, ObjectKind::Group(_)) {
            if let ObjectKind::Group(c) = &mut o.kind {
                leaves_mut(c, out);
            }
        } else {
            out.push(o);
        }
    }
}

/// A synthetic scene of `n` objects: a mix of boxes, spheres and props (each prop is several leaf meshes), on a grid.
fn synthetic(n: usize) -> Scene {
    let mut objs = Vec::new();
    for i in 0..n {
        let (x, z) = ((i % 100) as f32 * 1.5, (i / 100) as f32 * 1.5);
        let body = match i % 4 {
            0 => r#""type":"box","size":[0.5,0.5,0.5]"#.to_string(),
            1 => r#""type":"sphere","radius":0.3"#.to_string(),
            2 => r#""type":"prop","prop":"crate""#.to_string(),
            _ => r#""type":"prop","prop":"chair""#.to_string(),
        };
        objs.push(format!(r##"{{"id":"o{i}",{body},"position":[{x},0.5,{z}],"material":{{"color":"#c08040"}}}}"##));
    }
    let json = format!(
        r##"{{"meta":{{"fps":30,"duration":1.0,"resolution":[640,360]}},"background":{{"sky_top":"#5a8fd6","sky_bottom":"#eaf3ff"}},
        "ambient":{{"color":"#ffffff","intensity":0.2}},"camera":{{"fov":60,"position":[0,3,-5],"target":[10,0,10]}},
        "lights":[{{"id":"sun","type":"directional","direction":[-0.5,-1,-0.35],"color":"#fff2df","intensity":1.9,"cast_shadows":true,"shadow_radius":30}}],
        "objects":[{}]}}"##,
        objs.join(",")
    );
    red_engine2::schema::parse_scene(&json).unwrap_or_else(|e| panic!("synthetic scene: {e:?}"))
}

struct Stats {
    us: Vec<f64>,
    allocs: f64,
    upload_bytes: f64,
}

fn pct(v: &[f64], p: f64) -> f64 {
    v[((v.len() as f64 - 1.0) * p).round() as usize]
}

fn report(scene: &str, mode: &str, meshes: usize, mut s: Stats) {
    s.us.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "{scene:<16} {mode:<11} meshes={meshes:<6} samples={:<5} p50={:>9.1}us p95={:>9.1}us p99={:>9.1}us  allocs/frame={:>7.1}  upload={:>9.0} B/frame",
        s.us.len(),
        pct(&s.us, 0.5),
        pct(&s.us, 0.95),
        pct(&s.us, 0.99),
        s.allocs,
        s.upload_bytes
    );
}

fn view_planes(frame: usize) -> ([glam::Vec4; 6], [glam::Vec4; 6]) {
    // A camera that moves every frame; the "light" is a fixed orthographic box over the scene.
    let eye = Vec3::new(frame as f32 * 0.01, 3.0, -5.0);
    let vp = glam::camera::rh::proj::directx::perspective(1.0, 16.0 / 9.0, 0.1, 200.0)
        * glam::camera::rh::view::look_at_mat4(eye, Vec3::new(10.0, 0.0, 10.0), Vec3::Y);
    let light = glam::camera::rh::proj::directx::orthographic(-40.0, 40.0, -40.0, 40.0, -50.0, 50.0)
        * glam::camera::rh::view::look_at_mat4(Vec3::new(20.0, 30.0, 10.0), Vec3::new(10.0, 0.0, 10.0), Vec3::Y);
    (frustum_planes(vp), frustum_planes(light))
}

fn bench_scene(name: &str, mut scene: Scene, frames: usize) {
    let bounds = local_bounds(&scene);
    let n = bounds.len();
    let offsets = wrap_offsets(&scene);
    let slots = n * offsets.len();
    let hidden = vec![false; n];

    // ---- old path ----
    let mut s = Stats { us: Vec::new(), allocs: 0.0, upload_bytes: 0.0 };
    let a0 = allocs_now();
    for f in 0..frames {
        let (cam, light) = view_planes(f);
        let t0 = Instant::now();
        let bytes = uncached_uniforms(&scene, 0.0, &offsets, STRIDE, slots);
        // The old loop also recomputed every world bound and both visibility vectors each frame.
        let mut vis = Vec::with_capacity(slots * 2);
        for i in 0..slots {
            let m = Mat4::from_cols_array(bytemuck::cast_slice::<u8, f32>(&bytes[i * STRIDE as usize..i * STRIDE as usize + 64]).try_into().unwrap());
            let (lo, hi) = bounds[i % n];
            let (c, h) = world_aabb(m, lo, hi);
            vis.push(!aabb_outside_frustum(c, h, &cam));
            vis.push(!aabb_outside_frustum(c, h, &light));
        }
        std::hint::black_box((&bytes, &vis));
        s.us.push(t0.elapsed().as_secs_f64() * 1e6);
        s.upload_bytes = bytes.len() as f64;
    }
    s.allocs = (allocs_now() - a0) as f64 / frames as f64;
    report(name, "old", n, s);

    // ---- cached paths ----
    for (mode, fraction) in [("cam-only", 0.0), ("1% moving", 0.01), ("all moving", 1.0)] {
        let mut st = SceneStaging::new(n, offsets.len(), 0, STRIDE);
        let mut ranges = Vec::new();
        let move_count = ((n as f64 * fraction).ceil() as usize).min(n);
        let mut warm = |st: &mut SceneStaging, scene: &Scene| {
            st.update_scene(scene, 0.0, &offsets, &bounds);
            st.drain_dirty_into(&mut ranges);
        };
        for _ in 0..3 {
            warm(&mut st, &scene);
        }
        let mut s = Stats { us: Vec::new(), allocs: 0.0, upload_bytes: 0.0 };
        let mut alloc_total = 0u64;
        let mut up_total = 0f64;
        for f in 0..frames {
            if move_count > 0 {
                // Scene mutation (what physics / network sync does); not timed, not counted.
                let mut ls = Vec::new();
                leaves_mut(&mut scene.objects, &mut ls);
                let step = ls.len().checked_div(move_count).unwrap_or(1).max(1);
                for (k, o) in ls.iter_mut().step_by(step).take(move_count).enumerate() {
                    o.position = Track::constant(Vec3::new(k as f32 * 0.7, 0.5 + (f % 7) as f32 * 0.01, (f % 5) as f32));
                }
            }
            let (cam, light) = view_planes(f);
            let a0 = allocs_now();
            let t0 = Instant::now();
            st.update_scene(&scene, 0.0, &offsets, &bounds);
            st.cull(&cam, Some(&light), &hidden);
            st.drain_dirty_into(&mut ranges);
            let mut up = 0usize;
            for r in &ranges {
                up += std::hint::black_box(&st.bytes()[r.clone()]).len();
            }
            s.us.push(t0.elapsed().as_secs_f64() * 1e6);
            alloc_total += allocs_now() - a0;
            up_total += up as f64;
        }
        s.allocs = alloc_total as f64 / frames as f64;
        s.upload_bytes = up_total / frames as f64;
        report(name, mode, n, s);
    }
}

fn main() {
    let filter = std::env::args().skip(1).find(|a| !a.starts_with('-'));
    let want = |name: &str| filter.as_deref().is_none_or(|f| name.contains(f));
    for map in ["test_lab", "house", "school", "prop_hunt_yard", "endless_shore"] {
        if want(map) {
            let scene = red_engine2::load_scene(Path::new(&format!("examples/{map}.json"))).expect("example scene");
            bench_scene(map, scene, 600);
        }
    }
    for n in [500usize, 2_000, 8_000] {
        let name = format!("synthetic-{n}");
        if want(&name) {
            bench_scene(&name, synthetic(n), if n >= 8_000 { 200 } else { 400 });
        }
    }
}
