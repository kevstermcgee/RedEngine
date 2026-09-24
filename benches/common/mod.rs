//! Shared bench fixtures: synthetic maps with a known number of loose props, so a number means the
//! same thing every run and on every machine.

use red_engine2::schema::Scene;

/// A large floor with `n` loose props on a grid (a mix of crates, barrels and apples), the way a
/// stocked store shelf or a school's clutter is mostly small loose things nobody touches.
pub fn synthetic_scene(n: usize) -> Scene {
    let side = (n as f32).sqrt().ceil() as usize;
    let spacing = 1.6f32;
    let half = side as f32 * spacing * 0.5;
    let mut objs = String::new();
    for i in 0..n {
        let (gx, gz) = ((i % side) as f32 * spacing - half, (i / side) as f32 * spacing - half);
        let obj = match i % 3 {
            0 => format!(r##"{{"id":"crate_{i}","type":"prop","prop":"crate","position":[{gx},0,{gz}],"material":{{"color":"#a07040"}}}}"##),
            1 => format!(r##"{{"id":"barrel_{i}","type":"prop","prop":"barrel","position":[{gx},0,{gz}],"material":{{"color":"#3a6ea5"}}}}"##),
            _ => format!(r##"{{"id":"apple_{i}","type":"prefab","prefab":"apple_red","position":[{gx},0,{gz}]}}"##),
        };
        objs.push_str(&obj);
        objs.push(',');
    }
    let text = format!(
        r##"{{"camera":{{"position":[0,1.7,{z}],"target":[0,1,0]}},"objects":[{objs}
            {{"id":"floor","type":"box","position":[0,-0.1,0],"size":[{w},0.2,{w}],"material":{{"color":"#888888"}}}}]}}"##,
        z = half + 5.0,
        w = half * 2.0 + 20.0
    );
    red_engine2::schema::parse_scene(&text).unwrap_or_else(|e| panic!("synthetic scene: {e:?}"))
}
