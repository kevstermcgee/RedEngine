//! `red_engine2 verify <scene>`: the executable feedback loop. A scene carries its own
//! expectations in a top-level `"checks"` block; `verify` runs every one against the real
//! engine code and prints PASS/FAIL with the evidence, exiting 1 on any failure — so an AI can
//! edit, run one command, and know whether it broke something without a human in the loop.
//!
//! ```json
//! "checks": {
//!   "lint":    { "max_errors": 0, "max_warnings": 3, "forbid": ["leak"] },
//!   "reach":   [ { "to": [3, -1.5], "why": "kitchen reachable" } ],
//!   "walk":    [ { "name": "front door to bedroom", "path": "0,8; 1.5,4; ...",
//!                  "ends_near": [0.5, -0.5], "tol": 0.35, "floor_y": 3.0 } ],
//!   "objects": { "exist": ["sofa_1"], "absent": ["debug_cube"], "min_count": 30,
//!                "count": [ { "kind": "prefab:chair_wooden_1", "min": 2 } ] },
//!   "views":   [ { "name": "living", "eye": [x,y,z], "at": [x,y,z], "fov": 70, "max_diff": 0.01 } ]
//! }
//! ```
//!
//! `views` are golden-image regression tests: the first run (or `--bless`) records
//! `golden/<scene>/<name>.png` next to the scene; later runs render again, and if more than
//! `max_diff` (default 1%) of pixels changed visibly the check fails and a side-by-side
//! `golden | now | diff` image is written under `out/verify/` for the AI (or human) to look at.

use super::lint::{self, Severity};
use super::reach::{self, ReachParams};
use super::shots::{self, FrameOpts};
use super::world::{load_or_report, MapWorld};
use crate::render::Renderer;
use glam::{Vec2, Vec3};
use image::{Rgb, RgbImage};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Flags for a `verify` run (bless goldens, skip views, only-filter).
#[derive(Default, Clone)]
pub struct Options {
    /// Record the current render of every view as its golden image.
    pub bless: bool,
    pub skip_views: bool,
    /// Only run checks whose name contains this.
    pub only: Option<String>,
    /// Where diff images go (default `out/verify`).
    pub out_dir: Option<PathBuf>,
}

/// Outcome of one check: name, pass/fail, evidence and an optional diff-image path.
pub struct CheckResult {
    pub name: String,
    pub ok: bool,
    pub detail: String,
    pub artifact: Option<PathBuf>,
}

/// All results of a `verify` run for one scene.
pub struct Report {
    pub scene: PathBuf,
    pub results: Vec<CheckResult>,
}

impl Report {
    /// Number of failed checks.
    pub fn failed(&self) -> usize {
        self.results.iter().filter(|r| !r.ok).count()
    }

    /// PASS/FAIL text with evidence per check.
    pub fn render(&self) -> String {
        let mut s = String::new();
        for r in &self.results {
            s.push_str(&format!("{} {}  {}\n", if r.ok { "PASS" } else { "FAIL" }, r.name, r.detail));
            if let Some(a) = &r.artifact {
                s.push_str(&format!("     see {}\n", a.display()));
            }
        }
        s.push_str(&format!("{}: {} check(s), {} failed\n", self.scene.display(), self.results.len(), self.failed()));
        s
    }

    /// The report as JSON.
    pub fn to_json(&self) -> Value {
        json!({
            "scene": self.scene.display().to_string(),
            "failed": self.failed(),
            "checks": self.results.iter().map(|r| json!({"name": r.name, "ok": r.ok, "detail": r.detail, "artifact": r.artifact.as_ref().map(|p| p.display().to_string())})).collect::<Vec<_>>(),
        })
    }
}

fn pass(name: impl Into<String>, detail: impl Into<String>) -> CheckResult {
    CheckResult { name: name.into(), ok: true, detail: detail.into(), artifact: None }
}
fn fail(name: impl Into<String>, detail: impl Into<String>) -> CheckResult {
    CheckResult { name: name.into(), ok: false, detail: detail.into(), artifact: None }
}

fn f32s(v: &Value) -> Option<Vec<f32>> {
    v.as_array()?.iter().map(|x| x.as_f64().map(|f| f as f32)).collect()
}

fn v2(v: &Value) -> Option<Vec2> {
    let a = f32s(v)?;
    (a.len() >= 2).then(|| Vec2::new(a[0], a[1]))
}

fn parse_path(s: &str) -> Result<Vec<Vec2>, String> {
    s.split(';')
        .filter(|p| !p.trim().is_empty())
        .map(|p| {
            let n: Result<Vec<f32>, _> = p.split(',').map(|x| x.trim().parse::<f32>()).collect();
            match n {
                Ok(n) if n.len() == 2 => Ok(Vec2::new(n[0], n[1])),
                _ => Err(format!("bad waypoint '{p}' (expected x,z)")),
            }
        })
        .collect()
}

fn selected(opts: &Options, name: &str) -> bool {
    opts.only.as_deref().is_none_or(|o| name.contains(o))
}

/// Runs every check in the scene's `checks` block.
pub fn run(path: &Path, opts: &Options) -> Result<Report, String> {
    let world = load_or_report(path)?;
    let checks = world.raw.get("checks").cloned().unwrap_or(Value::Null);
    let mut results = Vec::new();
    if checks.is_null() {
        results.push(fail("checks", "the scene has no top-level \"checks\" block (see `red_engine2 describe scene`); at minimum add {\"lint\": {\"max_errors\": 0}}"));
        return Ok(Report { scene: path.to_path_buf(), results });
    }
    let r = reach::compute(&world, &ReachParams::default());
    let findings = lint::lint(&world, &r);

    if let Some(l) = checks.get("lint").filter(|_| selected(opts, "lint")) {
        let max_e = l.get("max_errors").and_then(Value::as_u64).unwrap_or(0) as usize;
        let max_w = l.get("max_warnings").and_then(Value::as_u64);
        let forbid: Vec<&str> = l.get("forbid").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let errs: Vec<&lint::Finding> = findings.iter().filter(|f| f.sev == Severity::Error).collect();
        let warns = findings.iter().filter(|f| f.sev == Severity::Warn).count();
        let forbidden: Vec<&lint::Finding> = findings.iter().filter(|f| forbid.contains(&f.code)).collect();
        let ok = errs.len() <= max_e && max_w.is_none_or(|m| warns as u64 <= m) && forbidden.is_empty();
        let mut detail = format!("{} error(s) (max {max_e}), {warns} warning(s){}", errs.len(), max_w.map(|m| format!(" (max {m})")).unwrap_or_default());
        if !ok {
            for f in errs.iter().chain(forbidden.iter()).take(6) {
                detail.push_str(&format!("\n     [{}] {}", f.code, f.message));
            }
        }
        results.push(CheckResult { name: "lint".into(), ok, detail, artifact: None });
    }

    if let Some(arr) = checks.get("reach").and_then(Value::as_array).filter(|_| selected(opts, "reach")) {
        for (i, c) in arr.iter().enumerate() {
            let name = format!("reach[{i}]{}", c.get("why").and_then(Value::as_str).map(|w| format!(" {w}")).unwrap_or_default());
            let Some(to) = c.get("to").and_then(f32s).filter(|t| t.len() == 2 || t.len() == 3) else {
                results.push(fail(name, "needs \"to\": [x,z] or [x,z,y]"));
                continue;
            };
            let rr = match c.get("from").and_then(v2) {
                Some(s) => reach::compute(&world, &ReachParams { start: Some(s), ..Default::default() }),
                None => reach::compute(&world, &ReachParams::default()),
            };
            let p = Vec2::new(to[0], to[1]);
            let ok = if to.len() == 3 { rr.reachable(p, to[2], 0.3) } else { !rr.levels_at(p).is_empty() };
            results.push(if ok { pass(name, format!("({:.1}, {:.1}) reachable", p.x, p.y)) } else { fail(name, format!("({:.1}, {:.1}) is NOT reachable from the start", p.x, p.y)) });
        }
    }

    if let Some(arr) = checks.get("walk").and_then(Value::as_array).filter(|_| selected(opts, "walk")) {
        for (i, c) in arr.iter().enumerate() {
            let name = format!("walk[{i}] {}", c.get("name").and_then(Value::as_str).unwrap_or(""));
            let Some(path_s) = c.get("path").and_then(Value::as_str) else {
                results.push(fail(name, "needs \"path\": \"x,z; x,z; ...\""));
                continue;
            };
            let wps = match parse_path(path_s) {
                Ok(w) if !w.is_empty() => w,
                Ok(_) => {
                    results.push(fail(name, "empty path"));
                    continue;
                }
                Err(e) => {
                    results.push(fail(name, e));
                    continue;
                }
            };
            let start = c.get("from").and_then(v2).unwrap_or(world.spawn);
            let steps = super::walk::walk(&world, start, &wps);
            if steps.len() < wps.len() || steps.last().is_some_and(|l| !l.reached) {
                let last = steps.last();
                results.push(fail(name, match last {
                    Some(l) => format!("stuck on leg {} toward ({:.1}, {:.1}); stopped at ({:.2}, {:.2}) y={:.2}", steps.len(), l.target.x, l.target.y, l.pos.x, l.pos.y, l.foot_y),
                    None => "could not start".to_string(),
                }));
                continue;
            }
            let end = steps.last().unwrap();
            let want = c.get("ends_near").and_then(v2).unwrap_or(*wps.last().unwrap());
            let tol = c.get("tol").and_then(Value::as_f64).unwrap_or(0.35) as f32;
            let dist = (end.pos - want).length();
            let floor_ok = c.get("floor_y").and_then(Value::as_f64).is_none_or(|fy| (end.foot_y - fy as f32).abs() <= 0.2);
            let ok = dist <= tol && floor_ok;
            let detail = format!("ended ({:.2}, {:.2}) y={:.2}; wanted within {tol} of ({:.1}, {:.1}){}", end.pos.x, end.pos.y, end.foot_y, want.x, want.y, c.get("floor_y").and_then(Value::as_f64).map(|f| format!(" at y={f}")).unwrap_or_default());
            results.push(if ok { pass(name, detail) } else { fail(name, detail) });
        }
    }

    if let Some(o) = checks.get("objects").filter(|_| selected(opts, "objects")) {
        results.extend(check_objects(&world, o));
    }

    if let Some(arr) = checks.get("views").and_then(Value::as_array).filter(|_| selected(opts, "view") && !opts.skip_views) {
        results.extend(check_views(path, arr, opts)?);
    }

    if results.is_empty() {
        results.push(fail("checks", "no check ran (unknown keys or --only matched nothing). Known: lint, reach, walk, objects, views"));
    }
    Ok(Report { scene: path.to_path_buf(), results })
}

fn kind_of(o: &Value) -> String {
    match o.get("type").and_then(Value::as_str) {
        Some("prop") => format!("prop:{}", o.get("prop").and_then(Value::as_str).unwrap_or("?")),
        Some("prefab") => format!("prefab:{}", o.get("prefab").and_then(Value::as_str).unwrap_or("?")),
        Some(t) => t.to_string(),
        None => "?".to_string(),
    }
}

fn check_objects(world: &MapWorld, o: &Value) -> Vec<CheckResult> {
    let mut out = Vec::new();
    let objs: Vec<&Value> = world.raw.get("objects").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
    let ids: Vec<&str> = objs.iter().filter_map(|x| x.get("id").and_then(Value::as_str)).collect();
    let strs = |k: &str| -> Vec<&str> { o.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default() };
    let missing: Vec<&str> = strs("exist").into_iter().filter(|e| !ids.contains(e)).collect();
    if o.get("exist").is_some() {
        out.push(if missing.is_empty() { pass("objects.exist", format!("{} required object(s) present", strs("exist").len())) } else { fail("objects.exist", format!("missing: {}", missing.join(", "))) });
    }
    let present: Vec<&str> = strs("absent").into_iter().filter(|e| ids.contains(e)).collect();
    if o.get("absent").is_some() {
        out.push(if present.is_empty() { pass("objects.absent", "none of the forbidden objects exist") } else { fail("objects.absent", format!("should not exist: {}", present.join(", "))) });
    }
    let n = objs.len();
    if let Some(min) = o.get("min_count").and_then(Value::as_u64) {
        out.push(if n as u64 >= min { pass("objects.min_count", format!("{n} top-level objects (>= {min})")) } else { fail("objects.min_count", format!("only {n} top-level objects, expected >= {min}")) });
    }
    if let Some(max) = o.get("max_count").and_then(Value::as_u64) {
        out.push(if n as u64 <= max { pass("objects.max_count", format!("{n} top-level objects (<= {max})")) } else { fail("objects.max_count", format!("{n} top-level objects, expected <= {max}")) });
    }
    for (i, c) in o.get("count").and_then(Value::as_array).into_iter().flatten().enumerate() {
        let kind = c.get("kind").and_then(Value::as_str).unwrap_or("");
        let have = objs.iter().filter(|x| kind_of(x) == kind).count();
        let min = c.get("min").and_then(Value::as_u64).unwrap_or(0) as usize;
        let max = c.get("max").and_then(Value::as_u64).map(|m| m as usize);
        let ok = have >= min && max.is_none_or(|m| have <= m);
        let name = format!("objects.count[{i}] {kind}");
        let detail = format!("{have} (min {min}{})", max.map(|m| format!(", max {m}")).unwrap_or_default());
        out.push(if ok { pass(name, detail) } else { fail(name, detail) });
    }
    out
}

/// Fraction of pixels whose largest channel difference exceeds `thresh` (0..255).
pub fn diff_fraction(a: &RgbImage, b: &RgbImage, thresh: u8) -> f32 {
    if a.dimensions() != b.dimensions() {
        return 1.0;
    }
    let bad = a.pixels().zip(b.pixels()).filter(|(p, q)| p.0.iter().zip(q.0.iter()).any(|(x, y)| x.abs_diff(*y) > thresh)).count();
    bad as f32 / (a.width() * a.height()) as f32
}

fn triptych(golden: &RgbImage, now: &RgbImage) -> RgbImage {
    let (w, h) = now.dimensions();
    let mut out = RgbImage::from_pixel(w * 3, h, Rgb([12, 14, 18]));
    for y in 0..h {
        for x in 0..w {
            let n = *now.get_pixel(x, y);
            let g = if golden.dimensions() == now.dimensions() { *golden.get_pixel(x, y) } else { Rgb([40, 0, 40]) };
            out.put_pixel(x, y, g);
            out.put_pixel(w + x, y, n);
            let d = n.0.iter().zip(g.0.iter()).map(|(a, b)| a.abs_diff(*b) as u32).max().unwrap_or(0);
            let v = (d * 4).min(255) as u8;
            out.put_pixel(2 * w + x, y, if d > 24 { Rgb([255, v, 0]) } else { Rgb([v / 4, v / 4, v / 4]) });
        }
    }
    out
}

fn check_views(path: &Path, arr: &[Value], opts: &Options) -> Result<Vec<CheckResult>, String> {
    let mut out = Vec::new();
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "scene".into());
    let golden_dir = path.parent().map(Path::to_path_buf).unwrap_or_default().join("golden").join(&stem);
    let out_dir = opts.out_dir.clone().unwrap_or_else(|| PathBuf::from("out/verify"));
    let mut base = shots::prepare(load_or_report(path)?, &FrameOpts { size: Some((320, 240)), ..Default::default() });
    let mut renderer = Renderer::new(&base).map_err(|e| e.to_string())?;
    for (i, v) in arr.iter().enumerate() {
        let name = v.get("name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("view{i}"));
        let label = format!("view {name}");
        let (Some(eye), Some(at)) = (v.get("eye").and_then(f32s).filter(|e| e.len() == 3), v.get("at").and_then(f32s).filter(|e| e.len() == 3)) else {
            out.push(fail(label, "needs \"eye\": [x,y,z] and \"at\": [x,y,z]"));
            continue;
        };
        base.camera.position = crate::track::Track::constant(Vec3::new(eye[0], eye[1], eye[2]));
        base.camera.target = crate::track::Track::constant(Vec3::new(at[0], at[1], at[2]));
        base.camera.fov = crate::track::Track::constant(v.get("fov").and_then(Value::as_f64).unwrap_or(70.0) as f32);
        let rgb = renderer.render_frame(&base, 0.0);
        let now = RgbImage::from_raw(base.width, base.height, rgb).ok_or("frame size mismatch")?;
        let gpath = golden_dir.join(format!("{name}.png"));
        if opts.bless || !gpath.exists() {
            std::fs::create_dir_all(&golden_dir).map_err(|e| e.to_string())?;
            now.save(&gpath).map_err(|e| e.to_string())?;
            out.push(pass(label, format!("{} golden {}", if opts.bless { "blessed" } else { "no golden yet — recorded" }, gpath.display())));
            continue;
        }
        let golden = image::open(&gpath).map_err(|e| format!("{}: {e}", gpath.display()))?.to_rgb8();
        let max_diff = v.get("max_diff").and_then(Value::as_f64).unwrap_or(0.01) as f32;
        let frac = diff_fraction(&golden, &now, 24);
        if frac <= max_diff {
            out.push(pass(label, format!("{:.2}% of pixels changed (max {:.1}%)", frac * 100.0, max_diff * 100.0)));
        } else {
            std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
            let dpath = out_dir.join(format!("{stem}_{name}_diff.png"));
            triptych(&golden, &now).save(&dpath).map_err(|e| e.to_string())?;
            let mut r = fail(label, format!("{:.2}% of pixels changed (max {:.1}%). If the change is intended: `verify --bless`", frac * 100.0, max_diff * 100.0));
            r.artifact = Some(dpath);
            out.push(r);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(checks: &str) -> String {
        format!(
            r##"{{"camera":{{"fov":60,"position":[0,1.7,0],"target":[0,1.5,5]}},
            "objects":[
              {{"id":"wall_n","type":"wall","from":[-4,-4],"to":[4,-4],"height":2.8}},
              {{"id":"wall_s","type":"wall","from":[-4,4],"to":[4,4],"height":2.8}},
              {{"id":"wall_w","type":"wall","from":[-4,-4],"to":[-4,4],"height":2.8}},
              {{"id":"wall_e","type":"wall","from":[4,-4],"to":[4,4],"height":2.8}},
              {{"id":"floor","type":"plane","size":[8,8],"position":[0,0.01,0],"material":{{"color":"#886644"}}}},
              {{"id":"crate_1","type":"prop","prop":"crate","position":[2,0,2]}}
            ],"checks":{checks}}}"##
        )
    }

    fn run_text(checks: &str) -> Report {
        let dir = std::env::temp_dir().join("re2_verify_tests");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("t{}.json", checks.len()));
        std::fs::write(&p, scene(checks)).unwrap();
        run(&p, &Options { skip_views: true, ..Default::default() }).unwrap()
    }

    #[test]
    fn passing_checks_pass_and_failing_ones_fail_with_evidence() {
        let r = run_text(r#"{"walk":[{"name":"cross","path":"0,0; 3,-3"}],"objects":{"exist":["crate_1"],"min_count":5,"count":[{"kind":"prop:crate","min":1,"max":1}]},"reach":[{"to":[2,-2]}]}"#);
        assert_eq!(r.failed(), 0, "{}", r.render());
        let r = run_text(r#"{"objects":{"exist":["nope"],"absent":["crate_1"]},"reach":[{"to":[40,40]}]}"#);
        assert_eq!(r.failed(), 3, "{}", r.render());
        assert!(r.render().contains("missing: nope"));
    }

    #[test]
    fn a_walk_that_ends_away_from_where_it_should_fails() {
        let r = run_text(r#"{"walk":[{"name":"x","path":"0,0; 3,-3","ends_near":[-3,3],"tol":0.3}]}"#);
        assert_eq!(r.failed(), 1, "{}", r.render());
    }

    #[test]
    fn a_scene_without_checks_reports_how_to_add_them() {
        let dir = std::env::temp_dir().join("re2_verify_tests");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("nochecks.json");
        std::fs::write(&p, r#"{"camera":{},"objects":[]}"#).unwrap();
        let r = run(&p, &Options::default()).unwrap();
        assert_eq!(r.failed(), 1);
        assert!(r.render().contains("checks"));
    }

    #[test]
    fn diff_fraction_counts_visibly_changed_pixels() {
        let a = RgbImage::from_pixel(10, 10, Rgb([100, 100, 100]));
        let mut b = a.clone();
        assert_eq!(diff_fraction(&a, &b, 24), 0.0);
        for x in 0..5 {
            b.put_pixel(x, 0, Rgb([200, 100, 100]));
        }
        assert!((diff_fraction(&a, &b, 24) - 0.05).abs() < 1e-6);
        b.put_pixel(9, 9, Rgb([110, 100, 100])); // below threshold: ignored
        assert!((diff_fraction(&a, &b, 24) - 0.05).abs() < 1e-6);
    }
}
