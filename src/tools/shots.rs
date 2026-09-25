//! Rendering helpers for reviewing a map: `frame` with a free camera / cutaway / hidden objects,
//! and `tour`, a labelled contact sheet with a view of every zone and every floor.
//!
//! These drive the ordinary offline [`Renderer`] by rewriting the loaded [`Scene`]'s camera and
//! object list before rendering — no separate rendering path, so what you see here is what
//! `render`/`frame` produce.

use super::edit::glob;
#[cfg(feature = "gfx")]
use super::font::draw_text;
use super::world::{ItemKind, MapWorld};
#[cfg(feature = "gfx")]
use crate::render::Renderer;
use crate::track::Track;
use glam::{Vec2, Vec3};
#[cfg(feature = "gfx")]
use image::{Rgb, RgbImage};
use std::collections::HashSet;
use std::path::Path;

/// Options for `frame`: free camera, fov, hidden object globs, cutaway height.
#[derive(Clone, Default)]
pub struct FrameOpts {
    pub eye: Option<Vec3>,
    pub at: Option<Vec3>,
    pub fov: Option<f32>,
    /// Glob patterns of top-level object ids to leave out of the render.
    pub hide: Vec<String>,
    /// Leave out every top-level object whose lowest point is at or above this height (peel
    /// off a roof / upper floors to see into the level below).
    pub cut_above: Option<f32>,
    pub size: Option<(u32, u32)>,
    pub t: f32,
}

/// Loads `path` and applies the frame options, returning a scene ready for `Renderer::new`.
pub fn prepare(world: MapWorld, opts: &FrameOpts) -> crate::schema::Scene {
    let mut scene = world.scene;
    if !opts.hide.is_empty() || opts.cut_above.is_some() {
        let mut lowest: std::collections::HashMap<&str, f32> = std::collections::HashMap::new();
        for it in &world.items {
            let e = lowest.entry(it.top_id.as_str()).or_insert(f32::INFINITY);
            *e = e.min(it.min.y);
        }
        let hidden: HashSet<String> = scene
            .objects
            .iter()
            .filter(|o| opts.hide.iter().any(|p| glob(p, &o.id)) || opts.cut_above.is_some_and(|c| lowest.get(o.id.as_str()).is_some_and(|y| *y >= c)))
            .map(|o| o.id.clone())
            .collect();
        scene.objects.retain(|o| !hidden.contains(&o.id));
    }
    if let Some(e) = opts.eye {
        scene.camera.position = Track::constant(e);
    }
    if let Some(a) = opts.at {
        scene.camera.target = Track::constant(a);
    }
    if let Some(f) = opts.fov {
        scene.camera.fov = Track::constant(f);
    }
    if let Some((w, h)) = opts.size {
        scene.width = w.max(2) + w % 2;
        scene.height = h.max(2) + h % 2;
    }
    scene
}

/// Renders one frame of a scene with the given options to `out`.
#[cfg(feature = "gfx")]
pub fn render_frame(path: &Path, out: &Path, opts: &FrameOpts) -> Result<(), String> {
    let world = super::world::load_or_report(path)?;
    let scene = prepare(world, opts);
    let mut renderer = Renderer::new(&scene).map_err(|e| e.to_string())?;
    let rgb = renderer.render_frame(&scene, opts.t);
    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    image::save_buffer(out, &rgb, scene.width, scene.height, image::ColorType::Rgb8).map_err(|e| e.to_string())
}

/// One view in a tour.
#[derive(Clone)]
pub struct View {
    pub label: String,
    pub eye: Vec3,
    pub at: Vec3,
    pub fov: f32,
    pub cut_above: Option<f32>,
}

/// Lowest ceiling slab above floor height `h` (a big, thin box starting >= 2 m up), if any.
fn ceiling_above(world: &MapWorld, h: f32) -> Option<f32> {
    world
        .items
        .iter()
        .filter(|i| matches!(i.kind, ItemKind::Box) && (i.max.y - i.min.y) <= 0.6 && i.min.y >= h + 2.0 && (i.max.x - i.min.x) * (i.max.z - i.min.z) >= 9.0)
        .map(|i| i.min.y)
        .fold(None, |a, y| Some(a.map_or(y, |m: f32| m.min(y))))
}

/// Automatic views: an exterior overview, a cutaway per floor, and two opposite-corner views of
/// every zone (needs `zones` in the scene JSON for the per-room shots).
pub fn auto_views(world: &MapWorld) -> Vec<View> {
    let reach = super::reach::compute(world, &super::reach::ReachParams { cell: 0.1, ..Default::default() });
    auto_views_with(world, &reach)
}

/// The reachable floor cell (at height `y`, inside `min..max`) nearest to `want` — so a tour
/// camera starts somewhere a player could actually stand, never inside a wardrobe.
fn nearest_standable(reach: &super::reach::Reach, want: Vec2, y: f32, min: Vec2, max: Vec2) -> Vec2 {
    let mut best = (f32::INFINITY, want);
    for (i, lv) in reach.levels.iter().enumerate() {
        if !lv.iter().any(|l| (l - y).abs() < 0.3) {
            continue;
        }
        let p = reach.cell_center(i);
        if p.x < min.x || p.x > max.x || p.y < min.y || p.y > max.y {
            continue;
        }
        let d = (p - want).length_squared();
        if d < best.0 {
            best = (d, p);
        }
    }
    best.1
}

/// The automatic `tour` views: exterior, per-floor cutaways and each zone from two corners.
pub fn auto_views_with(world: &MapWorld, reach: &super::reach::Reach) -> Vec<View> {
    let (bmin, bmax) = world.solid_bounds();
    let center = (bmin + bmax) * 0.5;
    let ext = bmax - bmin;
    let span = ext.x.max(ext.y);
    let mut views = Vec::new();

    // Floors: distinct zone heights, or just the ground.
    let mut floors: Vec<f32> = world.zones.iter().map(|z| z.y).collect();
    floors.sort_by(|a, b| a.partial_cmp(b).unwrap());
    floors.dedup_by(|a, b| (*a - *b).abs() < 0.3);
    if floors.is_empty() {
        floors.push(0.0);
    }

    views.push(View {
        label: "overview".into(),
        eye: Vec3::new(center.x - ext.x * 0.55, span * 0.55 + 3.0, bmin.y - span * 0.75),
        at: Vec3::new(center.x, 1.5, center.y),
        fov: 55.0,
        cut_above: None,
    });
    for &h in &floors {
        let cut = ceiling_above(world, h).map(|c| c - 0.01);
        views.push(View {
            label: format!("floor y={h:.1} cutaway"),
            eye: Vec3::new(center.x, h + span * 0.85, center.y - span * 0.42),
            at: Vec3::new(center.x, h + 0.4, center.y),
            fov: 50.0,
            cut_above: cut,
        });
    }
    for z in &world.zones {
        let c = (z.min + z.max) * 0.5;
        let inset = 0.6;
        let corners = [Vec2::new(z.min.x + inset, z.min.y + inset), Vec2::new(z.max.x - inset, z.max.y - inset)];
        for (i, corner) in corners.iter().enumerate() {
            let stand = nearest_standable(reach, *corner, z.y, z.min, z.max);
            views.push(View {
                label: format!("{} ({})", z.id, if i == 0 { "a" } else { "b" }),
                eye: Vec3::new(stand.x, z.y + 1.6, stand.y),
                at: Vec3::new(c.x, z.y + 1.0, c.y),
                fov: 80.0,
                cut_above: None,
            });
        }
    }
    views
}

#[cfg(feature = "gfx")]
fn downsample2(img: &RgbImage) -> RgbImage {
    let (w, h) = (img.width() / 2, img.height() / 2);
    RgbImage::from_fn(w, h, |x, y| {
        let mut acc = [0u32; 3];
        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let p = img.get_pixel(x * 2 + dx, y * 2 + dy);
            for i in 0..3 {
                acc[i] += p.0[i] as u32;
            }
        }
        Rgb([(acc[0] / 4) as u8, (acc[1] / 4) as u8, (acc[2] / 4) as u8])
    })
}

/// Renders every view and lays them out in a labelled grid (`cols` columns), each tile at
/// half the scene's resolution.
#[cfg(feature = "gfx")]
pub fn tour(path: &Path, out: &Path, views: Option<Vec<View>>, cols: u32, only: Option<&str>) -> Result<Vec<String>, String> {
    let base = super::world::load_or_report(path)?;
    let mut views = views.unwrap_or_else(|| auto_views(&base));
    if let Some(pat) = only {
        views.retain(|v| glob(&if pat.contains('*') { pat.to_string() } else { format!("*{pat}*") }, &v.label));
    }
    if views.is_empty() {
        return Err("no views to render (does the scene define `zones`? try --view or no filter)".to_string());
    }
    // One renderer per distinct cutaway (GPU resources are sized to the object list).
    let mut tiles: Vec<(String, RgbImage)> = Vec::new();
    let mut cache: Vec<(Option<u32>, crate::schema::Scene, Renderer)> = Vec::new();
    for v in &views {
        let key = v.cut_above.map(|c| (c * 100.0) as u32);
        let idx = match cache.iter().position(|(k, _, _)| *k == key) {
            Some(i) => i,
            None => {
                let world = super::world::load_or_report(path)?;
                let scene = prepare(world, &FrameOpts { cut_above: v.cut_above, ..Default::default() });
                let renderer = Renderer::new(&scene).map_err(|e| e.to_string())?;
                cache.push((key, scene, renderer));
                cache.len() - 1
            }
        };
        let (_, scene, renderer) = &mut cache[idx];
        scene.camera.position = Track::constant(v.eye);
        scene.camera.target = Track::constant(v.at);
        scene.camera.fov = Track::constant(v.fov);
        let rgb = renderer.render_frame(scene, 0.0);
        let img = RgbImage::from_raw(scene.width, scene.height, rgb).ok_or("frame size mismatch")?;
        tiles.push((v.label.clone(), downsample2(&img)));
    }
    let (tw, th) = (tiles[0].1.width(), tiles[0].1.height());
    let cols = cols.clamp(1, tiles.len() as u32);
    let rows = (tiles.len() as u32).div_ceil(cols);
    let mut sheet = RgbImage::from_pixel(tw * cols, th * rows, Rgb([12, 14, 18]));
    for (i, (label, tile)) in tiles.iter().enumerate() {
        let (cx, cy) = ((i as u32 % cols) * tw, (i as u32 / cols) * th);
        for y in 0..th {
            for x in 0..tw {
                sheet.put_pixel(cx + x, cy + y, *tile.get_pixel(x, y));
            }
        }
        // 1px separators and a label plate.
        for x in 0..tw {
            sheet.put_pixel(cx + x, cy, Rgb([12, 14, 18]));
        }
        for y in 0..th {
            sheet.put_pixel(cx, cy + y, Rgb([12, 14, 18]));
        }
        let plate = super::font::text_width(label, 2) as u32 + 10;
        for y in 4..24 {
            for x in 4..(4 + plate).min(tw) {
                let p = sheet.get_pixel_mut(cx + x, cy + y);
                for c in 0..3 {
                    p.0[c] /= 3;
                }
            }
        }
        draw_text(&mut sheet, (cx + 9) as i32, (cy + 8) as i32, label, 2, Rgb([255, 255, 255]), None);
    }
    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    sheet.save(out).map_err(|e| e.to_string())?;
    Ok(views.iter().map(|v| v.label.clone()).collect())
}

/// This build has no renderer (`--no-default-features`): `frame`/`tour`/`render`/golden views are unavailable.
#[cfg(not(feature = "gfx"))]
pub fn render_frame(_path: &Path, _out: &Path, _opts: &FrameOpts) -> Result<(), String> {
    Err(super::NO_GFX.to_string())
}

/// See [`render_frame`]: unavailable without the `gfx` feature.
#[cfg(not(feature = "gfx"))]
pub fn tour(_path: &Path, _out: &Path, _views: Option<Vec<View>>, _cols: u32, _only: Option<&str>) -> Result<Vec<String>, String> {
    Err(super::NO_GFX.to_string())
}
