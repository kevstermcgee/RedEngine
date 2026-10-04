//! Cascaded sun shadows, rendered (src/shadow.rs): a tall pillar close to the camera and another far from it (beyond the middle map, inside the far one) both cast a
//! shadow on the ground, and switching shadows off removes exactly those. Renders on whatever adapter the machine has (a software one is fine); skips itself when none.

#![cfg(feature = "gfx")]

use red_engine2::render::Renderer;

fn scene(shadows: bool) -> red_engine2::schema::Scene {
    // Looking straight down from 90 m at x = 25: the near pillar is at x = 28, the far one 40 m to its right. The shadow follows the camera with radius 20, so the
    // maps cover 6 m (near, which carries the small plants), 20 m (middle) and 80 m (far).
    let text = format!(
        r##"{{
  "meta": {{ "fps": 30, "duration": 1.0, "resolution": [640, 360] }},
  "background": {{ "sky_top": "#5a8fd6", "sky_bottom": "#eaf3ff" }},
  "ambient": {{ "color": "#ffffff", "intensity": 0.15 }},
  "camera": {{ "fov": 60, "position": [25, 90, 0.01], "target": [25, 0, 0] }},
  "lights": [ {{ "id": "sun", "type": "directional", "direction": [-0.5, -1, -0.2], "color": "#ffffff", "intensity": 2.0,
                 "cast_shadows": {shadows}, "shadow_radius": 20, "shadow_follow": true }} ],
  "objects": [
    {{ "id": "ground", "type": "plane", "size": [400, 400], "position": [25, 0, 0], "material": {{ "color": "#9a9a9a", "roughness": 0.9 }} }},
    {{ "id": "near", "type": "box", "size": [1, 10, 1], "position": [28, 5, 0], "material": {{ "color": "#cc4444" }} }},
    {{ "id": "far", "type": "box", "size": [1, 10, 1], "position": [65, 5, 0], "material": {{ "color": "#cc4444" }} }}
  ]
}}"##
    );
    red_engine2::schema::parse_scene(&text).expect("the test scene parses")
}

fn luminance(rgb: &[u8], i: usize) -> f32 {
    (rgb[i * 3] as f32 + rgb[i * 3 + 1] as f32 + rgb[i * 3 + 2] as f32) / 3.0
}

/// The pixels that are darker with shadows than without, as `(column, row)`.
fn shadowed(lit: &[u8], shaded: &[u8], w: usize) -> Vec<(usize, usize)> {
    (0..lit.len() / 3).filter(|&i| luminance(lit, i) - luminance(shaded, i) > 25.0).map(|i| (i % w, i / w)).collect()
}

#[test]
fn a_near_pillar_and_a_far_pillar_both_cast_a_shadow_that_the_unshadowed_scene_lacks() {
    let (with, without) = (scene(true), scene(false));
    let (Ok(mut a), Ok(mut b)) = (Renderer::new(&with), Renderer::new(&without)) else {
        eprintln!("no GPU adapter on this machine: the shadow render test is skipped");
        return;
    };
    let shaded = a.render_frame(&with, 0.0);
    let lit = b.render_frame(&without, 0.0);
    let w = with.width as usize;
    let dark = shadowed(&lit, &shaded, w);
    // The camera looks at x = 25 across about 104 m: about 3.5 pixels a metre. The near pillar is 3 m right of centre, the far one 40 m (140 pixels).
    let centre = w as f32 / 2.0;
    let near = dark.iter().filter(|(x, _)| ((*x as f32) - centre).abs() < 70.0).count();
    let far = dark.iter().filter(|(x, _)| ((*x as f32) - centre - 150.0).abs() < 40.0).count();
    assert!(near > 40, "the pillar by the camera casts a shadow: {near} darker pixels near the middle");
    assert!(far > 40, "the pillar 40 m away casts one too, from the far map: {far} darker pixels around 40 m to the right");
    assert!(dark.len() < lit.len() / 3 / 4, "shadows are patches, not a blackout: {} of {} pixels", dark.len(), lit.len() / 3);
}
