//! The time-of-day sky, rendered (ADR 2026-10-04-a-day-and-night-cycle): noon is bright and blue, midnight is dark with stars and no stray colour cast, sunset is warm at
//! the horizon, and the same moment renders the same pixels. These guard the sky shader, which cannot be checked any other way than by looking at its output.
//!
//! The test renders on whatever adapter the machine has (a software one is fine) and skips itself when there is none.

#![cfg(feature = "gfx")]

use glam::Vec3;
use red_engine2::render::Renderer;
use red_engine2::track::Track;

fn scene() -> red_engine2::schema::Scene {
    let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/day_cycle.json")).expect("examples/day_cycle.json");
    let mut s = red_engine2::schema::parse_scene(&text).expect("the example scene parses");
    s.width = 320;
    s.height = 180;
    s
}

/// Renders `scene` looking along `at` from the scene's eye at clock `hour`, as RGB bytes; `None` when there is no GPU adapter.
fn frame(scene: &mut red_engine2::schema::Scene, renderer: &mut Renderer, hour: f32, at: Vec3) -> Vec<u8> {
    let t = scene.clock.as_ref().expect("a clock").t_for_hour(hour);
    scene.camera.target = Track::constant(Vec3::new(0.0, 1.6, 0.0) + at);
    renderer.render_frame(scene, t)
}

fn region_mean(rgb: &[u8], w: usize, x0: usize, x1: usize, y0: usize, y1: usize) -> [f32; 3] {
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

#[test]
fn the_sky_is_blue_at_noon_starry_and_uncast_at_midnight_and_warm_at_sunset() {
    let mut sc = scene();
    let Ok(mut r) = Renderer::new(&sc) else {
        eprintln!("no GPU adapter on this machine: the sky render test is skipped");
        return;
    };
    let w = sc.width as usize;
    // Looking level at the horizon, the upper third of the frame is sky.
    let level = Vec3::new(0.0, 0.0, -10.0);
    let noon = frame(&mut sc, &mut r, 12.0, level);
    let sky = region_mean(&noon, w, 0, w, 0, 50);
    assert!(sky[2] > sky[0] + 25.0 && sky[1] > 100.0, "a blue daytime sky: {sky:?}");

    // Midnight, looking up into the sky: dark, bluish, and speckled with stars.
    let night = frame(&mut sc, &mut r, 0.0, Vec3::new(0.0, 6.0, -10.0));
    let mean = region_mean(&night, w, 0, w, 0, 130);
    assert!(mean.iter().sum::<f32>() / 3.0 < 70.0, "night is dark: {mean:?}");
    assert!(mean[2] >= mean[0], "no red cast over the night sky (the sun's halo must not reach across it): {mean:?}");
    let bright_points = (0..130 * w).filter(|i| (0..3).map(|c| night[i * 3 + c] as u32).sum::<u32>() > 250).count();
    assert!(bright_points >= 8, "stars are visible: {bright_points} bright pixels");
    let none = {
        let mut dark = scene();
        dark.clock.as_mut().unwrap().stars = 0.0;
        dark.clock.as_mut().unwrap().moon = false;
        let mut r2 = Renderer::new(&dark).expect("the same adapter");
        frame(&mut dark, &mut r2, 0.0, Vec3::new(0.0, 6.0, -10.0))
    };
    let lit = |img: &[u8]| (0..130 * w).filter(|i| (0..3).map(|c| img[i * 3 + c] as u32).sum::<u32>() > 250).count();
    assert!(lit(&none) < bright_points / 3, "turning the stars off removes them: {} vs {}", lit(&none), bright_points);

    // Sunset: looking toward the sun, the sky just above the horizon is warm and brighter than the sky overhead.
    let sun_az = sc.clock.as_ref().unwrap().state(sc.clock.as_ref().unwrap().t_for_hour(17.9), 0).sun_dir;
    let toward_sun = Vec3::new(sun_az.x, 0.12, sun_az.z) * 20.0;
    let dusk = frame(&mut sc, &mut r, 17.9, toward_sun);
    let low = region_mean(&dusk, w, 0, w, 60, 86);
    let high = region_mean(&dusk, w, 0, w, 0, 16);
    assert!(low[0] > low[2] + 20.0, "a warm horizon at sunset: {low:?}");
    assert!(low.iter().sum::<f32>() > high.iter().sum::<f32>(), "brightest at the horizon: {low:?} vs {high:?}");

    // The same moment renders the same pixels.
    assert_eq!(frame(&mut sc, &mut r, 0.0, Vec3::new(0.0, 6.0, -10.0)), night, "rendering is deterministic");
}
