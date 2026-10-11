//! `red_engine2 museum SCENE`: what a museum scene's rooms hold, pictures of its pieces and of the worlds inside its paintings, and its plan.
//!
//! * With no picture asked for it prints one room (`--room X,Z`, default the first): its wing, size, plan, every piece with its plaque, where the room's key is
//!   hidden and the curator's note.
//! * `--shot OUT.png` draws the room from its doorway; with `--look N` from in front of piece N; with `--inside N[/I/J]` from inside painting N (and
//!   painting I inside that, ...), standing where a player arrives. `--t` is the scene time (paintings move).
//! * `--map OUT.png --explore R` draws the pause menu's plan with every room within R rooms of `--room` discovered and every other key found.

use crate::museum::exhibit::{Kind, PocketPath};
use crate::museum::{Museum, RoomId};
use glam::Vec3;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// What to show.
pub struct Args {
    pub scene: PathBuf,
    pub seed: Option<u32>,
    pub room: (i32, i32),
    pub shot: Option<PathBuf>,
    pub look: Option<u8>,
    pub inside: Option<String>,
    pub map: Option<PathBuf>,
    pub explore: u32,
    pub t: f32,
    /// Draw every style of the collection on one wall (`--styles OUT.png`).
    pub styles: Option<PathBuf>,
}

fn visit(scene: &Path, seed: Option<u32>) -> Result<(crate::schema::Scene, Museum), String> {
    let mut s = crate::load_scene(scene).map_err(|e| format!("{}: {}", scene.display(), e.join("; ")))?;
    let spec = s.museum.clone().ok_or_else(|| format!("{}: has no `museum` block (see `describe scene`)", scene.display()))?;
    let seed = seed.unwrap_or_else(|| spec.seed.value(None));
    if let Some(m) = s.museum.as_mut() {
        std::sync::Arc::make_mut(m).seed = crate::atmosphere::LaunchSeed::Fixed(seed);
    }
    let spec = s.museum.clone().unwrap_or(spec);
    Ok((s, Museum::new(spec, seed, None)))
}

/// Parses `N` or `N/I/J`.
fn path_of(room: RoomId, text: &str) -> Result<PocketPath, String> {
    let mut parts = text.split('/').map(|p| p.trim().parse::<u8>().map_err(|_| format!("--inside: `{text}` is not N or N/I/J (piece numbers)")));
    let exhibit = parts.next().ok_or("--inside: empty")??;
    let nest = parts.collect::<Result<Vec<u8>, String>>()?;
    Ok(PocketPath { room, exhibit, nest })
}

/// Runs the command; returns what to print.
pub fn run(a: &Args) -> Result<String, String> {
    let (scene, mut m) = visit(&a.scene, a.seed)?;
    let room = RoomId::new(a.room.0, a.room.1);
    let k = m.contents(room);
    let spec = m.spec.clone();
    let c = &spec.collection;
    let theme = &c.themes[k.room.theme];
    let mut out = String::new();
    let _ = writeln!(out, "Gallery {} ({}, {}) in {} ({}), seed {}", room.number(), room.x, room.z, theme.wing, theme.name, m.layout.seed);
    let doors: Vec<&str> = crate::museum::Side::ALL
        .iter()
        .filter(|s| k.room.has_door(**s))
        .map(|s| match s {
            crate::museum::Side::North => "north",
            crate::museum::Side::East => "east",
            crate::museum::Side::South => "south",
            crate::museum::Side::West => "west",
        })
        .collect();
    let _ = writeln!(
        out,
        "  {} x {} m, {} m high, {:?}, {:?} floor{}{}; doors: {}",
        k.room.size.x,
        k.room.size.y,
        k.room.height,
        k.room.plan,
        k.room.floor,
        if k.room.skylight { ", skylight" } else { "" },
        if k.room.beams { ", beams" } else { "" },
        doors.join(", ")
    );
    let route = if k.key.route.is_empty() {
        String::new()
    } else {
        format!(" then painting {}", k.key.route.iter().map(|r| r.to_string()).collect::<Vec<_>>().join(" then "))
    };
    let _ = writeln!(out, "  key: piece {} ({}), {} deep{}", k.key.exhibit, k.exhibits[k.key.exhibit as usize].kind.word(), k.key.depth, route);
    let _ = writeln!(out, "  curator's note: {}", k.note.text);
    for e in &k.exhibits {
        let what = match (e.kind, e.family) {
            (Kind::Sculpture | Kind::Installation, Some(f)) => {
                format!("{} ({}, {})", e.kind.word(), f.name(), c.materials.get(e.art.style as usize - 1).map_or("?", String::as_str))
            }
            _ => format!("{} ({}, {:?} frame)", e.kind.word(), c.styles.get(e.art.style as usize - 1).map_or("?", String::as_str), e.frame),
        };
        let _ = writeln!(
            out,
            "  [{}] {}{} — \u{201c}{}\u{201d}, {}, {}, {}",
            e.index,
            what,
            if e.masterpiece { " MASTERPIECE" } else { "" },
            e.plaque.title,
            e.plaque.artist,
            e.plaque.year,
            e.plaque.medium
        );
        let _ = writeln!(out, "      {}", e.plaque.story);
    }
    if let Some(text) = &a.inside {
        let path = path_of(room, text)?;
        let p = crate::museum::exhibit::pocket(&m.layout, c, &k, &path)
            .ok_or_else(|| format!("--inside {text}: no such painting world (piece {} is not a painting, or it goes too deep)", path.exhibit))?;
        let _ = writeln!(
            out,
            "inside \u{201c}{}\u{201d} ({} deep): {} ground, radius {:.0} m, {} painting(s){}",
            p.title,
            path.depth(),
            format!("{:?}", p.ground).to_lowercase(),
            p.radius,
            p.frames.len(),
            if p.key.is_some() { ", THE KEY is here" } else { "" }
        );
        for f in &p.frames {
            let _ = writeln!(
                out,
                "  [{}] {} on a {:?}{} — \u{201c}{}\u{201d}, {}",
                f.index,
                c.styles.get(f.art.style as usize - 1).map_or("?", String::as_str),
                f.stand,
                if f.on_route { " (on the way to the key)" } else { "" },
                f.plaque.title,
                f.plaque.artist
            );
        }
    }
    if let Some(shot) = &a.shot {
        shoot(&scene, &mut m, room, a, shot)?;
        let _ = writeln!(out, "wrote {}", shot.display());
    }
    let style_names: Vec<String> = c.styles.iter().enumerate().map(|(i, s)| format!("{} {s}", i + 1)).collect();
    if let Some(shot) = &a.shot {
        shoot(&scene, &mut m, room, a, shot)?;
        let _ = writeln!(out, "wrote {}", shot.display());
    }
    if let Some(sheet) = &a.styles {
        shoot_sheet(&scene, &m, a, sheet)?;
        let names = style_names;
        let _ = writeln!(out, "styles, left to right, top to bottom: {}", names.join(", "));
        let _ = writeln!(out, "wrote {}", sheet.display());
    }
    if let Some(map) = &a.map {
        draw_map(&mut m, room, a.explore, map)?;
        let _ = writeln!(out, "wrote {}", map.display());
    }
    Ok(out)
}

#[cfg(feature = "gfx")]
fn shoot(scene: &crate::schema::Scene, m: &mut Museum, room: RoomId, a: &Args, out: &Path) -> Result<(), String> {
    use crate::track::Track;
    let k = m.contents(room);
    let (eye, at, inside) = if let Some(text) = &a.inside {
        let path = path_of(room, text)?;
        let p = crate::museum::exhibit::pocket(&m.layout, &m.spec.collection, &k, &path).ok_or("--inside: no such painting world")?;
        let eye = crate::museum::state::POCKET_ORIGIN + p.spawn + Vec3::Y * 1.6;
        (eye, eye + p.spawn_facing * 5.0 + Vec3::Y * 0.1, Some(path))
    } else if let Some(i) = a.look {
        let e = k.exhibits.get(i as usize).ok_or_else(|| format!("--look {i}: the room has {} pieces", k.exhibits.len()))?;
        let dist = (e.size.x * 0.8 + 1.2).max(2.4);
        let stand = Vec3::new(e.at.x, 1.6, e.at.z) + e.facing * dist;
        (stand, e.focus(), None)
    } else {
        let start = m.start();
        let eye = start.at + Vec3::Y * 1.65 - Vec3::new(-start.yaw.sin(), 0.0, -start.yaw.cos()) * 4.0;
        (eye, room.centre() + Vec3::Y * 1.9, None)
    };
    let mut scene_for = crate::load_scene(&a.scene).map_err(|e| e.join("; "))?;
    scene_for.museum = scene.museum.clone();
    scene_for.camera.position = Track::constant(eye);
    scene_for.camera.target = Track::constant(at);
    let mut renderer = crate::render::Renderer::new(&scene_for).map_err(|e| e.to_string())?;
    if let (Some(path), Some(v)) = (inside, renderer.museum_visit()) {
        v.enter_now(&path).ok_or("--inside: could not enter")?;
    }
    let rgb = renderer.render_frame(&scene_for, a.t);
    image::save_buffer(out, &rgb, scene_for.width, scene_for.height, image::ColorType::Rgb8).map_err(|e| e.to_string())
}

#[cfg(feature = "gfx")]
fn shoot_sheet(scene: &crate::schema::Scene, m: &Museum, a: &Args, out: &Path) -> Result<(), String> {
    use crate::track::Track;
    let cols = 7;
    let sheet = crate::museum::build::style_sheet(&m.spec.collection, cols, a.seed.unwrap_or(1) as f32);
    let mut scene_for = crate::load_scene(&a.scene).map_err(|e| e.join("; "))?;
    scene_for.museum = scene.museum.clone();
    scene_for.width = 1600;
    scene_for.height = 900;
    let rows = m.spec.collection.styles.len().div_ceil(cols) as f32;
    let eye = Vec3::new(0.0, 1.6, (rows * 1.45).max(cols as f32 * 1.95 * 0.5) * 1.25);
    scene_for.camera.position = Track::constant(eye);
    scene_for.camera.target = Track::constant(Vec3::new(0.0, 1.6, 0.0));
    let mut renderer = crate::render::Renderer::new(&scene_for).map_err(|e| e.to_string())?;
    if let Some(v) = renderer.museum_visit() {
        v.stage(sheet);
    }
    let rgb = renderer.render_frame(&scene_for, a.t);
    image::save_buffer(out, &rgb, scene_for.width, scene_for.height, image::ColorType::Rgb8).map_err(|e| e.to_string())
}

#[cfg(not(feature = "gfx"))]
fn shoot_sheet(_: &crate::schema::Scene, _: &Museum, _: &Args, _: &Path) -> Result<(), String> {
    Err(super::NO_GFX.to_string())
}

#[cfg(not(feature = "gfx"))]
fn shoot(_: &crate::schema::Scene, _: &mut Museum, _: RoomId, _: &Args, _: &Path) -> Result<(), String> {
    Err(super::NO_GFX.to_string())
}

#[cfg(feature = "render")]
fn draw_map(m: &mut Museum, room: RoomId, explore: u32, out: &Path) -> Result<(), String> {
    let r = explore as i32;
    for x in -r..=r {
        for z in -r..=r {
            let id = RoomId::new(room.x + x, room.z + z);
            m.progress.visited.insert(id);
            if (x + z).rem_euclid(3) != 0 {
                m.grant_key(id);
            }
        }
    }
    let (w, h) = (1280u32, 800u32);
    let mut px = vec![0u8; (w * h * 4) as usize];
    let mut img = crate::ui::serif::Image { w, h, px: &mut px };
    let view = crate::museum::map::MapView::around(room.centre(), h);
    crate::museum::map::paint(m, &crate::ui::serif::Serif::load(), &mut img, &view, (room.centre(), 0.3), 0.0);
    image::save_buffer(out, &px, w, h, image::ColorType::Rgba8).map_err(|e| e.to_string())
}

#[cfg(not(feature = "render"))]
fn draw_map(_: &mut Museum, _: RoomId, _: u32, _: &Path) -> Result<(), String> {
    Err(super::NO_GFX.to_string())
}
