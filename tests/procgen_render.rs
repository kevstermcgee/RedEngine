//! The streamed world, rendered (ADR 2026-10-04-an-endless-world): a `procgen` scene draws ground, trees and flowers around the camera wherever the camera is,
//! the picture is the same twice, and walking a long way away still gives a meadow. The test renders on whatever adapter the machine has (a software one is
//! fine) and skips itself when there is none.

#![cfg(feature = "gfx")]

use glam::Vec3;
use red_engine2::render::Renderer;
use red_engine2::track::Track;

fn scene() -> red_engine2::schema::Scene {
    let text =
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/endless_meadow.json")).expect("examples/endless_meadow.json");
    let mut s = red_engine2::schema::parse_scene(&text).expect("the example scene parses");
    s.width = 320;
    s.height = 180;
    s
}

fn render(scene: &mut red_engine2::schema::Scene, renderer: &mut Renderer, eye: Vec3, hour: f32) -> Vec<u8> {
    scene.camera.position = Track::constant(eye);
    scene.camera.target = Track::constant(eye + Vec3::new(0.0, -0.05, -10.0));
    let t = scene.clock.as_ref().expect("a clock").t_for_hour(hour);
    renderer.render_frame(scene, t)
}

fn mean(rgb: &[u8], w: usize, x0: usize, x1: usize, y0: usize, y1: usize) -> [f32; 3] {
    let (mut s, mut n) = ([0.0f32; 3], 0.0);
    for y in y0..y1 {
        for x in x0..x1 {
            for c in 0..3 {
                s[c] += rgb[(y * w + x) * 3 + c] as f32;
            }
            n += 1.0;
        }
    }
    s.map(|v| v / n)
}

fn colours(rgb: &[u8], w: usize, y0: usize, y1: usize) -> usize {
    let mut set = std::collections::HashSet::new();
    for y in y0..y1 {
        for x in 0..w {
            let i = (y * w + x) * 3;
            set.insert((rgb[i] / 4, rgb[i + 1] / 4, rgb[i + 2] / 4));
        }
    }
    set.len()
}

#[test]
fn a_generated_world_renders_a_meadow_near_and_far_from_the_start() {
    let mut s = scene();
    let Ok(mut r) = Renderer::new(&s) else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let (w, h) = (320usize, 180usize);
    for (name, eye) in
        [("the start", Vec3::new(0.0, 1.6, 0.0)), ("50 km away", open_spot(7, (50_000.0, -30_000.0))), ("400 km away", open_spot(7, (400_000.0, 250_000.0)))]
    {
        let img = render(&mut s, &mut r, eye, 11.0);
        let sky = mean(&img, w, 0, w, 0, h / 8);
        let ground = mean(&img, w, 0, w, h * 3 / 4, h);
        assert!(sky[2] > sky[0] + 20.0, "{name}: the sky should be blue: {sky:?}");
        assert!(ground[1] > ground[0] && ground[1] > ground[2], "{name}: the ground should be green: {ground:?}");
        assert!(ground[1] > 40.0, "{name}: the ground should be lit: {ground:?}");
        let detail = colours(&img, w, h / 2, h);
        assert!(detail > 250, "{name}: the ground is flat: {detail} colours");
    }
}

#[test]
fn the_same_place_and_hour_render_the_same_pixels_and_the_hour_matters() {
    let mut s = scene();
    let Ok(mut r) = Renderer::new(&s) else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let eye = open_spot(7, (120.0, 80.0));
    let a = render(&mut s, &mut r, eye, 10.0);
    let b = render(&mut s, &mut r, eye, 10.0);
    assert_eq!(a, b, "a still frame must not depend on when it was drawn");
    let night = render(&mut s, &mut r, eye, 0.5);
    let (day_light, night_light) = (mean(&a, 320, 0, 320, 90, 180), mean(&night, 320, 0, 320, 90, 180));
    assert!(day_light[1] > night_light[1] * 2.0, "midnight should be darker than morning: {day_light:?} vs {night_light:?}");
}

#[test]
fn the_wind_moves_the_grass_but_not_the_ground() {
    // Without a clock the light is fixed, so the only thing that changes between two moments is the wind.
    let mut s = scene();
    s.clock = None;
    let Ok(mut r) = Renderer::new(&s) else {
        eprintln!("no GPU adapter: skipping");
        return;
    };
    let eye = open_spot(7, (10.0, 10.0));
    s.camera.position = Track::constant(eye);
    s.camera.target = Track::constant(eye + Vec3::new(0.0, -0.3, -4.0));
    let a = r.render_frame(&s, 0.0);
    let again = r.render_frame(&s, 0.0);
    assert_eq!(a, again);
    let b = r.render_frame(&s, 2.3);
    let differing = a.chunks(3).zip(b.chunks(3)).filter(|(p, q)| p != q).count();
    assert!(differing > 100, "the grass should have moved between two moments: {differing} pixels differ");
    assert!(differing < 320 * 180 / 2, "but the world as a whole should not: {differing} pixels differ");
}

/// A spot near `near` (x, z) with open sky: no tree within the chunk and its neighbours, so a camera there is not under a crown.
fn open_spot(seed: u32, near: (f64, f64)) -> Vec3 {
    use red_engine2::procgen::{ChunkId, Config, Kind, World};
    let world = World::new(Config { seed, ..Config::default() });
    let start = ChunkId::at(near.0, near.1);
    for r in 0..40i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs().max(dz.abs()) != r {
                    continue;
                }
                let id = ChunkId { x: start.x + dx, z: start.z + dz };
                let clear = (-1..=1).all(|oz| (-1..=1).all(|ox| world.plants(ChunkId { x: id.x + ox, z: id.z + oz }, Kind::Tree).is_empty()));
                if clear {
                    let (x, z) = id.centre();
                    return Vec3::new(x as f32, world.height(x, z) + 1.6, z as f32);
                }
            }
        }
    }
    panic!("no open ground within 40 chunks of {near:?}");
}
