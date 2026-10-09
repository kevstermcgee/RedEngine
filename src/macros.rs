//! Parse-time "macro" object types: `wall`, `fence`, `text` and `array`.
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

/// The punctuation the 5x7 font can draw (asked of the font, so a message about it cannot drift).
fn font_punctuation() -> String {
    (0x21u8..0x7f).map(char::from).filter(|c| !c.is_ascii_alphanumeric() && crate::tools::font::glyph(*c).is_some()).collect()
}
/// Most characters and lines one `text` object takes (a sign, not a page).
const MAX_TEXT_CHARS: usize = 400;
const MAX_TEXT_LINES: usize = 12;
/// Letter pixels per glyph row: the font is 5 wide and 7 tall.
const GLYPH_COLS: usize = crate::tools::font::GLYPH_W as usize;
const GLYPH_ROWS: usize = crate::tools::font::GLYPH_H as usize;

/// A rectangle of font pixels: `(col, row, width, height)`, row 0 at the top of the text block.
type PixelRect = (usize, usize, usize, usize);

/// The font's pixels for `text` (one string per line) as a grid, with the block's size in pixels.
/// `spacing` pixels sit between letters and `line_gap` between lines; `align` places shorter lines.
fn text_grid(lines: &[&str], spacing: usize, line_gap: usize, align: &str) -> (Vec<Vec<bool>>, usize, usize) {
    let width_of = |l: &str| {
        let n = l.chars().count();
        if n == 0 {
            0
        } else {
            n * GLYPH_COLS + (n - 1) * spacing
        }
    };
    let cols = lines.iter().map(|l| width_of(l)).max().unwrap_or(0);
    let rows = lines.len() * GLYPH_ROWS + lines.len().saturating_sub(1) * line_gap;
    let mut grid = vec![vec![false; cols]; rows];
    for (li, line) in lines.iter().enumerate() {
        let free = cols - width_of(line);
        let start = match align {
            "left" => 0,
            "right" => free,
            _ => free / 2,
        };
        let top = li * (GLYPH_ROWS + line_gap);
        for (ci, ch) in line.chars().enumerate() {
            let Some(rows_of) = crate::tools::font::glyph(ch) else { continue };
            let left = start + ci * (GLYPH_COLS + spacing);
            for (r, row) in rows_of.iter().enumerate() {
                for (c, b) in row.bytes().enumerate() {
                    if b == b'#' {
                        grid[top + r][left + c] = true;
                    }
                }
            }
        }
    }
    (grid, cols, rows)
}

/// Covers every inked pixel exactly once with as few rectangles as a greedy sweep finds: take the first free ink pixel, grow right while
/// ink, then grow down while the whole row segment is free ink. A letter becomes 3 to 8 boxes instead of up to 35.
fn merge_pixels(grid: &[Vec<bool>]) -> Vec<PixelRect> {
    let rows = grid.len();
    let cols = grid.first().map_or(0, Vec::len);
    let mut taken = vec![vec![false; cols]; rows];
    let mut out = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            if !grid[r][c] || taken[r][c] {
                continue;
            }
            let mut w = 1;
            while c + w < cols && grid[r][c + w] && !taken[r][c + w] {
                w += 1;
            }
            let mut h = 1;
            while r + h < rows && (c..c + w).all(|x| grid[r + h][x] && !taken[r + h][x]) {
                h += 1;
            }
            for row in taken.iter_mut().skip(r).take(h) {
                for t in row.iter_mut().skip(c).take(w) {
                    *t = true;
                }
            }
            out.push((c, r, w, h));
        }
    }
    out
}

/// Expands a `text` object: lettering from the engine's 5x7 font as merged boxes, facing +Z and reading along +X, centred on the object's
/// `position`. See SPEC.md (`### text`).
fn expand_text(obj: &Map<String, Value>, id: &str, errs: &mut Vec<String>) -> Vec<Value> {
    let Some(text) = obj.get("text").and_then(Value::as_str) else {
        errs.push(format!("{id}.text: a text object needs \"text\": \"WORDS\" (use \\n for a second line)"));
        return vec![];
    };
    if text.trim().is_empty() {
        errs.push(format!("{id}.text: is empty"));
        return vec![];
    }
    let lines: Vec<&str> = text.split('\n').collect();
    if text.chars().count() > MAX_TEXT_CHARS || lines.len() > MAX_TEXT_LINES {
        errs.push(format!("{id}.text: at most {MAX_TEXT_CHARS} characters and {MAX_TEXT_LINES} lines (a sign, not a page)"));
        return vec![];
    }
    let bad: Vec<char> = {
        let mut b: Vec<char> = text.chars().filter(|c| *c != '\n' && *c != ' ' && crate::tools::font::glyph(*c).is_none()).collect();
        b.sort_unstable();
        b.dedup();
        b
    };
    if !bad.is_empty() {
        errs.push(format!(
            "{id}.text: the font has no {} (it has letters, digits and {}; lowercase shows as capitals)",
            bad.iter().map(|c| format!("`{c}`")).collect::<Vec<_>>().join(", "),
            font_punctuation()
        ));
        return vec![];
    }
    let height = num(obj, "height", 0.3);
    let px = height / GLYPH_ROWS as f32;
    let depth = num(obj, "depth", px.max(0.01));
    let spacing = num(obj, "spacing", 1.0);
    let line_gap = num(obj, "line_gap", 3.0);
    if !(0.02..=20.0).contains(&height) || !(0.005..=2.0).contains(&depth) {
        errs.push(format!("{id}: 'height' must be 0.02..20 m (the letter height) and 'depth' 0.005..2 m"));
        return vec![];
    }
    if !(0.0..=6.0).contains(&spacing) || !(0.0..=12.0).contains(&line_gap) || spacing.fract() != 0.0 || line_gap.fract() != 0.0 {
        errs.push(format!("{id}: 'spacing' (0..6) and 'line_gap' (0..12) are whole numbers of letter pixels"));
        return vec![];
    }
    let align = obj.get("align").and_then(Value::as_str).unwrap_or("center");
    if !["left", "center", "right"].contains(&align) {
        errs.push(format!("{id}.align: `{align}` is not one of left, center, right"));
        return vec![];
    }
    let (grid, cols, rows) = text_grid(&lines, spacing as usize, line_gap as usize, align);
    let mat = obj.get("material").cloned().unwrap_or_else(|| json!({ "color": "#f2efe6", "roughness": 0.8 }));
    let backing = match obj.get("backing") {
        None => None,
        Some(Value::String(color)) => Some((material_from_hex(color, 0.9), 0.1, 0.04)),
        Some(Value::Object(b)) => {
            let m = b.get("material").cloned().or_else(|| b.get("color").and_then(Value::as_str).map(|c| material_from_hex(c, 0.9)));
            Some((m.unwrap_or_else(|| material_from_hex("#23303f", 0.9)), num(b, "margin", 0.1), num(b, "thickness", 0.04)))
        }
        Some(_) => {
            errs.push(format!("{id}.backing: a colour like \"#23303f\" or {{color?, material?, margin?, thickness?}}"));
            return vec![];
        }
    };
    // Letters start a hair inside the board so their backs never share its plane.
    let z0 = if backing.is_some() { -FIGHT_GAP } else { 0.0 };
    let (w_m, h_m) = (cols as f32 * px, rows as f32 * px);
    let mut kids = Vec::new();
    if let Some((bmat, margin, thick)) = &backing {
        kids.push(boxed(format!("{id}.backing"), [w_m + 2.0 * margin, h_m + 2.0 * margin, *thick], [0.0, 0.0, -thick / 2.0], 0.0, bmat));
    }
    for (c, r, w, h) in merge_pixels(&grid) {
        let cx = (c as f32 + w as f32 / 2.0) * px - w_m / 2.0;
        let cy = h_m / 2.0 - (r as f32 + h as f32 / 2.0) * px;
        kids.push(boxed(format!("{id}.l{r}_{c}"), [w as f32 * px, h as f32 * px, depth - z0], [cx, cy, (z0 + depth) / 2.0], 0.0, &mat));
    }
    kids
}

/// Most copies one `array` makes (a row of posts, not a field of grass).
const MAX_ARRAY: usize = 500;

fn vec3_of(v: Option<&Value>) -> Option<[f64; 3]> {
    let a = v?.as_array()?;
    if a.len() != 3 {
        return None;
    }
    Some([a[0].as_f64()?, a[1].as_f64()?, a[2].as_f64()?])
}

/// Renames every descendant of `node` to `<prefix>.<its id>`, so copies of a group template do not collide. Children without an id are reported.
fn rename_children(node: &mut Value, prefix: &str, errs: &mut Vec<String>) {
    let Some(kids) = node.get_mut("children").and_then(Value::as_array_mut) else { return };
    for (k, kid) in kids.iter_mut().enumerate() {
        match kid.get("id").and_then(Value::as_str).map(str::to_string) {
            Some(old) => kid["id"] = json!(format!("{prefix}.{old}")),
            None => {
                errs.push(format!("{prefix}.children[{k}]: a template's children need ids (the array prefixes them per copy)"));
                continue;
            }
        }
        let new = kid["id"].as_str().unwrap_or_default().to_string();
        rename_children(kid, &new, errs);
    }
}

/// Expands an `array`: copies of one `template` object along a `step` (`count` times) or at explicit `positions`, each named `<id>.<n>`.
/// Position and rotation of the copy are the template's own plus the offset; a group template's children are renamed per copy.
fn expand_array(obj: &Map<String, Value>, id: &str, errs: &mut Vec<String>) -> Vec<Value> {
    let Some(template) = obj.get("template").and_then(Value::as_object) else {
        errs.push(format!("{id}.template: an array needs \"template\": {{an object without an id}} to copy"));
        return vec![];
    };
    if template.contains_key("id") {
        errs.push(format!("{id}.template.id: remove it (the array names its copies {id}.0, {id}.1, ...)"));
        return vec![];
    }
    let base = match template.get("position") {
        None => [0.0; 3],
        Some(p) => match vec3_of(Some(p)) {
            Some(v) => v,
            None => {
                errs.push(format!("{id}.template.position: must be [x, y, z] (an array moves a fixed position, not a keyframed track)"));
                return vec![];
            }
        },
    };
    let base_rot = vec3_of(template.get("rotation")).unwrap_or([0.0; 3]);
    let offsets: Vec<[f64; 3]> = match (obj.get("positions"), obj.get("count")) {
        (Some(_), Some(_)) => {
            errs.push(format!("{id}: give 'count' (with 'step') or 'positions', not both"));
            return vec![];
        }
        (Some(list), None) => {
            let Some(arr) = list.as_array().filter(|a| !a.is_empty() && a.len() <= MAX_ARRAY) else {
                errs.push(format!("{id}.positions: a list of 1 to {MAX_ARRAY} [x, y, z] offsets"));
                return vec![];
            };
            let parsed: Vec<[f64; 3]> = arr.iter().filter_map(|p| vec3_of(Some(p))).collect();
            if parsed.len() != arr.len() {
                errs.push(format!("{id}.positions: every entry must be [x, y, z]"));
                return vec![];
            }
            parsed
        }
        (None, Some(count)) => {
            let Some(n) = count.as_u64().filter(|n| (1..=MAX_ARRAY as u64).contains(n)) else {
                errs.push(format!("{id}.count: a whole number from 1 to {MAX_ARRAY}"));
                return vec![];
            };
            let step = match obj.get("step") {
                None if n == 1 => [0.0; 3],
                None => {
                    errs.push(format!("{id}.step: needed with 'count' above 1, as [dx, dy, dz] metres between copies"));
                    return vec![];
                }
                Some(v) => match vec3_of(Some(v)) {
                    Some(s) => s,
                    None => {
                        errs.push(format!("{id}.step: must be [dx, dy, dz]"));
                        return vec![];
                    }
                },
            };
            (0..n).map(|i| [step[0] * i as f64, step[1] * i as f64, step[2] * i as f64]).collect()
        }
        (None, None) => {
            errs.push(format!("{id}: an array needs 'count' (with 'step') or 'positions'"));
            return vec![];
        }
    };
    let rot_step = match obj.get("rotation_step") {
        None => [0.0; 3],
        Some(v) => match vec3_of(Some(v)) {
            Some(r) => r,
            None => {
                errs.push(format!("{id}.rotation_step: must be [rx, ry, rz] degrees added per copy"));
                return vec![];
            }
        },
    };
    let mut out = Vec::new();
    for (i, off) in offsets.iter().enumerate() {
        let mut copy = Value::Object(template.clone());
        let name = format!("{id}.{i}");
        copy["id"] = json!(name);
        copy["position"] = json!([round_f64(base[0] + off[0]), round_f64(base[1] + off[1]), round_f64(base[2] + off[2])]);
        if rot_step != [0.0; 3] || template.contains_key("rotation") {
            let k = i as f64;
            copy["rotation"] =
                json!([round_f64(base_rot[0] + rot_step[0] * k), round_f64(base_rot[1] + rot_step[1] * k), round_f64(base_rot[2] + rot_step[2] * k)]);
        }
        rename_children(&mut copy, &name, errs);
        out.push(copy);
    }
    out
}

fn round_f64(x: f64) -> f64 {
    (x * 10000.0).round() / 10000.0
}

/// Expands a macro object (`"wall"`, `"fence"`, `"text"` or `"array"`) into a `group` JSON object with the macro's
/// own `id`. Returns the expanded group, or every validation error found (`path: message`).
/// A numeric option of a macro, checked before it expands: absent is fine (the default applies); present, it must be a number above `min` (or at least `min` when `inclusive`).
/// Anything else is an error that names the field, the number and the fix, where `num` used to fall back to the default and `.max(0.5)` to a minimum without a word.
fn check_num(obj: &Map<String, Value>, key: &str, path: &str, min: f64, inclusive: bool, fix: &str, errs: &mut Vec<String>) {
    let Some(v) = obj.get(key) else { return };
    match v.as_f64().filter(|n| n.is_finite() && if inclusive { *n >= min } else { *n > min }) {
        Some(_) => {}
        None => errs.push(format!(
            "{path}.{key}: must be a number {} {min} (got {}); {fix}",
            if inclusive { "of at least" } else { "greater than" },
            crate::strict::describe_value(v)
        )),
    }
}

/// The numeric options of `wall` and `fence` that `num` reads, checked before expansion (`text` checks its own).
fn check_macro(ty: &str, obj: &Map<String, Value>, id: &str, errs: &mut Vec<String>) {
    match ty {
        "wall" => {
            check_num(obj, "y", id, f64::NEG_INFINITY, true, "`y` is the height of the wall's base in metres", errs);
            check_num(obj, "height", id, 0.0, false, "`height` is the wall's height in metres; 2.7 is the default", errs);
            check_num(obj, "thickness", id, 0.0, false, "`thickness` is in metres; 0.2 is the default", errs);
            for (i, o) in obj.get("openings").and_then(Value::as_array).into_iter().flatten().filter_map(|v| v.as_object()).enumerate() {
                let path = format!("{id}.openings[{i}]");
                check_num(o, "width", &path, 0.0, false, "`width` is in metres", errs);
                check_num(o, "height", &path, 0.0, false, "`height` is in metres", errs);
                check_num(o, "sill", &path, 0.0, true, "`sill` is the height of the opening's bottom edge above the base, in metres", errs);
            }
        }
        "fence" => {
            check_num(obj, "y", id, f64::NEG_INFINITY, true, "`y` is the height of the fence's base in metres", errs);
            check_num(obj, "height", id, 0.0, false, "`height` is the fence's height in metres; 1.8 is the default", errs);
            check_num(obj, "post_spacing", id, 0.5, true, "posts closer than 0.5 m are one solid wall: use a `wall` for that", errs);
        }
        _ => {}
    }
}

pub fn expand(ty: &str, obj: &Map<String, Value>, id: &str) -> Result<Value, Vec<String>> {
    let mut errs = Vec::new();
    check_macro(ty, obj, id, &mut errs);
    if !errs.is_empty() {
        return Err(errs);
    }
    let kids = match ty {
        "wall" => expand_wall(obj, id, &mut errs),
        "fence" => expand_fence(obj, id, &mut errs),
        "text" => expand_text(obj, id, &mut errs),
        "array" => expand_array(obj, id, &mut errs),
        _ => unreachable!("caller checked the macro type"),
    };
    if !errs.is_empty() {
        return Err(errs);
    }
    let mut group = json!({ "id": id, "type": "group", "children": kids });
    if ty == "text" || ty == "array" {
        // Lettering and arrays are placed like any object (a wall's pieces carry absolute coordinates instead).
        for key in ["position", "rotation", "scale", "lint_ignore"] {
            if let Some(v) = obj.get(key) {
                group[key] = v.clone();
            }
        }
        // Lettering is decoration unless it asks otherwise; an array's copies keep whatever their template says.
        match (ty, obj.get("collide")) {
            (_, Some(c)) => group["collide"] = c.clone(),
            ("text", None) => group["collide"] = json!(false),
            _ => {}
        }
    }
    Ok(group)
}

pub const MACRO_TYPES: &[&str] = &["wall", "fence", "text", "array"];

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

    fn text(json: Value) -> Result<Value, Vec<String>> {
        expand("text", json.as_object().unwrap(), json["id"].as_str().unwrap())
    }

    fn kids(v: &Value) -> Vec<Value> {
        let mut out = Vec::new();
        all_boxes(v, &mut out);
        out
    }

    /// Every font pixel of `lines` covered exactly once by the merged rectangles.
    fn coverage(lines: &[&str], spacing: usize, gap: usize, align: &str) -> (usize, usize) {
        let (grid, cols, rows) = text_grid(lines, spacing, gap, align);
        let ink = grid.iter().flatten().filter(|b| **b).count();
        let mut hits = vec![vec![0u32; cols]; rows];
        let rects = merge_pixels(&grid);
        for (c, r, w, h) in &rects {
            for row in hits.iter_mut().skip(*r).take(*h) {
                for cell in row.iter_mut().skip(*c).take(*w) {
                    *cell += 1;
                }
            }
        }
        for r in 0..rows {
            for c in 0..cols {
                assert_eq!(hits[r][c], u32::from(grid[r][c]), "pixel ({c},{r}) of {lines:?}");
            }
        }
        (ink, rects.len())
    }

    #[test]
    fn merged_rectangles_cover_every_font_pixel_exactly_once_and_cut_the_box_count() {
        let (ink, boxes) = coverage(&["MOONLIGHT DELIVERY"], 1, 3, "center");
        assert!(boxes * 3 < ink, "{boxes} boxes for {ink} pixels");
        let (ink, boxes) = coverage(&["PARCELS 3/6", "BY MOONLIGHT!", "ok"], 2, 4, "right");
        assert!(boxes < ink, "{boxes} for {ink}");
        // A single letter is a handful of boxes, not up to thirty-five.
        assert!(coverage(&["B"], 1, 3, "center").1 <= 8);
    }

    #[test]
    fn a_sign_is_a_group_of_valid_uniquely_named_boxes_placed_like_any_object() {
        let v = text(json!({"id":"sign","type":"text","text":"DEPOT","height":0.35,"position":[2,1.5,-4],"rotation":[0,90,0],
            "material":{"color":"#ffcc00","emissive":"#553300"}}))
        .unwrap();
        assert_eq!((v["type"].as_str(), v["position"].clone(), v["rotation"].clone()), (Some("group"), json!([2, 1.5, -4]), json!([0, 90, 0])));
        assert_eq!(v["collide"], false, "lettering is decoration unless it asks otherwise");
        let boxes = kids(&v);
        let mut ids: Vec<&str> = boxes.iter().map(|b| b["id"].as_str().unwrap()).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "every piece has its own id");
        assert!(ids.iter().all(|i| i.starts_with("sign.l")), "{ids:?}");
        assert!(boxes.iter().all(|b| b["material"]["emissive"] == "#553300"), "the material reaches every letter");
        // The whole object parses as a scene (the generated ids and fields are what the parser demands).
        let scene = json!({"camera":{"position":[0,1.7,5],"target":[0,1.5,0]},"objects":[{"id":"sign","type":"text","text":"DEPOT","position":[0,1.5,0]}]});
        let parsed = crate::schema::parse_scene_in(&scene.to_string(), None).unwrap();
        assert!(matches!(&parsed.objects[0].kind, crate::schema::ObjectKind::Group(k) if !k.is_empty()));
        let solid = text(json!({"id":"s","type":"text","text":"A","collide":true})).unwrap();
        assert_eq!(solid["collide"], true);
    }

    #[test]
    fn lettering_reads_along_plus_x_with_the_top_of_the_text_up_and_stands_in_front_of_z_zero() {
        let v = text(json!({"id":"t","type":"text","text":"T","height":0.7})).unwrap();
        let boxes = kids(&v);
        let top = boxes.iter().max_by(|a, b| a["position"][1].as_f64().partial_cmp(&b["position"][1].as_f64()).unwrap()).unwrap();
        assert!((top["size"][0].as_f64().unwrap() - 0.5).abs() < 1e-3, "the bar of a T is five pixels wide, at the top: {top}");
        assert!(top["position"][1].as_f64().unwrap() > 0.25, "and above the middle");
        // Block is centred on the origin, 5 px wide and 7 px (0.7 m) tall.
        let (lo, hi) = boxes.iter().fold((f64::MAX, f64::MIN), |(lo, hi), b| {
            let (y, h) = (b["position"][1].as_f64().unwrap(), b["size"][1].as_f64().unwrap());
            (lo.min(y - h / 2.0), hi.max(y + h / 2.0))
        });
        assert!((lo + 0.35).abs() < 1e-3 && (hi - 0.35).abs() < 1e-3, "{lo} {hi}");
        assert!(boxes.iter().all(|b| b["position"][2].as_f64().unwrap() > 0.0), "the letters stand out toward +Z");
        // Lowercase is the same lettering.
        assert_eq!(kids(&text(json!({"id":"t","type":"text","text":"t","height":0.7})).unwrap()), boxes);
    }

    #[test]
    fn a_backing_board_sits_behind_the_letters_and_alignment_places_short_lines() {
        let v =
            text(json!({"id":"s","type":"text","text":"HELLO\nHI","height":0.14,"align":"left","backing":{"color":"#102030","margin":0.05,"thickness":0.03}}))
                .unwrap();
        let boxes = kids(&v);
        let back = boxes.iter().find(|b| b["id"] == "s.backing").expect("a backing board");
        // 5 letters of 5 px and 4 gaps = 29 px wide at 0.02 m, two lines = 17 px tall, plus the margin each side.
        assert!((back["size"][0].as_f64().unwrap() - (29.0 * 0.02 + 0.1)).abs() < 1e-3, "{back}");
        assert!((back["size"][1].as_f64().unwrap() - (17.0 * 0.02 + 0.1)).abs() < 1e-3, "{back}");
        assert!(back["position"][2].as_f64().unwrap() < 0.0, "behind the letters' front plane");
        let letters: Vec<&Value> = boxes.iter().filter(|b| b["id"] != "s.backing").collect();
        let left = letters.iter().map(|b| b["position"][0].as_f64().unwrap() - b["size"][0].as_f64().unwrap() / 2.0).fold(f64::MAX, f64::min);
        assert!((left + 29.0 * 0.02 / 2.0).abs() < 1e-3, "left aligned lines start at the block's left edge: {left}");
        assert!(letters.iter().all(|b| b["position"][2].as_f64().unwrap() + b["size"][2].as_f64().unwrap() / 2.0 > 0.0), "letters poke out of the board");
        assert_eq!(text(json!({"id":"s","type":"text","text":"A","backing":"#223344"})).unwrap()["children"][0]["id"], "s.backing");
    }

    #[test]
    fn a_bad_sign_says_what_is_wrong_and_what_the_font_has() {
        let err = |v: Value| text(v).unwrap_err().join("\n");
        let e = err(json!({"id":"s","type":"text","text":"CAFÉ @"}));
        assert!(e.contains("no `@`") && e.contains("`É`") && e.contains("letters, digits and"), "{e}");
        assert!(err(json!({"id":"s","type":"text"})).contains("needs \"text\""));
        assert!(err(json!({"id":"s","type":"text","text":"  "})).contains("is empty"));
        assert!(err(json!({"id":"s","type":"text","text":"A","height":0})).contains("'height'"));
        assert!(err(json!({"id":"s","type":"text","text":"A","align":"middle"})).contains("not one of left, center, right"));
        assert!(err(json!({"id":"s","type":"text","text":"A","spacing":1.5})).contains("whole numbers"));
        assert!(err(json!({"id":"s","type":"text","text":"A\n".repeat(13)})).contains("at most"));
        assert!(err(json!({"id":"s","type":"text","text":"A","backing":3})).contains(".backing"));
    }

    fn array(json: Value) -> Result<Value, Vec<String>> {
        expand("array", json.as_object().unwrap(), json["id"].as_str().unwrap())
    }

    fn all_ids(v: &Value, out: &mut Vec<String>) {
        for k in v.get("children").and_then(Value::as_array).into_iter().flatten() {
            out.push(k["id"].as_str().unwrap().to_string());
            all_ids(k, out);
        }
    }

    #[test]
    fn an_array_names_its_copies_and_steps_position_and_rotation() {
        let v = array(json!({"id":"posts","type":"array","count":4,"step":[1.5,0,0.25],"rotation_step":[0,15,0],"position":[10,0,0],
            "template":{"type":"box","size":[0.1,1,0.1],"position":[0,0.5,0],"rotation":[0,5,0],"material":{"color":"#8a6a40"}}}))
        .unwrap();
        let kids = kids(&v);
        assert_eq!(kids.iter().map(|k| k["id"].as_str().unwrap()).collect::<Vec<_>>(), ["posts.0", "posts.1", "posts.2", "posts.3"]);
        assert_eq!(kids[2]["position"], json!([3.0, 0.5, 0.5]));
        assert_eq!(kids[3]["rotation"], json!([0.0, 50.0, 0.0]), "the template's own rotation plus 15 degrees per copy");
        assert_eq!(v["position"], json!([10, 0, 0]), "the group carries the array's own placement");
        assert!(kids.iter().all(|k| k["material"]["color"] == "#8a6a40" && k["size"] == json!([0.1, 1, 0.1])));
    }

    #[test]
    fn explicit_positions_and_group_templates_get_unique_ids_the_parser_accepts() {
        let v = array(json!({"id":"lamps","type":"array","positions":[[0,2.4,0],[5,2.4,0],[5,2.4,6]],
            "template":{"type":"group","children":[{"id":"pole","type":"box","size":[0.1,2.4,0.1],"position":[0,-1.2,0]},
                                                   {"id":"head","type":"group","children":[{"id":"bulb","type":"sphere","radius":0.15}]}]}}))
        .unwrap();
        let mut ids = Vec::new();
        all_ids(&v, &mut ids);
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(n, ids.len(), "no two pieces share an id: {ids:?}");
        assert!(ids.contains(&"lamps.2.head.bulb".to_string()) && ids.contains(&"lamps.0.pole".to_string()), "{ids:?}");
        let scene = json!({"camera":{"position":[0,1.7,5],"target":[0,1,0]},"objects":[v]});
        crate::schema::parse_scene_in(&scene.to_string(), None).expect("the expanded array is a valid scene");
    }

    #[test]
    fn a_bad_array_says_what_to_give() {
        let err = |v: Value| array(v).unwrap_err().join("\n");
        let b = json!({"type":"box","size":[1,1,1]});
        assert!(err(json!({"id":"a","type":"array","count":3})).contains("needs \"template\""));
        assert!(err(json!({"id":"a","type":"array","template":{"id":"x","type":"box"},"count":2,"step":[1,0,0]})).contains("remove it"));
        assert!(err(json!({"id":"a","type":"array","template":b.clone()})).contains("needs 'count'"));
        assert!(err(json!({"id":"a","type":"array","template":b.clone(),"count":3})).contains(".step"));
        assert!(err(json!({"id":"a","type":"array","template":b.clone(),"count":0,"step":[1,0,0]})).contains("1 to 500"));
        assert!(err(json!({"id":"a","type":"array","template":b.clone(),"count":501,"step":[1,0,0]})).contains("1 to 500"));
        assert!(err(json!({"id":"a","type":"array","template":b.clone(),"count":2,"step":[1,0,0],"positions":[[0,0,0]]})).contains("not both"));
        assert!(err(json!({"id":"a","type":"array","template":b.clone(),"positions":[[0,0]]})).contains("[x, y, z]"));
        let mut keyframed = b.clone();
        keyframed["position"] = json!({"keyframes": []});
        assert!(err(json!({"id":"a","type":"array","template":keyframed,"count":2,"step":[1,0,0]})).contains("keyframed"));
        let nameless = json!({"id":"a","type":"array","count":2,"step":[1,0,0],"template":{"type":"group","children":[{"type":"box"}]}});
        assert!(err(nameless).contains("children need ids"));
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

    #[test]
    fn a_macro_option_that_is_not_a_usable_number_is_an_error_with_the_fix_not_the_default_or_a_minimum() {
        let wall = |extra: Value| {
            let mut o = json!({"type": "wall", "from": [0, 0], "to": [4, 0]});
            for (k, v) in extra.as_object().unwrap() {
                o[k] = v.clone();
            }
            expand("wall", o.as_object().unwrap(), "w").err().map(|e| e.join(" | ")).unwrap_or_default()
        };
        assert!(wall(json!({"height": "tall"})).contains("w.height: must be a number greater than 0 (got string \"tall\")"));
        assert!(wall(json!({"thickness": -0.2})).contains("w.thickness: must be a number greater than 0 (got number -0.2)"));
        assert!(wall(json!({"openings": [{"at": 1, "width": -1}]})).contains("w.openings[0].width: must be a number greater than 0"));
        assert_eq!(wall(json!({"height": 3.0, "thickness": 0.3})), "");
        let fence = expand("fence", json!({"points": [[0, 0], [4, 0]], "post_spacing": 0.1}).as_object().unwrap(), "f")
            .err()
            .map(|e| e.join(" | "))
            .unwrap_or_default();
        assert!(
            fence.contains("f.post_spacing: must be a number of at least 0.5 (got number 0.1); posts closer than 0.5 m are one solid wall: use a `wall`"),
            "{fence}"
        );
    }
}
