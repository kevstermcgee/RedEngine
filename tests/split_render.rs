//! Split-screen rendering (docs/analysis/2026-10-04-split-screen-survey-and-plan.md): every player's view is drawn by the one renderer into its rectangle of the window,
//! with its own HUD, the gutters stay dark, a single view is exactly what the old single-player path draws, and four views cost about what four views must.
//! Renders on whatever adapter the machine has (a software one is fine) and skips itself when there is none.

#![cfg(feature = "gfx")]

use glam::Vec3;
use red_engine2::app::{Offscreen, OffscreenSplit, ViewCamera};
use red_engine2::procgen::{ChunkId, Config, Kind, World};
use red_engine2::split_gpu::PlayerView;
use red_engine2::splitscreen::vertical_fov;

/// The tests share one GPU and one of them measures time: they take turns.
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn scene() -> red_engine2::schema::Scene {
    let text =
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/endless_meadow.json")).expect("examples/endless_meadow.json");
    red_engine2::schema::parse_scene(&text).expect("the example scene parses")
}

/// A spot with open sky near the start (no tree within the chunk and its neighbours).
fn open_spot() -> Vec3 {
    let world = World::new(Config { seed: 7, ..Config::default() });
    for r in 0..40i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs().max(dz.abs()) != r {
                    continue;
                }
                let id = ChunkId { x: dx, z: dz };
                if (-1..=1).all(|oz| (-1..=1).all(|ox| world.plants(ChunkId { x: id.x + ox, z: id.z + oz }, Kind::Tree).is_empty())) {
                    let (x, z) = id.centre();
                    return Vec3::new(x as f32, world.height(x, z) + 1.4, z as f32);
                }
            }
        }
    }
    panic!("no open ground");
}

/// A camera at `eye` looking along compass direction `k` (0 north, 1 east, 2 south, 3 west), with a field of view fitted to a view of `aspect`.
fn camera(eye: Vec3, k: usize, window_aspect: f32, aspect: f32) -> ViewCamera {
    let dir = [Vec3::NEG_Z, Vec3::X, Vec3::Z, Vec3::NEG_X][k % 4];
    let mut c = ViewCamera::look_at(eye, eye + dir * 10.0 + Vec3::Y * -0.4);
    c.far = 600.0;
    c.fov_deg = vertical_fov(70.0, window_aspect, aspect);
    c
}

fn region(px: &[u8], w: u32, r: red_engine2::splitscreen::Rect) -> Vec<u8> {
    let mut out = Vec::new();
    for y in r.y..r.y + r.h {
        out.extend_from_slice(&px[((y * w + r.x) * 4) as usize..((y * w + r.x + r.w) * 4) as usize]);
    }
    out
}

fn mean(px: &[u8]) -> f32 {
    px.chunks(4).map(|p| p[0] as f32 + p[1] as f32 + p[2] as f32).sum::<f32>() / (px.len() / 4).max(1) as f32 / 3.0
}

#[test]
fn four_views_are_four_different_pictures_in_their_own_rectangles_with_dark_gutters() {
    let _turn = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = scene();
    s.clock.as_mut().unwrap().start = 0.42;
    let Ok(mut split) = OffscreenSplit::new(&s, 640, 360, 4, 6) else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let l = split.layout().clone();
    let eye = open_spot();
    let views: Vec<PlayerView> = (0..4).map(|k| PlayerView { camera: camera(eye, k, 640.0 / 360.0, l.views[k].aspect()), layers: None, hidden: &[] }).collect();
    let img = split.render(&s, 0.0, &views, &[]).unwrap();
    assert_eq!(img.len(), 640 * 360 * 4);
    let regions: Vec<Vec<u8>> = l.views.iter().map(|r| region(&img, 640, *r)).collect();
    for (i, r) in regions.iter().enumerate() {
        assert!(mean(r) > 30.0, "view {i} is lit and drawn: {}", mean(r));
        for (j, o) in regions.iter().enumerate().skip(i + 1) {
            assert_ne!(r, o, "views {i} and {j} look different directions");
        }
    }
    // The gutters between the views are the dark clear colour.
    let gutter_x = l.views[0].x + l.views[0].w;
    for y in (0..360).step_by(20) {
        for x in gutter_x..gutter_x + 6 {
            let p = &img[((y * 640 + x) * 4) as usize..][..3];
            assert!(p.iter().all(|c| *c < 30), "gutter pixel ({x},{y}) = {p:?}");
        }
    }
    let again = split.render(&s, 0.0, &views, &[]).unwrap();
    assert_eq!(img, again, "the same frame is the same pixels");
}

#[test]
fn one_view_is_exactly_what_the_single_player_path_draws() {
    let _turn = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = scene();
    s.clock.as_mut().unwrap().start = 0.42;
    let (Ok(mut split), Ok(mut single)) = (OffscreenSplit::new(&s, 320, 180, 1, 0), Offscreen::new(&s, 320, 180)) else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let cam = camera(open_spot(), 1, 320.0 / 180.0, 320.0 / 180.0);
    let a = split.render(&s, 0.0, &[PlayerView { camera: cam, layers: None, hidden: &[] }], &[]).unwrap();
    // The single-player path streams the world in a few chunks a frame: draw until it has all arrived (two frames in a row the same).
    let mut b = single.render(&s, 0.0, &cam, std::iter::empty::<&str>(), None).unwrap();
    for _ in 0..400 {
        let next = single.render(&s, 0.0, &cam, std::iter::empty::<&str>(), None).unwrap();
        let settled = next == b;
        b = next;
        if settled {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // The blit round-trips every pixel through the sRGB texture (a value may move by one), and the far edge of a streamed world may be a chunk apart in detail: well under 1% of values differ.
    let worst = a.iter().zip(&b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
    let differing = a.iter().zip(&b).filter(|(x, y)| x != y).count();
    assert!(differing < a.len() / 100, "a split of one is the old path (worst difference {worst}, {differing} of {} values differ)", a.len());
}

#[test]
fn a_players_hud_appears_only_in_their_own_view() {
    let _turn = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = scene();
    s.clock.as_mut().unwrap().start = 0.42;
    let Ok(mut split) = OffscreenSplit::new(&s, 640, 360, 4, 6) else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let l = split.layout().clone();
    let (vw, vh) = l.size;
    // A solid magenta panel over the whole of player 3's view, nothing for the others.
    let mut hud = vec![0u8; (vw * vh * 4) as usize];
    for p in hud.chunks_mut(4) {
        p.copy_from_slice(&[255, 0, 255, 255]);
    }
    let eye = open_spot();
    let views: Vec<PlayerView> = (0..4).map(|k| PlayerView { camera: camera(eye, k, 640.0 / 360.0, l.views[k].aspect()), layers: None, hidden: &[] }).collect();
    let img = split.render(&s, 0.0, &views, &[None, None, Some(hud), None]).unwrap();
    for (i, r) in l.views.iter().enumerate() {
        let magenta = region(&img, 640, *r).chunks(4).filter(|p| p[0] > 240 && p[1] < 20 && p[2] > 240).count();
        if i == 2 {
            assert!(magenta as u32 > vw * vh * 9 / 10, "player 3's HUD covers their view: {magenta}");
        } else {
            assert_eq!(magenta, 0, "player {} must not see player 3's HUD", i + 1);
        }
    }
}

#[test]
fn four_views_cost_four_views_and_the_composition_is_free() {
    let _turn = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = scene();
    s.clock.as_mut().unwrap().start = 0.42;
    let Ok(mut four) = OffscreenSplit::new(&s, 960, 540, 4, 0) else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    // The same view size drawn once: what one of the four costs on its own.
    let mut one = OffscreenSplit::new(&s, 480, 270, 1, 0).unwrap();
    let eye = open_spot();
    let l4 = four.layout().clone();
    assert_eq!(l4.size, (480, 270));
    let v4: Vec<PlayerView> = (0..4).map(|k| PlayerView { camera: camera(eye, k, 960.0 / 540.0, l4.views[k].aspect()), layers: None, hidden: &[] }).collect();
    let v1 = [PlayerView { camera: camera(eye, 0, 960.0 / 540.0, 480.0 / 270.0), layers: None, hidden: &[] }];
    // Warm up (pipelines, streaming), then take the best of several frames: time is only ever noisy upward.
    let best = |f: &mut dyn FnMut()| {
        f();
        (0..6)
            .map(|_| {
                let t = std::time::Instant::now();
                f();
                t.elapsed().as_secs_f64()
            })
            .fold(f64::MAX, f64::min)
    };
    let t4 = best(&mut || {
        four.render(&s, 0.0, &v4, &[]).unwrap();
    });
    let t1 = best(&mut || {
        one.render(&s, 0.0, &v1, &[]).unwrap();
    });
    eprintln!("one view {:.0} ms, four views {:.0} ms ({:.2}x); the last view drew {:?}", t1 * 1000.0, t4 * 1000.0, t4 / t1, four.last_draw_stats());
    // Four views are four times one view plus a blit each: the bound leaves room for a noisy machine but not for a hidden extra pass per view.
    assert!(t4 < t1 * 4.7, "four views cost {:.2}x one: {:.0} ms against {:.0} ms", t4 / t1, t4 * 1000.0, t1 * 1000.0);
}

#[test]
fn a_crowded_screen_draws_less_far_so_the_whole_frame_stays_inside_the_triangle_budget() {
    let _turn = GPU.lock().unwrap_or_else(|e| e.into_inner());
    use red_engine2::splitscreen::view_distance;
    assert!(view_distance(1) > view_distance(2) && view_distance(2) > view_distance(3) && view_distance(3) == view_distance(4));
    let mut s = scene();
    s.clock.as_mut().unwrap().start = 0.42;
    let Ok(mut four) = OffscreenSplit::new(&s, 960, 540, 4, 0) else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let eye = open_spot();
    let l4 = four.layout().clone();
    let views: Vec<PlayerView> =
        (0..4).map(|k| PlayerView { camera: camera(eye, k, 960.0 / 540.0, l4.views[k].aspect()), layers: None, hidden: &[] }).collect();
    let mut total = |distance: f32| {
        four.set_view_distance(distance);
        let mut tris = 0;
        // Draw each view alone to count what it submits.
        for v in &views {
            four.render(&s, 0.0, std::slice::from_ref(v), &[]).unwrap();
            tris += four.last_draw_stats().map_or(0, |d| d.tris);
        }
        tris
    };
    let full = total(view_distance(1));
    let crowded = total(view_distance(4));
    eprintln!("four views at {} m: {full} triangles; at {} m: {crowded}", view_distance(1), view_distance(4));
    assert!(crowded < full, "drawing less far draws less: {crowded} against {full}");
    assert!(crowded < 3_000_000, "four views stay under three million triangles: {crowded}");
}

#[test]
#[ignore = "a measurement, not a check: cargo test --test split_render where_the_time_goes -- --ignored --nocapture"]
fn where_the_time_goes() {
    let mut s = scene();
    s.clock.as_mut().unwrap().start = 0.42;
    let eye = open_spot();
    for (name, w, h, n) in [
        ("1 view 960x540", 960, 540, 1),
        ("1 view 480x270", 480, 270, 1),
        ("1 view 240x136", 240, 136, 1),
        ("2 views side by side 960x540", 960, 540, 2),
        ("4 views 960x540", 960, 540, 4),
    ] {
        let Ok(mut split) = OffscreenSplit::new(&s, w, h, n, 0) else { return };
        let l = split.layout().clone();
        let views: Vec<PlayerView> =
            (0..n).map(|k| PlayerView { camera: camera(eye, k, w as f32 / h as f32, l.views[k].aspect()), layers: None, hidden: &[] }).collect();
        split.render(&s, 0.0, &views, &[]).unwrap();
        let best = (0..6)
            .map(|_| {
                let t = std::time::Instant::now();
                split.render(&s, 0.0, &views, &[]).unwrap();
                t.elapsed().as_secs_f64() * 1000.0
            })
            .fold(f64::MAX, f64::min);
        eprintln!("{name}: {best:.0} ms (views {:?}, last view drew {:?})", l.size, split.last_draw_stats());
    }
}
