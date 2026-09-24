//! Read-only inspection commands: `ls` (what's in the scene, where), `info` (everything about
//! one object and what surrounds it), `props` (the prop library's dimensions and collision).

use super::edit::glob;
use super::lint::Finding;
use super::world::{Item, ItemKind, MapWorld};
use crate::props::{collision, collision_box, local_bounds, prop_parts, Collision, PropKind};
use glam::Vec3;
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn fmt3(v: Vec3) -> String {
    format!("{:.2},{:.2},{:.2}", v.x, v.y, v.z)
}

/// One row per top-level object with the combined bounds of all its pieces.
struct Row {
    id: String,
    kind: String,
    pieces: usize,
    min: Vec3,
    max: Vec3,
    origin: Vec3,
}

fn rows(world: &MapWorld, all: bool) -> Vec<Row> {
    let mut map: BTreeMap<String, Row> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    for it in &world.items {
        let key = if all { it.id.clone() } else { it.top_id.clone() };
        let kind = if all || it.top_id == it.id { it.kind.label() } else { "group".to_string() };
        let macro_kind = if !all && it.top_id != it.id {
            world
                .raw
                .get("objects")
                .and_then(Value::as_array)
                .and_then(|a| a.iter().find(|o| o.get("id").and_then(Value::as_str) == Some(&it.top_id)))
                .map(|o| match o.get("type").and_then(Value::as_str) {
                    Some("prefab") => format!("prefab:{}", o.get("prefab").and_then(Value::as_str).unwrap_or("?")),
                    Some(t) => t.to_string(),
                    None => "?".to_string(),
                })
        } else {
            None
        };
        let kind = macro_kind.unwrap_or(kind);
        let e = map.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            Row { id: key, kind, pieces: 0, min: Vec3::splat(f32::INFINITY), max: Vec3::splat(f32::NEG_INFINITY), origin: it.origin }
        });
        e.pieces += 1;
        e.min = e.min.min(it.min);
        e.max = e.max.max(it.max);
    }
    order.into_iter().filter_map(|k| map.remove(&k)).collect()
}

/// Text (or JSON) listing of the scene's objects with world bounds, filtered by id substring and kind.
pub fn list_objects(world: &MapWorld, filter: Option<&str>, kind: Option<&str>, all: bool, json_out: bool) -> String {
    let mut rs = rows(world, all);
    if let Some(f) = filter {
        let pat = if f.contains('*') { f.to_string() } else { format!("*{f}*") };
        rs.retain(|r| glob(&pat, &r.id));
    }
    if let Some(k) = kind {
        rs.retain(|r| r.kind == k || r.kind.ends_with(&format!(":{k}")));
    }
    if json_out {
        let v: Vec<Value> = rs
            .iter()
            .map(|r| json!({"id": r.id, "type": r.kind, "pieces": r.pieces, "min": [r.min.x, r.min.y, r.min.z], "max": [r.max.x, r.max.y, r.max.z], "origin": [r.origin.x, r.origin.y, r.origin.z]}))
            .collect();
        return serde_json::to_string_pretty(&v).unwrap();
    }
    let idw = rs.iter().map(|r| r.id.len()).max().unwrap_or(2).clamp(2, 34);
    let kw = rs.iter().map(|r| r.kind.len()).max().unwrap_or(4).clamp(4, 24);
    let mut out = format!("{:<idw$}  {:<kw$}  {:>3}  {:<22}  {:<22}\n", "id", "type", "n", "min x,y,z", "max x,y,z", idw = idw, kw = kw);
    for r in &rs {
        out.push_str(&format!("{:<idw$}  {:<kw$}  {:>3}  {:<22}  {:<22}\n", r.id, r.kind, r.pieces, fmt3(r.min), fmt3(r.max), idw = idw, kw = kw));
    }
    out.push_str(&format!("{} object(s)\n", rs.len()));
    out
}

/// Everything about one object: its JSON, its world bounds, what's directly around it.
pub fn object_info(world: &MapWorld, id: &str, findings: &[Finding]) -> Result<String, String> {
    let items: Vec<&Item> = world.item_by_top_id(id).collect();
    if items.is_empty() {
        let mut close: Vec<&str> = world.items.iter().map(|i| i.top_id.as_str()).filter(|t| t.contains(id) || id.contains(*t)).collect();
        close.sort();
        close.dedup();
        return Err(format!("no object '{id}'{}", if close.is_empty() { String::new() } else { format!(" (did you mean: {}?)", close.iter().take(6).cloned().collect::<Vec<_>>().join(", ")) }));
    }
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for i in &items {
        min = min.min(i.min);
        max = max.max(i.max);
    }
    let mut out = String::new();
    let raw = world.raw.get("objects").and_then(Value::as_array).and_then(|a| a.iter().find(|o| o.get("id").and_then(Value::as_str) == Some(id)));
    if let Some(r) = raw {
        out.push_str(&format!("json: {}\n", serde_json::to_string(r).unwrap()));
    }
    out.push_str(&format!("pieces: {}   kind: {}\n", items.len(), items[0].kind.label()));
    out.push_str(&format!("world bounds: min ({})  max ({})  size {:.2} x {:.2} x {:.2}\n", fmt3(min), fmt3(max), max.x - min.x, max.y - min.y, max.z - min.z));
    out.push_str(&format!("origin: ({})\n", fmt3(items[0].origin)));
    // Neighbours within 1.5 m (footprint gap), excluding itself.
    let mut near: Vec<(f32, &str)> = Vec::new();
    for o in world.items.iter().filter(|o| o.top_id != id && o.is_solid()) {
        let dx = (min.x - o.max.x).max(o.min.x - max.x).max(0.0);
        let dz = (min.z - o.max.z).max(o.min.z - max.z).max(0.0);
        let dy_overlap = o.max.y > min.y + 0.05 && o.min.y < max.y - 0.05;
        let d = (dx * dx + dz * dz).sqrt();
        if d < 1.5 && dy_overlap {
            near.push((d, o.top_id.as_str()));
        }
    }
    near.sort_by(|a, b| a.partial_cmp(b).unwrap());
    near.dedup_by(|a, b| a.1 == b.1);
    if !near.is_empty() {
        out.push_str("nearby solids: ");
        out.push_str(&near.iter().take(10).map(|(d, n)| format!("{n} ({d:.2}m)")).collect::<Vec<_>>().join(", "));
        out.push('\n');
    }
    let related: Vec<&Finding> = findings.iter().filter(|f| f.ids.iter().any(|i| i == id)).collect();
    if related.is_empty() {
        out.push_str("lint: no findings for this object\n");
    } else {
        for f in related {
            out.push_str(&format!("lint {} [{}] {}\n", f.sev.label().trim(), f.code, f.message));
        }
    }
    Ok(out)
}

/// The prop library: name, size, and how each blocks the player.
pub fn list_props(json_out: bool) -> String {
    let mut rows: Vec<(String, Vec3, String, usize)> = Vec::new();
    for k in PropKind::ALL {
        let (mn, mx) = local_bounds(k);
        let coll = match collision(k) {
            Collision::Union => "solid".to_string(),
            Collision::None => "walk-through".to_string(),
            Collision::Box { .. } => {
                let (a, b) = collision_box(k).unwrap();
                format!("trunk {:.2}x{:.2}", b.x - a.x, b.z - a.z)
            }
        };
        rows.push((k.name().to_string(), mx - mn, coll, prop_parts(k).len()));
    }
    if json_out {
        let v: Vec<Value> = rows.iter().map(|(n, s, c, p)| json!({"prop": n, "size": [s.x, s.y, s.z], "collision": c, "parts": p})).collect();
        return serde_json::to_string_pretty(&v).unwrap();
    }
    let mut out = String::from("prop                 size w x h x d (m)      collision       parts\n");
    for (n, s, c, p) in rows {
        out.push_str(&format!("{:<20} {:>5.2} x {:>4.2} x {:>4.2}      {:<15} {}\n", n, s.x, s.y, s.z, c, p));
    }
    out.push_str("\nEvery prop's origin is the middle of its base (position.y = the surface it stands on). Props with a\nfront (sofa, bed, tv, stove, sink, toilet, ...) face local +Z: rotation [0,180,0] turns one to face -Z.\nColor: the instance material.color tints the body; parts like foliage, screens, and porcelain use fixed colors\n(for trees/bushes/flower_patch the material color IS the foliage/bloom color).\n");
    out
}

/// True if `it` is one of the kinds a `--kind` filter can name.
pub fn kind_matches(it: &Item, want: &str) -> bool {
    match it.kind {
        ItemKind::Prop(k) => want == "prop" || k.name() == want,
        _ => it.kind.label() == want,
    }
}
