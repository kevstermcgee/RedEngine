//! The asset catalogue: every placeable thing in one searchable list — the built-in `prop`s
//! (Rust-built, proper collision) and the JSON `prefab`s from `assets/*.json` — with tags, real
//! measured sizes, parameters, and a paste-ready usage snippet. `catalog --sheet out.png`
//! renders a labelled contact sheet so an AI (or a human) can *see* what exists before building.

use super::font::{draw_text, text_width};
use super::world::MapWorld;
use crate::prefabs;
use crate::props::{local_bounds, PropKind};
use glam::Vec3;
use image::{Rgb, RgbImage};
use serde_json::{json, Value};
use std::path::Path;

/// One catalogue entry: a prop or a prefab, with tags, description, params and size.
#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    /// `prop` (built into the engine) or `prefab` (JSON in `assets/`, or the scene itself).
    pub kind: &'static str,
    pub category: String,
    pub tags: Vec<String>,
    pub desc: String,
    /// `floor` / `wall` — where the origin sits (see `prefabs` docs).
    pub mount: String,
    pub min: Vec3,
    pub max: Vec3,
    /// Number of solid collision boxes (0 = walk-through).
    pub solid_parts: usize,
    pub params: Vec<(String, Value, String)>,
    pub variant_of: Option<String>,
    /// Set if the entry failed to expand/parse (a catalogue bug).
    pub error: Option<String>,
}

impl Entry {
    /// The entry's real-world bounding size in metres.
    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }
}

/// `(prop name, tags, one-line description)`. `tests::every_prop_has_catalogue_info` keeps this
/// in step with `PropKind::ALL`.
const PROP_INFO: &[(&str, &str, &str)] = &[
    ("crate", "storage container wood", "wooden shipping crate"),
    ("barrel", "storage container industrial", "steel-banded barrel"),
    ("traffic_cone", "road outdoor small", "orange traffic cone"),
    ("box_stack", "storage cardboard", "stack of cardboard boxes"),
    ("chair", "furniture chair seating", "plain dining chair"),
    ("trash_can", "utility bin", "kerbside trash can"),
    ("vending_machine", "store appliance electronics", "snack/drink vending machine"),
    ("bench", "furniture seating outdoor", "park bench"),
    ("fire_extinguisher", "safety small wall", "red fire extinguisher (stands on the floor)"),
    ("filing_cabinet", "office storage furniture", "two-drawer metal filing cabinet"),
    ("potted_plant", "plant decor", "leafy plant in a pot"),
    ("bookshelf", "furniture storage books", "tall bookshelf"),
    ("sofa", "furniture seating living", "three-seat sofa"),
    ("bed", "furniture bedroom", "double bed with pillows"),
    ("dining_table", "furniture table kitchen", "dining table"),
    ("tv", "electronics living", "flat-screen TV on a stand"),
    ("kitchen_counter", "furniture kitchen counter", "kitchen counter run with cabinets"),
    ("refrigerator", "appliance kitchen refrigerator", "two-door refrigerator"),
    ("stove", "appliance kitchen", "cooker with hobs"),
    ("sink", "kitchen bathroom plumbing", "sink with cabinet"),
    ("toilet", "bathroom plumbing", "toilet"),
    ("bathtub", "bathroom plumbing", "bathtub"),
    ("washer_dryer", "appliance laundry", "washer/dryer"),
    ("mailbox", "outdoor street", "post-mounted mailbox"),
    ("fence_section", "outdoor fence", "one fence panel (prefer the `fence` macro for runs)"),
    ("coffee_table", "furniture table living", "low coffee table"),
    ("nightstand", "furniture bedroom storage", "bedside table"),
    ("wardrobe", "furniture bedroom storage", "tall wardrobe"),
    ("desk", "furniture office table", "writing desk"),
    ("rug", "decor floor soft", "floor rug (walk-through)"),
    ("armchair", "furniture seating living", "upholstered armchair"),
    ("tree_oak", "plant tree outdoor", "broad oak tree (color = foliage)"),
    ("tree_pine", "plant tree outdoor", "pine tree (color = foliage)"),
    ("bush", "plant outdoor garden", "round bush (color = foliage)"),
    ("flower_patch", "plant outdoor garden", "flower bed, walk-through (color = blooms)"),
    ("hedge", "plant outdoor garden fence", "clipped hedge segment"),
    ("boulder", "outdoor rock", "large rock"),
    ("grill", "outdoor kitchen", "barbecue grill"),
    ("picnic_table", "outdoor furniture table seating", "picnic table with benches"),
];

fn prop_entry(k: PropKind) -> Entry {
    let (mn, mx) = local_bounds(k);
    let info = PROP_INFO.iter().find(|(n, _, _)| *n == k.name());
    let solid = match crate::props::collision(k) {
        crate::props::Collision::None => 0,
        _ => 1,
    };
    Entry {
        name: k.name().to_string(),
        kind: "prop",
        category: "prop".to_string(),
        tags: info.map(|i| i.1.split_whitespace().map(str::to_string).collect()).unwrap_or_default(),
        desc: info.map(|i| i.2.to_string()).unwrap_or_default(),
        mount: "floor".to_string(),
        min: mn,
        max: mx,
        solid_parts: solid,
        params: vec![("material.color".to_string(), json!("#rrggbb"), "tints the body (foliage/blooms for plants)".to_string())],
        variant_of: None,
        error: None,
    }
}

fn measure(def: &prefabs::PrefabDef) -> Entry {
    let mut e = Entry {
        name: def.name.clone(),
        kind: "prefab",
        category: def.category.clone(),
        tags: def.tags.clone(),
        desc: def.desc.clone(),
        mount: def.mount.clone(),
        min: Vec3::ZERO,
        max: Vec3::ZERO,
        solid_parts: 0,
        params: def.params.iter().map(|p| (p.name.clone(), p.default.clone(), p.desc.clone())).collect(),
        variant_of: def.extends.clone(),
        error: None,
    };
    let scene = prefabs::preview_scene(&def.name, None);
    match MapWorld::from_text(&scene.to_string(), Path::new("<catalog>")) {
        Ok(w) => {
            let mut mn = Vec3::splat(f32::INFINITY);
            let mut mx = Vec3::splat(f32::NEG_INFINITY);
            for it in &w.items {
                mn = mn.min(it.min);
                mx = mx.max(it.max);
                if it.is_solid() && it.kind == super::world::ItemKind::Box {
                    e.solid_parts += 1;
                }
            }
            if mn.x.is_finite() {
                e.min = mn;
                e.max = mx;
            }
        }
        Err(errs) => e.error = Some(errs.join("; ")),
    }
    e
}

/// Every catalogue entry: props first, then the built-in prefabs.
pub fn entries() -> Vec<Entry> {
    let mut out: Vec<Entry> = PropKind::ALL.iter().map(|k| prop_entry(*k)).collect();
    let (lib, _) = prefabs::builtin();
    out.extend(lib.defs.iter().map(measure));
    out
}

/// Looks an entry up by exact name.
pub fn find<'a>(all: &'a [Entry], name: &str) -> Option<&'a Entry> {
    all.iter().find(|e| e.name == name)
}

/// Filters by free-text query (every word must match the name, a tag, the category, or the
/// description), plus optional exact tag / category / kind filters.
pub fn filter<'a>(all: &'a [Entry], query: Option<&str>, tag: Option<&str>, category: Option<&str>, kind: Option<&str>) -> Vec<&'a Entry> {
    let words: Vec<String> = query.map(|q| q.to_lowercase().split_whitespace().map(str::to_string).collect()).unwrap_or_default();
    all.iter()
        .filter(|e| tag.is_none_or(|t| e.tags.iter().any(|x| x == t)))
        .filter(|e| category.is_none_or(|c| e.category == c))
        .filter(|e| kind.is_none_or(|k| e.kind == k))
        .filter(|e| {
            words.iter().all(|w| {
                e.name.contains(w.as_str()) || e.category.contains(w.as_str()) || e.tags.iter().any(|t| t.contains(w.as_str())) || e.desc.to_lowercase().contains(w.as_str())
            })
        })
        .collect()
}

fn dims(e: &Entry) -> String {
    let s = e.size();
    format!("{:.2}x{:.2}x{:.2}", s.x, s.y, s.z)
}

/// A paste-ready `add` JSON snippet for placing the entry.
pub fn usage_snippet(e: &Entry) -> String {
    let y = if e.mount == "wall" { 1.5 } else { 0.0 };
    let mut o = if e.kind == "prop" {
        json!({"id": format!("{}_1", e.name), "type": "prop", "prop": e.name, "position": [0, y, 0]})
    } else {
        json!({"id": format!("{}_1", e.name), "type": "prefab", "prefab": e.name, "position": [0, y, 0]})
    };
    if e.kind == "prefab" && !e.params.is_empty() {
        let p: serde_json::Map<String, Value> = e.params.iter().take(3).map(|(k, v, _)| (k.clone(), v.clone())).collect();
        o["params"] = Value::Object(p);
    }
    serde_json::to_string(&o).unwrap()
}

/// Compact listing. More than `long_over` rows collapse to grouped names unless `long` is set.
pub fn render_list(rows: &[&Entry], json_out: bool, long: bool) -> String {
    if json_out {
        let v: Vec<Value> = rows.iter().map(|e| entry_json(e)).collect();
        return serde_json::to_string_pretty(&v).unwrap();
    }
    if rows.is_empty() {
        return "no matches (try fewer words; `red_engine2 catalog` lists everything)\n".to_string();
    }
    if rows.len() > 30 && !long {
        let mut out = String::new();
        let mut cats: Vec<&str> = rows.iter().map(|e| e.category.as_str()).collect();
        cats.sort();
        cats.dedup();
        for c in cats {
            let names: Vec<&str> = rows.iter().filter(|e| e.category == c).map(|e| e.name.as_str()).collect();
            out.push_str(&format!("{c} ({}): {}\n", names.len(), names.join(" ")));
        }
        out.push_str(&format!(
            "\n{} entries. `catalog <words>` filters, `catalog <name>` shows params + a paste-ready snippet, `--long` prints sizes/tags, `--sheet out.png` renders them.\n",
            rows.len()
        ));
        return out;
    }
    let nw = rows.iter().map(|e| e.name.len()).max().unwrap_or(4).clamp(4, 26);
    let mut out = format!("{:<nw$}  {:<6}  {:<9}  {:<16}  tags\n", "name", "kind", "category", "size w x h x d", nw = nw);
    for e in rows {
        out.push_str(&format!("{:<nw$}  {:<6}  {:<9}  {:<16}  {}\n", e.name, e.kind, e.category, dims(e), e.tags.join(" "), nw = nw));
    }
    out.push_str(&format!("{} entries\n", rows.len()));
    out
}

/// The entry as JSON (for `--json` and the MCP server).
pub fn entry_json(e: &Entry) -> Value {
    let s = e.size();
    json!({
        "name": e.name, "kind": e.kind, "category": e.category, "tags": e.tags, "desc": e.desc, "mount": e.mount,
        "size": [s.x, s.y, s.z], "min": [e.min.x, e.min.y, e.min.z], "max": [e.max.x, e.max.y, e.max.z],
        "solid_parts": e.solid_parts, "variant_of": e.variant_of,
        "params": e.params.iter().map(|(k, v, d)| json!({"name": k, "default": v, "desc": d})).collect::<Vec<_>>(),
        "usage": usage_snippet(e), "error": e.error,
    })
}

/// Full detail view of one entry (params, size, snippet), as text or JSON.
pub fn render_detail(e: &Entry, json_out: bool) -> String {
    if json_out {
        return serde_json::to_string_pretty(&entry_json(e)).unwrap();
    }
    let s = e.size();
    let mut out = format!("{} ({}{}){}\n", e.name, e.kind, e.variant_of.as_ref().map(|b| format!(", variant of {b}")).unwrap_or_default(), if e.desc.is_empty() { String::new() } else { format!(" — {}", e.desc) });
    out.push_str(&format!("size {:.2} x {:.2} x {:.2} m (w x h x d)   bounds y {:.2}..{:.2}   mount: {}\n", s.x, s.y, s.z, e.min.y, e.max.y, e.mount));
    out.push_str(&format!("tags: {}\n", e.tags.join(" ")));
    out.push_str(&format!(
        "collision: {}\n",
        match (e.kind, e.solid_parts) {
            (_, 0) => "walk-through".to_string(),
            ("prop", _) => "one collider around the whole prop".to_string(),
            (_, n) => format!("{n} box part(s) collide individually (spheres/cylinders/cones don't)"),
        }
    ));
    if !e.params.is_empty() {
        out.push_str("params:\n");
        for (k, v, d) in &e.params {
            out.push_str(&format!("  {k} = {v}{}\n", if d.is_empty() { String::new() } else { format!("   # {d}") }));
        }
    }
    out.push_str(&format!("use:  red_engine2 add <scene> '{}'\n", usage_snippet(e)));
    if let Some(err) = &e.error {
        out.push_str(&format!("!! CATALOGUE ERROR: {err}\n"));
    }
    out
}

/// Renders `rows` as a labelled contact sheet: one 3/4-view tile per asset, name on top, real
/// size at the bottom. Tiny assets are scaled up for framing only (the label shows true size).
pub fn sheet(rows: &[&Entry], out: &Path, cols: u32, tile: (u32, u32)) -> Result<(), String> {
    if rows.is_empty() {
        return Err("nothing to render (the filter matched no assets)".to_string());
    }
    let spacing = 14.0f32;
    let cols = cols.clamp(1, rows.len() as u32);
    let mut objects: Vec<Value> = Vec::new();
    // (name, size label, eye, target)
    let mut views: Vec<(String, String, Vec3, Vec3)> = Vec::new();
    for (i, e) in rows.iter().enumerate() {
        let (cx, cz) = ((i as u32 % cols) as f32 * spacing, (i as u32 / cols) as f32 * spacing);
        let s = e.size();
        let maxdim = s.x.max(s.y).max(s.z).max(0.01);
        let k = (1.2 / maxdim).clamp(1.0, 14.0);
        let mut inst = if e.kind == "prop" {
            json!({"id": format!("a{i}"), "type": "prop", "prop": e.name, "material": {"color": "#b08a5a", "roughness": 0.7}})
        } else {
            json!({"id": format!("a{i}"), "type": "prefab", "prefab": e.name})
        };
        let lift = if e.mount == "wall" { 1.6 } else { 0.0 };
        inst["position"] = json!([cx, lift, cz]);
        inst["scale"] = json!(k);
        objects.push(inst);
        if e.mount == "wall" {
            objects.push(json!({"id": format!("w{i}"), "type": "box", "size": [(s.x * k + 1.2).max(2.0), 3.2, 0.1],
                "position": [cx, 1.6, cz - 0.05], "material": {"color": "#d8d0c0", "roughness": 0.9}}));
        }
        let center = Vec3::new(cx, lift, cz) + Vec3::new((e.min.x + e.max.x) * 0.5, (e.min.y + e.max.y) * 0.5, (e.min.z + e.max.z) * 0.5) * k;
        let radius = (s * k).length() * 0.5;
        let fov = 34.0f32;
        let dist = radius / (fov.to_radians() * 0.5).sin() * 1.08 + 0.1;
        let dir = Vec3::new(0.62, 0.5, 1.0).normalize();
        views.push((e.name.clone(), format!("{:.2} x {:.2} x {:.2} m", s.x, s.y, s.z), center + dir * dist, center));
    }
    objects.push(json!({"id": "ground", "type": "plane", "size": [cols as f32 * spacing + 40.0, (rows.len() as f32 / cols as f32).ceil() * spacing + 40.0],
        "position": [(cols as f32 - 1.0) * spacing * 0.5, -0.002, (rows.len() as f32 / cols as f32).ceil() * spacing * 0.5],
        "material": {"color": "#8f9a86", "roughness": 0.95}}));
    let scene_json = json!({
        "meta": { "fps": 30, "duration": 1, "resolution": [tile.0, tile.1] },
        "background": { "sky_top": "#9dbbdc", "sky_bottom": "#eef2f6" },
        "ambient": { "intensity": 0.55 },
        "camera": { "fov": 34, "near": 0.05, "position": [0, 2, 6], "target": [0, 0.5, 0] },
        "post": { "ao": 0.9, "outline": 0.55 },
        "lights": [
            { "id": "sun", "type": "directional", "direction": [-0.5, -1.0, -0.6], "intensity": 2.2, "color": "#fff4e0" },
            { "id": "fill", "type": "directional", "direction": [0.6, -0.4, -0.5], "intensity": 0.7, "color": "#a8c0e8" }
        ],
        "objects": objects,
    });
    let mut scene = crate::schema::parse_scene(&scene_json.to_string()).map_err(|e| e.join("\n"))?;
    let mut renderer = crate::render::Renderer::new(&scene).map_err(|e| e.to_string())?;
    let (tw, th) = (scene.width, scene.height);
    let rows_n = (rows.len() as u32).div_ceil(cols);
    let mut sheet = RgbImage::from_pixel(tw * cols, th * rows_n, Rgb([12, 14, 18]));
    for (i, (name, size, eye, at)) in views.iter().enumerate() {
        scene.camera.position = crate::track::Track::constant(*eye);
        scene.camera.target = crate::track::Track::constant(*at);
        let rgb = renderer.render_frame(&scene, 0.0);
        let img = RgbImage::from_raw(tw, th, rgb).ok_or("frame size mismatch")?;
        let (ox, oy) = ((i as u32 % cols) * tw, (i as u32 / cols) * th);
        for y in 0..th {
            for x in 0..tw {
                sheet.put_pixel(ox + x, oy + y, *img.get_pixel(x, y));
            }
        }
        for x in 0..tw {
            sheet.put_pixel(ox + x, oy, Rgb([12, 14, 18]));
        }
        for y in 0..th {
            sheet.put_pixel(ox, oy + y, Rgb([12, 14, 18]));
        }
        let plate = |sheet: &mut RgbImage, y0: u32, w: u32, h: u32| {
            for y in y0..y0 + h {
                for x in 4..(4 + w).min(tw) {
                    let p = sheet.get_pixel_mut(ox + x, oy + y);
                    for c in 0..3 {
                        p.0[c] /= 3;
                    }
                }
            }
        };
        plate(&mut sheet, 4, text_width(name, 2) as u32 + 10, 20);
        draw_text(&mut sheet, (ox + 9) as i32, (oy + 8) as i32, name, 2, Rgb([255, 255, 255]), None);
        plate(&mut sheet, th - 22, text_width(size, 1) as u32 + 10, 18);
        draw_text(&mut sheet, (ox + 9) as i32, (oy + th - 17) as i32, size, 1, Rgb([200, 210, 225]), None);
    }
    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    sheet.save(out).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_prop_has_catalogue_info() {
        for k in PropKind::ALL {
            assert!(PROP_INFO.iter().any(|(n, t, d)| *n == k.name() && !t.is_empty() && !d.is_empty()), "prop '{}' has no PROP_INFO entry (tags + description)", k.name());
        }
        for (n, _, _) in PROP_INFO {
            assert!(PropKind::from_name(n).is_some(), "PROP_INFO names unknown prop '{n}'");
        }
    }

    #[test]
    fn every_prefab_expands_measures_and_rests_on_its_mount() {
        let all = entries();
        let mut bad = Vec::new();
        for e in all.iter().filter(|e| e.kind == "prefab") {
            if let Some(err) = &e.error {
                bad.push(format!("{}: {err}", e.name));
                continue;
            }
            let s = e.size();
            if s.max_element() <= 0.005 {
                bad.push(format!("{}: renders nothing (size {s:?})", e.name));
            }
            if s.max_element() > 8.0 {
                bad.push(format!("{}: absurdly large {s:?}", e.name));
            }
            if e.mount == "floor" && (e.min.y < -0.005 || e.min.y > 0.03) {
                bad.push(format!("{}: floor prefab must start at y=0 (its lowest point is y={:.3}) — origin is the middle of the base", e.name, e.min.y));
            }
            if e.tags.is_empty() || e.desc.is_empty() {
                bad.push(format!("{}: needs tags and a desc", e.name));
            }
            if !["floor", "wall"].contains(&e.mount.as_str()) {
                bad.push(format!("{}: unknown mount '{}'", e.name, e.mount));
            }
        }
        assert!(bad.is_empty(), "catalogue problems:\n{}", bad.join("\n"));
    }

    #[test]
    fn catalogue_names_are_unique_across_props_and_prefabs() {
        let all = entries();
        let mut names: Vec<&str> = all.iter().map(|e| e.name.as_str()).collect();
        names.sort();
        let n = names.len();
        names.dedup();
        assert_eq!(n, names.len(), "a prefab shares a name with a prop or another prefab");
    }

    #[test]
    fn search_matches_words_across_name_tags_and_desc() {
        let all = entries();
        let chairs = filter(&all, Some("chair"), None, None, None);
        assert!(chairs.iter().any(|e| e.name == "chair_folding") && chairs.iter().any(|e| e.name == "chair"));
        let apples = filter(&all, Some("apple"), None, None, Some("prefab"));
        assert!(apples.len() >= 3);
    }
}
