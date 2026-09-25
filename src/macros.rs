//! Parse-time "macro" object types: `wall` and `fence`.
//!
//! Hand-authoring a wall with a door in it means splitting it into pieces and doing the offset
//! arithmetic by hand — exactly the kind of fiddly, error-prone work that produced misaligned
//! doorways and gaps in early maps. A macro object is authored as one compact description and
//! *expands at parse time into an ordinary `group` of `box` objects*, so nothing downstream
//! (rendering, collision, the analysis tools) needs to know macros exist: they only ever see
//! boxes. Expansion is plain JSON-to-JSON, so it is also unit-testable without a GPU.
//!
//! Expanded piece ids are `<macro id>.<piece>` (e.g. `wall_front.seg0`, `wall_front.head1`), and
//! the macro object itself becomes the `group`, so tools report/edit at the macro's own id.

use serde_json::{json, Map, Value};

/// Coplanar faces of two different boxes z-fight (flicker between their colours), and the
/// pieces of a trimmed opening naturally want to share planes: the trim's inner face with the
/// wall's cut edge, the lintel's underside with the trim above it, the window sill's top with
/// the trim under it, a baseboard's end with the wall's end. Each is separated by this much,
/// *inside* the trim's volume where it can't be seen, so nothing has to be visibly misaligned.
const FIGHT_GAP: f32 = 0.003;

const TRIM_WIDTH: f32 = 0.06;
const GLASS_THICKNESS: f32 = 0.03;
const MIN_PIECE: f32 = 0.004;

fn num(obj: &Map<String, Value>, key: &str, default: f32) -> f32 {
    obj.get(key).and_then(Value::as_f64).map(|x| x as f32).unwrap_or(default)
}

fn xz(v: Option<&Value>) -> Option<[f32; 2]> {
    let a = v?.as_array()?;
    if a.len() != 2 {
        return None;
    }
    Some([a[0].as_f64()? as f32, a[1].as_f64()? as f32])
}

fn round(x: f32) -> f64 {
    ((x as f64) * 10000.0).round() / 10000.0
}

fn boxed(id: String, size: [f32; 3], pos: [f32; 3], rot_y: f32, material: &Value) -> Value {
    let mut o = json!({
        "id": id,
        "type": "box",
        "size": [round(size[0]), round(size[1]), round(size[2])],
        "position": [round(pos[0]), round(pos[1]), round(pos[2])],
        "material": material,
    });
    if rot_y.abs() > 1e-4 {
        o["rotation"] = json!([0, round(rot_y), 0]);
    }
    o
}

fn material_of(obj: &Map<String, Value>) -> Value {
    obj.get("material").cloned().unwrap_or_else(|| json!({ "color": "#d8d0c0", "roughness": 0.85 }))
}

/// Material derived from a hex color string (for trim/posts) — everything else defaults.
fn material_from_hex(hex: &str, roughness: f32) -> Value {
    json!({ "color": hex, "roughness": roughness })
}

fn darken_hex(hex: &str, factor: f32) -> String {
    let h = hex.trim_start_matches('#');
    if h.len() < 6 {
        return "#6b5a44".to_string();
    }
    let c = |i: usize| -> u8 {
        let v = u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(128) as f32 * factor;
        v.clamp(0.0, 255.0) as u8
    };
    format!("#{:02x}{:02x}{:02x}", c(0), c(2), c(4))
}

/// Y-rotation (degrees) that turns local `+X` into the direction `(dx, dz)`.
fn yaw_for(dx: f32, dz: f32) -> f32 {
    (-dz).atan2(dx).to_degrees()
}

struct Opening {
    at: f32,
    width: f32,
    height: f32,
    sill: f32,
    glass: bool,
    trim: Option<String>,
}

/// Expands a `wall` object. See SPEC.md (`### wall`) for the authoring format.
fn expand_wall(obj: &Map<String, Value>, id: &str, errs: &mut Vec<String>) -> Vec<Value> {
    let (Some(from), Some(to)) = (xz(obj.get("from")), xz(obj.get("to"))) else {
        errs.push(format!("{id}.from/to: a wall needs 'from' and 'to' as [x, z] pairs"));
        return vec![];
    };
    let y = num(obj, "y", 0.0);
    let height = num(obj, "height", 2.7);
    let t = num(obj, "thickness", 0.2);
    if height <= 0.0 || t <= 0.0 {
        errs.push(format!("{id}: 'height' and 'thickness' must be > 0"));
        return vec![];
    }
    let (dx, dz) = (to[0] - from[0], to[1] - from[1]);
    let len = (dx * dx + dz * dz).sqrt();
    if len < 0.05 {
        errs.push(format!("{id}: wall 'from' and 'to' are the same point"));
        return vec![];
    }
    let extend = obj.get("extend").and_then(Value::as_bool).unwrap_or(true);
    let ext = if extend { t * 0.5 } else { 0.0 };
    let wall_trim = obj.get("trim").and_then(Value::as_str).map(str::to_string);
    let mat = material_of(obj);

    let mut openings: Vec<Opening> = Vec::new();
    if let Some(arr) = obj.get("openings") {
        let Some(arr) = arr.as_array() else {
            errs.push(format!("{id}.openings: must be an array"));
            return vec![];
        };
        for (i, o) in arr.iter().enumerate() {
            let path = format!("{id}.openings[{i}]");
            let Some(o) = o.as_object() else {
                errs.push(format!("{path}: must be an object"));
                continue;
            };
            crate::strict::check_keys(errs, &path, o, crate::strict::OPENING_KEYS);
            let kind = o.get("kind").and_then(Value::as_str).unwrap_or("door");
            let (def_h, def_sill) = match kind {
                "door" => (2.2, 0.0),
                "window" => (1.2, 0.9),
                "arch" => ((height * 0.85).min(2.4), 0.0),
                other => {
                    errs.push(format!("{path}.kind: unknown '{other}' (expected door, window, or arch)"));
                    continue;
                }
            };
            let Some(at) = o.get("at").and_then(Value::as_f64).map(|v| v as f32) else {
                errs.push(format!("{path}.at: missing (distance along the wall, from 'from', to the opening's center)"));
                continue;
            };
            let width = num(o, "width", if kind == "arch" { 1.6 } else { 1.0 });
            let oh = num(o, "height", def_h);
            let sill = num(o, "sill", def_sill);
            if kind == "door" && oh < crate::player::PLAYER_HEADROOM {
                errs.push(format!(
                    "{path}.height: a door must be at least {:.2} m tall (got {:.2}) — the player's body is 2.0 m, so a lower header blocks the doorway",
                    crate::player::PLAYER_HEADROOM,
                    oh
                ));
                continue;
            }
            if width <= 0.0 || oh <= 0.0 || sill < 0.0 {
                errs.push(format!("{path}: width/height must be > 0 and sill >= 0"));
                continue;
            }
            if at - width * 0.5 < -1e-3 || at + width * 0.5 > len + 1e-3 {
                errs.push(format!("{path}.at: opening spans {:.2}..{:.2} but the wall is only {:.2} long", at - width * 0.5, at + width * 0.5, len));
                continue;
            }
            if sill + oh > height + 1e-3 {
                errs.push(format!("{path}: sill + height ({:.2}) exceeds the wall height ({:.2})", sill + oh, height));
                continue;
            }
            let glass = o.get("glass").and_then(Value::as_bool).unwrap_or(kind == "window");
            let trim = o.get("trim").and_then(Value::as_str).map(str::to_string).or_else(|| wall_trim.clone());
            openings.push(Opening { at, width, height: oh, sill, glass, trim });
        }
    }
    openings.sort_by(|a, b| a.at.partial_cmp(&b.at).unwrap());
    for pair in openings.windows(2) {
        if pair[0].at + pair[0].width * 0.5 > pair[1].at - pair[1].width * 0.5 + 1e-3 {
            errs.push(format!(
                "{id}.openings: openings at {:.2} and {:.2} overlap (each is centered on its 'at' with its own 'width')",
                pair[0].at, pair[1].at
            ));
        }
    }

    if let Some(Value::Object(b)) = obj.get("baseboard") {
        crate::strict::check_keys(errs, &format!("{id}.baseboard"), b, crate::strict::BASEBOARD_KEYS);
    }
    let base = obj.get("baseboard").map(|b| match b {
        Value::String(s) => (s.clone(), 0.12),
        Value::Object(m) => (m.get("color").and_then(Value::as_str).unwrap_or("#f2efe8").to_string(), num(m, "height", 0.12)),
        _ => ("#f2efe8".to_string(), 0.12),
    });

    // Group-local frame: X along the wall (0 at the wall's start), Y up from `y`, Z across the
    // thickness. Local x is shifted by -len/2 when a piece is emitted, since the group sits at
    // the wall's midpoint.
    let half = len * 0.5;
    let mut children = Vec::new();
    let piece = |name: String, x0: f32, x1: f32, y0: f32, y1: f32, thick: f32, m: &Value, kids: &mut Vec<Value>| {
        let (w, h) = (x1 - x0, y1 - y0);
        if w > MIN_PIECE && h > MIN_PIECE {
            kids.push(boxed(name, [w, h, thick], [(x0 + x1) * 0.5 - half, (y0 + y1) * 0.5, 0.0], 0.0, m));
        }
    };

    // Solid runs between openings.
    // Around a trimmed opening the wall run stops `FIGHT_GAP` short of the cut edge (the trim's
    // jamb overlaps that sliver, so it stays sealed); baseboards stop a hair short of every cut
    // edge and wall end, so their end faces don't share a plane with the wall's.
    let mut cursor = -ext;
    let mut seg = 0;
    for o in &openings {
        let gap = if o.trim.is_some() { FIGHT_GAP } else { 0.0 };
        let end = o.at - o.width * 0.5 - gap;
        piece(format!("{id}.seg{seg}"), cursor, end, 0.0, height, t, &mat, &mut children);
        if let Some((bc, bh)) = &base {
            piece(format!("{id}.base{seg}"), cursor + FIGHT_GAP, end - FIGHT_GAP, 0.0, *bh, t + 0.03, &material_from_hex(bc, 0.6), &mut children);
        }
        seg += 1;
        cursor = o.at + o.width * 0.5 + gap;
    }
    piece(format!("{id}.seg{seg}"), cursor, len + ext, 0.0, height, t, &mat, &mut children);
    if let Some((bc, bh)) = &base {
        piece(format!("{id}.base{seg}"), cursor + FIGHT_GAP, len + ext - FIGHT_GAP, 0.0, *bh, t + 0.03, &material_from_hex(bc, 0.6), &mut children);
    }

    for (i, o) in openings.iter().enumerate() {
        let (x0, x1) = (o.at - o.width * 0.5, o.at + o.width * 0.5);
        let top = o.sill + o.height;
        // With trim: the header/sill cover the sliver the wall runs left open, and start/stop
        // `FIGHT_GAP` inside the trim so their faces aren't coplanar with the trim's.
        let (g, gv) = if o.trim.is_some() { (FIGHT_GAP, FIGHT_GAP) } else { (0.0, 0.0) };
        piece(format!("{id}.head{i}"), x0 - g, x1 + g, top + gv, height, t, &mat, &mut children);
        piece(format!("{id}.sill{i}"), x0 - g, x1 + g, 0.0, o.sill, t, &mat, &mut children);
        if o.glass {
            let glass = json!({ "color": "#6f9fd0", "roughness": 0.1, "metallic": 0.1, "emissive": "#3c6aa0" });
            piece(format!("{id}.glass{i}"), x0, x1, o.sill, top, GLASS_THICKNESS, &glass, &mut children);
        }
        if let Some(tc) = &o.trim {
            let tm = material_from_hex(tc, 0.55);
            let ft = TRIM_WIDTH;
            let pt = t + 0.04;
            // (Above a window's sill trim, so their end faces aren't coplanar either.)
            let jamb0 = if o.sill > 0.0 { o.sill + FIGHT_GAP } else { 0.0 };
            piece(format!("{id}.trim{i}l"), x0 - ft, x0, jamb0, top, pt, &tm, &mut children);
            piece(format!("{id}.trim{i}r"), x1, x1 + ft, jamb0, top, pt, &tm, &mut children);
            piece(format!("{id}.trim{i}t"), x0 - ft, x1 + ft, top, top + ft, pt, &tm, &mut children);
            if o.sill > 0.0 {
                piece(format!("{id}.trim{i}b"), x0 - ft, x1 + ft, o.sill - ft, o.sill + FIGHT_GAP, t + 0.07, &tm, &mut children);
            }
        }
    }

    // Wrap in a positioned/rotated group so every child stays in the simple wall-local frame.
    let mid = [(from[0] + to[0]) * 0.5, y, (from[1] + to[1]) * 0.5];
    vec![json!({
        "id": format!("{id}.frame"),
        "type": "group",
        "position": [round(mid[0]), round(mid[1]), round(mid[2])],
        "rotation": [0, round(yaw_for(dx, dz)), 0],
        "children": children,
    })]
}

/// Expands a `fence` object into posts, rails and panels along a polyline.
fn expand_fence(obj: &Map<String, Value>, id: &str, errs: &mut Vec<String>) -> Vec<Value> {
    let Some(pts_raw) = obj.get("points").and_then(Value::as_array) else {
        errs.push(format!("{id}.points: a fence needs a 'points' array of [x, z] pairs"));
        return vec![];
    };
    let mut pts: Vec<[f32; 2]> = Vec::new();
    for (i, p) in pts_raw.iter().enumerate() {
        match xz(Some(p)) {
            Some(v) => pts.push(v),
            None => errs.push(format!("{id}.points[{i}]: must be an [x, z] pair")),
        }
    }
    if pts.len() < 2 {
        errs.push(format!("{id}.points: need at least 2 points"));
        return vec![];
    }
    let closed = obj.get("closed").and_then(Value::as_bool).unwrap_or(false);
    if closed {
        pts.push(pts[0]);
    }
    let segments = pts.len() - 1;
    let y = num(obj, "y", 0.0);
    let height = num(obj, "height", 1.8);
    let spacing = num(obj, "post_spacing", 2.0).max(0.5);
    let style = obj.get("style").and_then(Value::as_str).unwrap_or("panel");
    if style != "panel" && style != "rail" {
        errs.push(format!("{id}.style: unknown '{style}' (expected panel or rail)"));
        return vec![];
    }
    let mat = material_of(obj);
    let base_color = mat.get("color").and_then(Value::as_str).unwrap_or("#b8a888").to_string();
    let post_hex = obj.get("post_color").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| darken_hex(&base_color, 0.72));
    let post_mat = material_from_hex(&post_hex, 0.7);
    let post_w = 0.14;

    // Gates: [{ "at": [x, z], "width": 2.4 }] — a gap cut in whichever segment passes closest.
    let mut gaps: Vec<([f32; 2], f32)> = Vec::new();
    if let Some(arr) = obj.get("gaps").and_then(Value::as_array) {
        for (i, g) in arr.iter().enumerate() {
            if let Some(go) = g.as_object() {
                crate::strict::check_keys(errs, &format!("{id}.gaps[{i}]"), go, crate::strict::GAP_KEYS);
            }
            match (g.get("at").and_then(|v| xz(Some(v))), g.get("width").and_then(Value::as_f64)) {
                (Some(at), Some(w)) if w > 0.0 => gaps.push((at, w as f32)),
                _ => errs.push(format!("{id}.gaps[{i}]: needs 'at': [x, z] and 'width' > 0")),
            }
        }
    }

    let mut children = Vec::new();
    let mut post_i = 0;
    let mut bay_i = 0;
    for (si, w) in pts.windows(2).enumerate() {
        let (a, b) = (w[0], w[1]);
        let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dz * dz).sqrt();
        if len < 0.05 {
            continue;
        }
        let (ux, uz) = (dx / len, dz / len);
        let yaw = yaw_for(dx, dz);
        // Gap intervals [d0, d1] along this segment.
        let mut cut: Vec<(f32, f32)> = Vec::new();
        for (at, gw) in &gaps {
            let (rx, rz) = (at[0] - a[0], at[1] - a[1]);
            let along = rx * ux + rz * uz;
            let across = (-rx * uz + rz * ux).abs();
            if across < 0.6 && along > -0.01 && along < len + 0.01 {
                cut.push((along - gw * 0.5, along + gw * 0.5));
            }
        }
        let in_gap = |d: f32| cut.iter().any(|(g0, g1)| d > *g0 + 1e-3 && d < *g1 - 1e-3);
        let bays = (len / spacing).ceil().max(1.0) as usize;
        let bay = len / bays as f32;
        let at_pt = |d: f32| [a[0] + ux * d, a[1] + uz * d];
        // Posts at bay boundaries (skipped inside a gap; posts flank the gap instead).
        let mut post_ds: Vec<f32> = (0..=bays).map(|k| k as f32 * bay).filter(|d| !in_gap(*d)).collect();
        for (g0, g1) in &cut {
            post_ds.push(g0.max(0.0));
            post_ds.push(g1.min(len));
        }
        post_ds.sort_by(|p, q| p.partial_cmp(q).unwrap());
        post_ds.dedup_by(|p, q| (*p - *q).abs() < 0.05);
        // The shared corner post between two segments is emitted once (by the earlier segment),
        // and a closed loop's final post is the first segment's first post.
        if si > 0 {
            post_ds.retain(|d| *d > 0.05);
        }
        if closed && si == segments - 1 {
            post_ds.retain(|d| *d < len - 0.05);
        }
        for d in &post_ds {
            let p = at_pt(*d);
            children.push(boxed(format!("{id}.post{post_i}"), [post_w, height + 0.06, post_w], [p[0], y + (height + 0.06) * 0.5, p[1]], yaw, &post_mat));
            children.push(boxed(format!("{id}.cap{post_i}"), [post_w + 0.06, 0.05, post_w + 0.06], [p[0], y + height + 0.085, p[1]], yaw, &post_mat));
            post_i += 1;
        }
        for k in 0..bays {
            // The bay's span between its two posts, minus any gate gap (panels stop at the
            // posts that flank the gap).
            let mut spans = vec![(k as f32 * bay + post_w * 0.5, (k + 1) as f32 * bay - post_w * 0.5)];
            for (g0, g1) in &cut {
                let h = post_w * 0.5;
                spans = spans
                    .into_iter()
                    .flat_map(|(lo, hi)| {
                        let mut out = Vec::new();
                        if lo < g0 - h {
                            out.push((lo, hi.min(g0 - h)));
                        }
                        if hi > g1 + h {
                            out.push((lo.max(g1 + h), hi));
                        }
                        out
                    })
                    .collect();
            }
            for (d0, d1) in spans {
                let blen = d1 - d0;
                if blen <= 0.15 {
                    continue;
                }
                let p = at_pt((d0 + d1) * 0.5);
                if style == "panel" {
                    let ph = height - 0.3;
                    children.push(boxed(format!("{id}.panel{bay_i}"), [blen, ph, 0.06], [p[0], y + 0.12 + ph * 0.5, p[1]], yaw, &mat));
                    children.push(boxed(format!("{id}.toprail{bay_i}"), [blen, 0.09, 0.1], [p[0], y + height - 0.05, p[1]], yaw, &post_mat));
                    children.push(boxed(format!("{id}.botrail{bay_i}"), [blen, 0.1, 0.1], [p[0], y + 0.08, p[1]], yaw, &post_mat));
                } else {
                    for (ri, ry) in [height * 0.85, height * 0.5, height * 0.15].iter().enumerate() {
                        children.push(boxed(format!("{id}.rail{bay_i}_{ri}"), [blen, 0.07, 0.05], [p[0], y + ry, p[1]], yaw, &mat));
                    }
                }
                bay_i += 1;
            }
        }
    }
    vec![json!({ "id": format!("{id}.frame"), "type": "group", "children": children })]
}

/// Expands a macro object (`"wall"` or `"fence"`) into a `group` JSON object with the macro's
/// own `id`. Returns the expanded group, or every validation error found (`path: message`).
pub fn expand(ty: &str, obj: &Map<String, Value>, id: &str) -> Result<Value, Vec<String>> {
    let mut errs = Vec::new();
    let kids = match ty {
        "wall" => expand_wall(obj, id, &mut errs),
        "fence" => expand_fence(obj, id, &mut errs),
        _ => unreachable!("caller checked the macro type"),
    };
    if !errs.is_empty() {
        return Err(errs);
    }
    Ok(json!({ "id": id, "type": "group", "children": kids }))
}

pub const MACRO_TYPES: &[&str] = &["wall", "fence"];

#[cfg(test)]
mod tests {
    use super::*;

    fn wall(json: Value) -> Result<Value, Vec<String>> {
        expand("wall", json.as_object().unwrap(), json["id"].as_str().unwrap())
    }

    fn all_boxes(v: &Value, out: &mut Vec<Value>) {
        if let Some(kids) = v.get("children").and_then(Value::as_array) {
            for k in kids {
                if k["type"] == "group" {
                    all_boxes(k, out);
                } else {
                    out.push(k.clone());
                }
            }
        }
    }

    #[test]
    fn wall_with_door_leaves_a_gap_and_a_header() {
        let v = wall(json!({"id":"w","type":"wall","from":[0,0],"to":[6,0],"height":2.7,"thickness":0.2,
            "openings":[{"at":3.0,"width":1.0,"kind":"door"}]}))
        .unwrap();
        let mut boxes = Vec::new();
        all_boxes(&v, &mut boxes);
        let ids: Vec<&str> = boxes.iter().map(|b| b["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"w.seg0") && ids.contains(&"w.seg1") && ids.contains(&"w.head0"), "{ids:?}");
        assert!(!ids.iter().any(|i| i.contains("sill")), "a door has no sill piece");
        // No solid piece may cover the doorway span (x from 2.5..3.5 measured from `from`).
        let seg0 = boxes.iter().find(|b| b["id"] == "w.seg0").unwrap();
        let seg1 = boxes.iter().find(|b| b["id"] == "w.seg1").unwrap();
        let (c0, w0) = (seg0["position"][0].as_f64().unwrap(), seg0["size"][0].as_f64().unwrap());
        let (c1, w1) = (seg1["position"][0].as_f64().unwrap(), seg1["size"][0].as_f64().unwrap());
        assert!((c0 + w0 / 2.0 - (2.5 - 3.0)).abs() < 1e-3, "seg0 must end at the door's left edge");
        assert!((c1 - w1 / 2.0 - (3.5 - 3.0)).abs() < 1e-3, "seg1 must start at the door's right edge");
    }

    /// Two boxes whose faces lie in the same plane, face the same way and overlap z-fight (the
    /// pixels flicker between the two materials). A trimmed, baseboarded wall with a door and a
    /// window is where the pieces naturally want to share planes.
    #[test]
    fn a_trimmed_wall_has_no_coplanar_overlapping_faces() {
        for (trim, base) in [(true, true), (true, false), (false, true), (false, false)] {
            let mut w = json!({"id":"w","type":"wall","from":[0,0],"to":[7,0],"height":2.6,"thickness":0.15,
                "openings":[{"at":1.5,"width":1.0,"kind":"door"},{"at":4.0,"width":1.2,"kind":"window","sill":0.9,"height":1.0},
                            {"at":6.0,"width":0.9,"kind":"arch","height":2.2}]});
            if trim {
                w["trim"] = json!("#e8e2d0");
            }
            if base {
                w["baseboard"] = json!("#f2efe8");
            }
            let mut boxes = Vec::new();
            all_boxes(&wall(w).unwrap(), &mut boxes);
            let n = |b: &Value, k: &str, i: usize| b[k][i].as_f64().unwrap();
            // (axis, facing sign, plane coordinate, [lo, hi] on the other two axes)
            let faces = |b: &Value| {
                let mut out = Vec::new();
                for axis in 0..3 {
                    for sign in [-1.0, 1.0] {
                        let plane = n(b, "position", axis) + sign * n(b, "size", axis) / 2.0;
                        let others: Vec<[f64; 2]> = (0..3)
                            .filter(|&a| a != axis)
                            .map(|a| [n(b, "position", a) - n(b, "size", a) / 2.0, n(b, "position", a) + n(b, "size", a) / 2.0])
                            .collect();
                        out.push((axis, sign, plane, others));
                    }
                }
                out
            };
            let all: Vec<_> = boxes.iter().map(|b| (b["id"].as_str().unwrap().to_string(), faces(b))).collect();
            for i in 0..all.len() {
                for j in i + 1..all.len() {
                    for fa in &all[i].1 {
                        for fb in &all[j].1 {
                            // Bottoms resting on the floor face the ground: never seen.
                            let on_floor = fa.0 == 1 && fa.1 < 0.0 && fa.2.abs() < 1e-6;
                            if !on_floor && fa.0 == fb.0 && fa.1 == fb.1 && (fa.2 - fb.2).abs() < 1e-6 {
                                let overlap = |k: usize| (fa.3[k][1].min(fb.3[k][1]) - fa.3[k][0].max(fb.3[k][0])).max(0.0);
                                assert!(
                                    overlap(0) * overlap(1) < 1e-9,
                                    "trim={trim} base={base}: '{}' and '{}' have coplanar overlapping faces (axis {}, at {:.4})",
                                    all[i].0,
                                    all[j].0,
                                    fa.0,
                                    fa.2
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn opening_outside_the_wall_is_an_error() {
        let e = wall(json!({"id":"w","type":"wall","from":[0,0],"to":[2,0],
            "openings":[{"at":1.9,"width":1.0}]}))
        .unwrap_err();
        assert!(e[0].contains("w.openings[0].at"), "{e:?}");
    }

    #[test]
    fn door_shorter_than_the_player_is_an_error() {
        let e = wall(json!({"id":"w","type":"wall","from":[0,0],"to":[4,0],
            "openings":[{"at":2.0,"width":1.0,"kind":"door","height":2.0}]}))
        .unwrap_err();
        assert!(e[0].contains("w.openings[0].height"), "{e:?}");
    }

    #[test]
    fn overlapping_openings_are_an_error() {
        let e = wall(json!({"id":"w","type":"wall","from":[0,0],"to":[6,0],
            "openings":[{"at":2.0,"width":1.5},{"at":2.9,"width":1.5}]}))
        .unwrap_err();
        assert!(e.iter().any(|m| m.contains("overlap")), "{e:?}");
    }

    #[test]
    fn window_gets_sill_head_and_glass() {
        let v = wall(json!({"id":"w","type":"wall","from":[0,0],"to":[4,0],
            "openings":[{"at":2.0,"width":1.2,"kind":"window"}]}))
        .unwrap();
        let mut boxes = Vec::new();
        all_boxes(&v, &mut boxes);
        let ids: Vec<&str> = boxes.iter().map(|b| b["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"w.sill0") && ids.contains(&"w.head0") && ids.contains(&"w.glass0"), "{ids:?}");
    }

    #[test]
    fn fence_closed_loop_has_posts_and_panels() {
        let obj = json!({"id":"f","type":"fence","points":[[0,0],[4,0],[4,4],[0,4]],"closed":true});
        let v = expand("fence", obj.as_object().unwrap(), "f").unwrap();
        let mut boxes = Vec::new();
        all_boxes(&v, &mut boxes);
        assert!(boxes.iter().filter(|b| b["id"].as_str().unwrap().starts_with("f.post")).count() >= 8);
        assert!(boxes.iter().any(|b| b["id"].as_str().unwrap().starts_with("f.panel")));
    }

    #[test]
    fn fence_gap_removes_the_panel_over_it() {
        let obj = json!({"id":"f","type":"fence","points":[[0,0],[10,0]],"gaps":[{"at":[5,0],"width":2.4}]});
        let v = expand("fence", obj.as_object().unwrap(), "f").unwrap();
        let mut boxes = Vec::new();
        all_boxes(&v, &mut boxes);
        for b in boxes.iter().filter(|b| b["id"].as_str().unwrap().starts_with("f.panel")) {
            let (c, w) = (b["position"][0].as_f64().unwrap(), b["size"][0].as_f64().unwrap());
            let (lo, hi) = (c - w / 2.0, c + w / 2.0);
            assert!(hi <= 3.85 || lo >= 6.15, "panel {lo}..{hi} intrudes into the 3.8..6.2 gate gap");
        }
    }
}
