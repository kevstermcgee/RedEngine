//! Map analysis and checks: validate, lint, reach, plan, walk, info, diff, verify, and the headless sim/replay commands.

use super::*;

pub(crate) fn analyze(scene: &Path, cell: f32, phase: Option<&str>) -> Result<(MapWorld, reach::Reach, Vec<lint::Finding>), String> {
    let world = load_or_report_phase(scene, phase)?;
    let r = reach::compute(&world, &ReachParams { cell, ..Default::default() });
    let findings = lint::lint_phases(&world, &r, cell);
    Ok((world, r, findings))
}

pub(crate) fn run_validate(scene: &Path) -> Result<(), String> {
    match red_engine2::validate_scene_file(scene) {
        Ok(()) => {
            println!("OK: scene is valid");
            Ok(())
        }
        Err(errs) => {
            for e in &errs {
                eprintln!("error: {e}");
            }
            Err(format!("{} error(s)", errs.len()))
        }
    }
}

pub(crate) fn run_lint(scene: &Path, json: bool, strict: bool, cell: f32, phase: Option<&str>) -> Result<(), String> {
    let (world, r, findings) = analyze(scene, cell, phase)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&lint::to_json(&findings)).unwrap());
    } else {
        let solid = world.items.iter().filter(|i| i.is_solid()).count();
        println!(
            "{}: {} objects, {} solid pieces, {} zone(s); walkable {:.0} m^2 across {} floor(s)",
            scene.display(),
            world.scene.objects.len(),
            solid,
            world.zones.len(),
            r.total_area(),
            r.floors().len()
        );
        print!("{}", lint::format_report(&findings));
    }
    let errors = findings.iter().filter(|f| f.sev == Severity::Error).count();
    let warns = findings.iter().filter(|f| f.sev == Severity::Warn).count();
    if errors > 0 || (strict && warns > 0) {
        return Err(String::new());
    }
    Ok(())
}

pub(crate) fn run_reach(scene: &Path, from: Option<&str>, to: Option<&str>, cell: f32, json: bool, phase: Option<&str>) -> Result<(), String> {
    let world = load_or_report_phase(scene, phase)?;
    let start = from.map(v2).transpose()?;
    let r = reach::compute(&world, &ReachParams { cell, start, ..Default::default() });
    if !r.start_ok {
        return Err(format!("the start ({:.1}, {:.1}) is inside solid geometry", r.start.x, r.start.y));
    }
    if let Some(t) = to {
        let v = floats(t)?;
        if v.len() < 2 || v.len() > 3 {
            return Err("--to must be x,z or x,z,y".to_string());
        }
        let p = Vec2::new(v[0] as f32, v[1] as f32);
        let heights: Vec<f32> = r.levels_at(p);
        let ok = if v.len() == 3 { r.reachable(p, v[2] as f32, 0.3) } else { !heights.is_empty() };
        println!(
            "({:.1}, {:.1}{}) is {}{}",
            p.x,
            p.y,
            if v.len() == 3 { format!(", y={:.1}", v[2]) } else { String::new() },
            if ok { "REACHABLE" } else { "NOT reachable" },
            if heights.is_empty() {
                String::new()
            } else {
                format!("  (standing heights there: {})", heights.iter().map(|h| format!("{h:.2}")).collect::<Vec<_>>().join(", "))
            }
        );
        return if ok { Ok(()) } else { Err(String::new()) };
    }
    let floors = r.floors();
    let mut zones_json = Vec::new();
    let mut text = String::new();
    text.push_str(&format!("reach from ({:.1}, {:.1}) y={:.2}   grid {:.2} m\n", r.start.x, r.start.y, r.start_y, r.cell));
    text.push_str("floors the player can stand on:\n");
    for (y, area) in &floors {
        text.push_str(&format!("  y={y:.1}   {area:.0} m^2\n"));
    }
    if !world.zones.is_empty() {
        text.push_str("zones:\n");
        for z in &world.zones {
            let (got, free) = r.area_in(&world, z.min, z.max, z.y, 0.35);
            let pct = if free > 0.0 { got / free * 100.0 } else { 0.0 };
            let status = if free < 0.5 {
                "no floor?"
            } else if got < 0.5 {
                "UNREACHABLE"
            } else if pct < 60.0 {
                "PARTLY SEALED"
            } else {
                "ok"
            };
            text.push_str(&format!("  {:<18} y={:<4.1} {:>5.1} / {:<5.1} m^2  {:>3.0}%  {}\n", z.id, z.y, got, free, pct, status));
            zones_json.push(serde_json::json!({"zone": z.id, "y": z.y, "reachable_m2": got, "free_m2": free, "status": status}));
        }
        let passages = r.passages(&world.zones);
        if !passages.is_empty() {
            text.push_str("connections (doorways/openings, same floor):\n");
            for (a, b, mid, w) in &passages {
                text.push_str(&format!("  {a} <-> {b}   ~{w:.2} m wide at ({:.1}, {:.1})\n", mid.x, mid.y));
            }
        }
    }
    for it in world.items.iter().filter(|i| i.stairs.is_some()) {
        let s = it.stairs.unwrap();
        let (bot, top) = (s.point(-0.5, 0.0), s.point(s.run + 0.5, 0.0));
        let (bok, tok) = (r.reachable(bot, s.base_y(), 0.2), r.reachable(top, s.base_y() + s.rise, 0.2));
        text.push_str(&format!(
            "stairs '{}': bottom ({:.1},{:.1}) y={:.1} {}   top ({:.1},{:.1}) y={:.1} {}\n",
            it.top_id,
            bot.x,
            bot.y,
            s.base_y(),
            if bok { "reachable" } else { "NOT reachable" },
            top.x,
            top.y,
            s.base_y() + s.rise,
            if tok { "reachable" } else { "NOT reachable" }
        ));
    }
    let drops = r.drop_clusters();
    if drops.is_empty() {
        text.push_str("drops: none\n");
    } else {
        for (p, from, to, n) in &drops {
            text.push_str(&format!("drop: {:.1} m at ({:.1}, {:.1}) from y={:.1} to y={:.1} (~{n} cells)\n", from - to, p.x, p.y, from, to));
        }
    }
    text.push_str(if r.leaks.is_empty() {
        "perimeter: sealed (the player cannot leave the map)\n"
    } else {
        "perimeter: LEAKS (the player can walk off the map)\n"
    });
    if json {
        println!(
            "{}",
            serde_json::json!({"floors": floors.iter().map(|(y, a)| serde_json::json!({"y": y, "area_m2": a})).collect::<Vec<_>>(), "zones": zones_json, "leaks": r.leaks.len(), "drops": drops.len()})
        );
    } else {
        print!("{text}");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_plan(
    scene: &Path,
    out: Option<PathBuf>,
    y: f32,
    all_floors: bool,
    ascii: bool,
    ascii_cell: f32,
    scale: f32,
    bounds: Option<&str>,
    labels: &str,
    no_reach: bool,
    no_lint: bool,
    phase: Option<&str>,
) -> Result<(), String> {
    let (world, r, findings) = analyze(scene, 0.1, phase)?;
    let findings = if no_lint { vec![] } else { findings };
    let labels = match labels {
        "auto" => Labels::Auto,
        "all" => Labels::All,
        "none" => Labels::None,
        other => return Err(format!("--labels must be auto, all, or none (got '{other}')")),
    };
    let bounds = bounds.map(rect).transpose()?;
    let mut heights = vec![y];
    if all_floors {
        heights = r.floors().iter().map(|f| f.0).collect();
        if heights.is_empty() {
            heights.push(0.0);
        }
    }
    for h in heights {
        let opts = PlanOptions { y: h, scale, bounds, labels, show_reach: !no_reach, show_findings: !no_lint, overlays: Vec::new() };
        if ascii {
            print!("{}", plan::render_ascii(&world, Some(&r), &findings, &opts, ascii_cell));
            continue;
        }
        let path = match (&out, all_floors) {
            (Some(p), false) => p.clone(),
            (Some(p), true) => {
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("plan");
                p.with_file_name(format!("{stem}_y{h:.1}.png"))
            }
            (None, _) => PathBuf::from(format!("out/plan_y{h:.1}.png")),
        };
        let img = plan::render_png(&world, Some(&r), &findings, &opts);
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
        }
        img.save(&path).map_err(|e| e.to_string())?;
        println!("wrote {} ({}x{})", path.display(), img.width(), img.height());
    }
    if !ascii && !findings.is_empty() {
        println!("findings marked on the plan (numbered in the order of `lint`'s warnings/errors):");
        for (n, f) in findings.iter().filter(|f| f.sev >= Severity::Warn).enumerate() {
            println!("  {}: [{}] {}", n + 1, f.code, f.message);
        }
    }
    Ok(())
}

/// `x,z` or `x,z,y` -> (position, optional floor height).
fn v2y(s: &str) -> Result<(Vec2, Option<f32>), String> {
    let v = floats(s)?;
    match v.len() {
        2 => Ok((Vec2::new(v[0] as f32, v[1] as f32), None)),
        3 => Ok((Vec2::new(v[0] as f32, v[1] as f32), Some(v[2] as f32))),
        _ => Err(format!("expected x,z or x,z,y but got '{s}'")),
    }
}

pub(crate) fn run_walk(
    scene: &Path,
    path: Option<&str>,
    from: Option<&str>,
    auto: bool,
    to: Option<&str>,
    cell: f32,
    explain: Option<&Path>,
    phase: Option<&str>,
) -> Result<(), String> {
    use red_engine2::tools::pathing;
    let world = load_or_report_phase(scene, phase)?;
    let (start, start_y) = from.map(v2y).transpose()?.unwrap_or((world.spawn, Some(world.spawn_y)));
    let json = envelope::capturing();
    let write_explain =
        |wps: &[Vec2], steps: &[red_engine2::tools::walk::WalkStep], diag: Option<&pathing::Diagnosis>, rr: Option<&reach::Reach>| -> Result<(), String> {
            if let Some(out) = explain {
                let img = pathing::explain_image(&world, start, wps, steps, diag, rr);
                if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
                    std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
                }
                img.save(out).map_err(|e| format!("{}: {e}", out.display()))?;
                eprintln!("wrote {} (yellow = route, red = where it stopped, red boxes = what is in the way)", out.display());
            }
            Ok(())
        };

    if auto || (path.is_none() && to.is_some()) {
        let to_s = to.ok_or("--auto needs --to X,Z[,Y]")?;
        let (dest, to_y) = v2y(to_s)?;
        let opts = pathing::RouteOptions { cell, to_y, from_y: start_y };
        return match pathing::plan_route(&world, start, dest, &opts) {
            Ok(route) => {
                let ticks: u32 = route.steps.iter().map(|s| s.ticks).sum();
                let end = route.steps.last().map(|s| (s.pos, s.foot_y)).unwrap_or((start, 0.0));
                if json {
                    println!(
                        "{}",
                        serde_json::json!({"ok": true, "auto": true, "path": route.path_string(),
                            "waypoints": route.waypoints.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>(),
                            "length_m": (route.length * 10.0).round() / 10.0, "margin_m": route.margin, "ticks": ticks,
                            "ends_at": [end.0.x, end.0.y], "floor_y": (end.1 * 100.0).round() / 100.0,
                            "check": {"name": "auto route", "from": [start.x, start.y], "path": route.path_string(), "ends_near": [dest.x, dest.y]}})
                    );
                } else {
                    println!(
                        "route OK: {} waypoint(s), {:.1} m, {} ticks (~{:.1} s walking), planned with {:.2} m extra body margin",
                        route.waypoints.len(),
                        route.length,
                        ticks,
                        ticks as f32 / 60.0,
                        route.margin
                    );
                    println!("  --path \"{}\"", route.path_string());
                    println!("  ends ({:.2}, {:.2}) at y={:.2}", end.0.x, end.0.y, end.1);
                    println!(
                        "  as a scene check: {{\"name\": \"...\", \"from\": [{:.2}, {:.2}], \"path\": \"{}\", \"ends_near\": [{:.2}, {:.2}]}}",
                        start.x,
                        start.y,
                        route.path_string(),
                        dest.x,
                        dest.y
                    );
                    println!(
                        "  or let verify plan it every run: {{\"name\": \"...\", \"from\": [{:.2}, {:.2}], \"to\": [{:.2}, {:.2}], \"auto\": true}}",
                        start.x, start.y, dest.x, dest.y
                    );
                }
                write_explain(&route.waypoints, &route.steps, None, None)?;
                Ok(())
            }
            Err(e) => {
                // Say why: walk the straight line and diagnose where it stops.
                let steps = red_engine2::tools::walk::walk_from(&world, start, start_y.unwrap_or(0.0), &[dest]);
                // One flood from the stop point serves the diagnosis and the picture.
                let rr = steps.last().filter(|s| !s.reached).map(|s| reach::compute(&world, &ReachParams { start: Some(s.pos), ..Default::default() }));
                let diag = steps.last().filter(|s| !s.reached).zip(rr.as_ref()).map(|(s, r)| pathing::diagnose_with(&world, s.pos, s.foot_y, dest, r));
                if json {
                    println!("{}", serde_json::json!({"ok": false, "auto": true, "error": e, "diagnosis": diag.as_ref().map(|d| d.to_json())}));
                } else if let Some(d) = &diag {
                    print!("{}", d.render());
                }
                write_explain(&[dest], &steps, diag.as_ref(), rr.as_ref())?;
                Err(format!("no route: {e}"))
            }
        };
    }

    let path = path.ok_or("give --path \"x,z; x,z\" to replay a route, or --auto --to X,Z to plan one")?;
    let wps: Vec<Vec2> = path.split(';').filter(|p| !p.trim().is_empty()).map(v2).collect::<Result<_, _>>()?;
    if wps.is_empty() {
        return Err("--path needs at least one waypoint".to_string());
    }
    let steps = red_engine2::tools::walk::walk_from(&world, start, start_y.unwrap_or(0.0), &wps);
    let failed = steps.len() < wps.len() || steps.last().is_some_and(|l| !l.reached);
    let rr = steps.last().filter(|s| !s.reached).map(|s| reach::compute(&world, &ReachParams { start: Some(s.pos), ..Default::default() }));
    let diag = steps.last().filter(|s| !s.reached).zip(rr.as_ref()).map(|(s, r)| pathing::diagnose_with(&world, s.pos, s.foot_y, s.target, r));
    if json {
        let mut v = red_engine2::tools::walk::to_json(&steps, wps.len());
        v["diagnosis"] = diag.as_ref().map(|d| d.to_json()).unwrap_or(Value::Null);
        println!("{v}");
    } else {
        print!("{}", red_engine2::tools::walk::format_walk(&steps, wps.len()));
        if let Some(d) = &diag {
            print!("{}", d.render());
        }
    }
    write_explain(&wps, &steps, diag.as_ref(), rr.as_ref())?;
    if failed {
        return Err(String::new());
    }
    Ok(())
}

pub(crate) fn run_info(scene: &Path, id: &str) -> Result<(), String> {
    let (world, _r, findings) = analyze(scene, 0.1, None)?;
    print!("{}", inspect::object_info(&world, id, &findings)?);
    Ok(())
}

pub(crate) fn run_diff(a: &Path, b: Option<&Path>, git: bool) -> Result<(), String> {
    let read = |p: &Path| -> Result<Value, String> {
        serde_json::from_str(&std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?).map_err(|e| format!("{}: {e}", p.display()))
    };
    let (old, new) = match (b, git) {
        (Some(b), false) => (read(a)?, read(b)?),
        (None, true) => {
            let abs = a.canonicalize().map_err(|e| format!("{}: {e}", a.display()))?;
            let dir = abs.parent().ok_or("no parent dir")?;
            let file = abs.file_name().ok_or("no file name")?.to_string_lossy().to_string();
            let out =
                std::process::Command::new("git").arg("-C").arg(dir).arg("show").arg(format!("HEAD:./{file}")).output().map_err(|e| format!("git: {e}"))?;
            if !out.status.success() {
                return Err(format!("git could not show HEAD:{file}: {}", String::from_utf8_lossy(&out.stderr).trim()));
            }
            (serde_json::from_slice(&out.stdout).map_err(|e| format!("HEAD version is not valid JSON: {e}"))?, read(a)?)
        }
        _ => return Err("give two scene files, or one file with --git".to_string()),
    };
    print!("{}", red_engine2::tools::diff::diff(&old, &new).text);
    Ok(())
}

pub(crate) fn run_verify(scene: &Path, bless: bool, no_views: bool, only: Option<String>, out_dir: Option<PathBuf>, json: bool) -> Result<(), String> {
    let report = verify::run(scene, &verify::Options { bless, skip_views: no_views, only, out_dir })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report.to_json()).unwrap());
    } else {
        print!("{}", report.render());
    }
    if report.failed() > 0 {
        return Err(String::new());
    }
    Ok(())
}

pub(crate) fn run_features(query: &[String], check: bool) -> Result<(), String> {
    use red_engine2::tools::features;
    let root = std::env::current_dir().map_err(|e| e.to_string())?;
    let all = features::load_at(&root)?;
    if check {
        let problems = features::check(&all, &root);
        if envelope::capturing() {
            println!("{}", serde_json::json!({"ok": problems.is_empty(), "features": all.len(), "problems": problems}));
        } else if problems.is_empty() {
            println!("docs/features.json is true: {} features, every listed file, suite and doc exists, every source file has an owner", all.len());
        } else {
            for p in &problems {
                println!("  {p}");
            }
            println!("{} problem(s) in docs/features.json", problems.len());
        }
        return if problems.is_empty() { Ok(()) } else { Err(String::new()) };
    }
    let text = query.join(" ");
    if text.is_empty() {
        if envelope::capturing() {
            println!("{}", serde_json::json!(all.iter().map(|f| serde_json::json!({"name": f.name, "summary": f.summary, "files": f.files, "tests": f.tests, "commands": f.commands, "docs": f.docs, "depends_on": f.depends_on})).collect::<Vec<_>>()));
        } else {
            print!("{}", features::render_list(&all));
        }
        return Ok(());
    }
    if let Some(f) = all.iter().find(|f| f.name == text) {
        print!("{}", features::render_one(f));
        return Ok(());
    }
    let hits = features::find(&all, &text);
    if hits.is_empty() {
        return Err(format!("no feature matches '{text}' (run `features` for the list)"));
    }
    for f in hits {
        print!("{}\n", features::render_one(f));
    }
    Ok(())
}

pub(crate) fn run_impact(files: &[String], git: Option<&str>) -> Result<(), String> {
    use red_engine2::tools::features;
    let all = features::load_at(&std::env::current_dir().map_err(|e| e.to_string())?)?;
    let changed: Vec<String> = match git {
        Some(base) => features::changed_files(&std::env::current_dir().map_err(|e| e.to_string())?, base)?,
        None => files.to_vec(),
    };
    if changed.is_empty() {
        return Err("no changed files: name some, or use --git in a repository with changes".to_string());
    }
    let i = features::impact(&all, &changed);
    if envelope::capturing() {
        println!("{}", features::impact_json(&i, &changed));
    } else {
        print!("{}", features::render_impact(&i, changed.len()));
    }
    Ok(())
}

/// Windows cannot delete or overwrite a running `.exe`, and the steps of a plan rebuild `red_engine2.exe` (the integration tests run it), which would
/// fail with "failed to remove file". A running exe *can* be renamed, so move ours out of cargo's way; [`restore_exe`] puts it back when cargo did not
/// rebuild it. Only done for a binary inside the cargo target directory.
fn move_exe_aside(root: &Path) -> Option<(PathBuf, PathBuf)> {
    let me = std::env::current_exe().ok()?;
    let target = std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| root.join("target"));
    let inside = |p: &Path, t: &Path| std::fs::canonicalize(p).ok().zip(std::fs::canonicalize(t).ok()).is_some_and(|(p, t)| p.starts_with(t));
    if !inside(&me, &target) {
        return None;
    }
    let dir = me.parent()?;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().contains(".running-") {
                let _ = std::fs::remove_file(e.path()); // left by an earlier run; refused by the OS while that run is alive
            }
        }
    }
    let aside = dir.join(format!("{}.running-{}.exe", me.file_stem()?.to_string_lossy(), std::process::id()));
    std::fs::rename(&me, &aside).ok()?;
    Some((me, aside))
}

/// Undoes [`move_exe_aside`] unless cargo already produced a new binary at the original path.
fn restore_exe(moved: Option<(PathBuf, PathBuf)>) {
    if let Some((orig, aside)) = moved {
        if !orig.exists() {
            let _ = std::fs::rename(&aside, &orig);
        }
    }
}

/// What `affected` was asked to do besides which files changed.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AffectedFlags {
    pub quick: bool,
    pub full: bool,
    pub dry_run: bool,
    pub keep_going: bool,
    pub no_cache: bool,
    /// The bounded edit-loop tier (see `affected::plan_partial`).
    pub partial: bool,
    /// Partial only: no tests.
    pub check_only: bool,
    /// Partial only: graphics-free checks when provably safe.
    pub headless: bool,
}

/// The commit `rev` names (so a stamp is tied to the real base, not to a moving ref name), or `rev` itself when git cannot say.
fn resolve_rev(root: &Path, rev: &str) -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--verify", &format!("{rev}^{{commit}}")])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| rev.to_string())
}

/// `src/lib.rs` in the working tree against `base_ref`: only module declarations were added (false when either version cannot be read).
fn lib_rs_only_adds_modules(root: &std::path::Path, base_ref: &str) -> bool {
    let old =
        std::process::Command::new("git").args(["show", &format!("{base_ref}:src/lib.rs")]).current_dir(root).output().ok().filter(|o| o.status.success());
    match (old, std::fs::read_to_string(root.join("src/lib.rs"))) {
        (Some(old), Ok(new)) => red_engine2::tools::affected::crate_root_only_adds_modules(&String::from_utf8_lossy(&old.stdout), &new),
        _ => false,
    }
}

pub(crate) fn run_affected(files: &[String], base: Option<&str>, flags: AffectedFlags) -> Result<(), String> {
    use red_engine2::tools::{affected, features, symbols};
    let AffectedFlags { quick, full, dry_run, keep_going, no_cache, partial, check_only, headless } = flags;
    let root = symbols::find_root().ok_or("`affected` needs the engine's source tree: run it inside a red-engine-2 checkout (or set RE2_SRC)")?;
    let all = features::load_at(&root)?;
    let serial = features::serial_suites_at(&root);
    // A partial run looks at what you just did (changes since HEAD); the other tiers look at everything you would push (changes since the merge base).
    let explicit = !files.is_empty();
    let base_ref = base.map(str::to_string).unwrap_or_else(|| if partial { "HEAD".to_string() } else { features::default_base(&root) });
    let mut changed: Vec<String> = if explicit { files.to_vec() } else { features::changed_files(&root, &base_ref)? };
    // A crate root that only gained module declarations is not a boundary change: the new files carry their own tests (see `crate_root_only_adds_modules`).
    let mut notes = Vec::new();
    if !explicit && changed.iter().any(|c| c == "src/lib.rs") && lib_rs_only_adds_modules(&root, &base_ref) {
        changed.retain(|c| c != "src/lib.rs");
        notes.push("src/lib.rs only gained module declarations: not treated as a crate-root change (the new files own their tests)".to_string());
    }
    let opts = affected::Options { quick, full, partial, check_only, headless, ..Default::default() };
    let mut headless_note = None;
    let feature_set = if partial && headless {
        match affected::headless_safe(&root, &changed) {
            Ok(()) => affected::Features::Headless,
            Err(why) => {
                headless_note = Some(format!("--headless ignored, checking with the default features: {why}"));
                affected::Features::Default
            }
        }
    } else {
        affected::Features::Default
    };
    let mut plan = if partial { affected::plan_partial(&all, &serial, &changed, &opts, feature_set) } else { affected::plan(&all, &serial, &changed, &opts) };
    plan.notes.extend(notes);
    if let Some(n) = headless_note {
        plan.notes.push(n);
    }
    if !partial {
        affected::prune_doc_step(&mut plan, &root);
    }
    let config = affected::Config::detect(plan.features);
    // A list of files typed by hand says nothing about the rest of the tree: such a run is never remembered or reused.
    let cacheable = !no_cache && !explicit;
    let key =
        affected::StampKey { base: resolve_rev(&root, &base_ref), config: config.id(), variant: if check_only { "check-only".into() } else { String::new() } };
    let json = envelope::capturing();
    let plan_with_config = |p: &affected::Plan| {
        let mut v = affected::plan_json(p);
        v["config"] = serde_json::json!(config.render());
        v
    };
    if dry_run {
        if json {
            println!("{}", plan_with_config(&plan));
        } else {
            println!("config: {}", config.render());
            print!("{}", affected::render_plan(&plan));
        }
        return Ok(());
    }
    if plan.steps.is_empty() {
        if json {
            println!("{}", serde_json::json!({"verified": !partial, "partial": partial, "plan": plan_with_config(&plan), "results": []}));
        } else {
            println!("config: {}", config.render());
            print!("{}", affected::render_plan(&plan));
            println!("nothing to run");
        }
        return Ok(());
    }
    if cacheable {
        if let Some(scope) = affected::already_green_in(&root, &plan.changed, plan.scope, &key) {
            if json {
                println!(
                    "{}",
                    serde_json::json!({"verified": !partial, "partial": partial, "cached": true, "scope": scope.name(), "plan": plan_with_config(&plan), "results": []})
                );
            } else if partial {
                println!("already passed this partial check (identical files, base, configuration): nothing to run. This is not verification; `--no-cache` forces a re-run.");
                for r in &plan.full_required {
                    println!("full verification still required: {r}");
                }
            } else {
                println!("already verified ({} scope, identical file contents and configuration): nothing to run. `--no-cache` forces a re-run.", scope.name());
            }
            return Ok(());
        }
    }
    if !json {
        println!("config: {}", config.render());
        print!("{}", affected::render_plan(&plan));
    }
    let started = std::time::Instant::now();
    let moved = if cfg!(windows) { move_exe_aside(&root) } else { None };
    let results = affected::run(&plan, &root, &root.join("out").join("logs"), keep_going, &mut |r| {
        if json {
            return;
        }
        println!("{} {:<13} {:>6.1}s{}", if r.ok { "ok  " } else { "FAIL" }, r.name, r.secs, r.tally.as_ref().map(|t| format!("  {t}")).unwrap_or_default());
        for l in &r.failures {
            println!("     | {l}");
        }
        if !r.ok {
            println!("     full log: {}", r.log.display());
        }
    });
    restore_exe(moved);
    let ok = results.len() == plan.steps.len() && results.iter().all(|r| r.ok);
    if ok && cacheable {
        affected::record_green_in(&root, &plan.changed, plan.scope, &key);
    }
    if json {
        let rs: Vec<_> = results
            .iter()
            .map(|r| serde_json::json!({"name": r.name, "ok": r.ok, "secs": r.secs, "tally": r.tally, "failures": r.failures, "log": r.log.display().to_string()}))
            .collect();
        println!(
            "{}",
            serde_json::json!({"verified": ok && !partial, "partial": partial, "passed": ok, "scope": plan.scope.name(), "plan": plan_with_config(&plan), "results": rs})
        );
    } else if ok && partial {
        println!("partial pass in {:.0}s: this is NOT verification.", started.elapsed().as_secs_f64());
        for r in &plan.full_required {
            println!("full verification still required: {r}");
        }
    } else if ok {
        println!("verified ({} scope) in {:.0}s", plan.scope.name(), started.elapsed().as_secs_f64());
        if plan.scope != affected::Scope::Full {
            println!("integration boundary (before pushing): `red_engine2 affected --full` (= scripts/ci.sh)");
        }
    }
    if ok {
        Ok(())
    } else {
        Err("verification failed (the failing lines are above; the full logs are under out/logs/)".to_string())
    }
}

pub(crate) fn run_package(zip: &Path, verify: bool, allow_dirty: bool, no_build: bool) -> Result<(), String> {
    use red_engine2::tools::package;
    if verify {
        let bytes = std::fs::read(zip).map_err(|e| format!("{}: {e}", zip.display()))?;
        let (manifest, problems) = package::verify(&bytes)?;
        let hard = package::failures(&problems);
        if envelope::capturing() {
            println!(
                "{}",
                serde_json::json!({"ok": hard.is_empty(), "commit": manifest["commit"], "dirty": manifest["dirty"], "files": manifest["files"].as_object().map_or(0, |f| f.len()), "problems": problems})
            );
        } else {
            println!(
                "{}: commit {}{}, {} file(s)",
                zip.display(),
                manifest["commit"].as_str().unwrap_or("?"),
                if manifest["dirty"] == true { " (dirty tree)" } else { "" },
                manifest["files"].as_object().map_or(0, |f| f.len())
            );
            for p in &problems {
                println!("  {p}");
            }
            println!(
                "{}",
                if hard.is_empty() { "VERIFIED: every file matches the manifest and the headless binaries are clean" } else { "FAILED verification" }
            );
        }
        return if hard.is_empty() { Ok(()) } else { Err(String::new()) };
    }
    if zip.exists() {
        return Err(format!("{} already exists; a package never overwrites (choose a new name)", zip.display()));
    }
    let root = std::env::current_dir().map_err(|e| e.to_string())?;
    package::preflight(&root, allow_dirty)?; // refuse a dirty tree now, not after minutes of release builds
    let (binaries, notes) = if no_build {
        let exe = if cfg!(windows) { ".exe" } else { "" };
        let mut bins = Vec::new();
        for (dir, names) in [("package-gui", ["re2", "red_engine2"]), ("package-headless", ["red_server", "red_bot"])] {
            for n in names {
                bins.push(package::Binary { name: format!("{n}{exe}"), path: root.join("target").join(dir).join("release").join(format!("{n}{exe}")) });
            }
        }
        (bins, Default::default())
    } else {
        package::build_binaries(&root)?
    };
    let built = package::build(&package::Options { root, files: None, binaries, allow_dirty, git: None, notes })?;
    if let Some(parent) = zip.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(zip, &built.zip).map_err(|e| format!("{}: {e}", zip.display()))?;
    if envelope::capturing() {
        println!(
            "{}",
            serde_json::json!({"ok": true, "package": zip.display().to_string(), "files": built.files, "bytes": built.zip.len(), "sha256": built.sha256, "commit": built.manifest["commit"], "dirty": built.manifest["dirty"]})
        );
    } else {
        println!(
            "wrote {} ({} files, {:.1} MB)\nsha256 {}\ncommit {}{}",
            zip.display(),
            built.files,
            built.zip.len() as f64 / 1e6,
            built.sha256,
            built.manifest["commit"].as_str().unwrap_or("?"),
            if built.manifest["dirty"] == true { " (DIRTY TREE, listed in the manifest)" } else { "" }
        );
        println!("check it any time with: red_engine2 package --verify {}", zip.display());
    }
    Ok(())
}

pub(crate) fn run_portmap(action: &str, port: u16, lease: u32, router: Option<std::net::IpAddr>, allow_permanent: bool) -> Result<(), String> {
    use red_engine2::tools::portmap::{self, Options};
    let opts = Options { port, lease, router, allow_permanent };
    let out = match action {
        "status" => portmap::status(&opts),
        "enable" => portmap::enable(&opts),
        "remove" => portmap::remove(&opts),
        "keep" => {
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let s = stop.clone();
            ctrlc::set_handler(move || s.store(true, std::sync::atomic::Ordering::Relaxed)).map_err(|e| format!("cannot install Ctrl-C handler: {e}"))?;
            portmap::keep(&opts, &stop)
        }
        other => return Err(format!("unknown portmap action '{other}' (status | enable | remove | keep)")),
    };
    match out {
        Ok(text) => {
            print!("{text}");
            Ok(())
        }
        Err(e) => Err(e.to_string()),
    }
}

pub(crate) fn run_perf(scene: &Path, players: Option<usize>, secs: Option<f64>, windows: Option<usize>, budget_file: Option<&Path>) -> Result<(), String> {
    use red_engine2::tools::perf;
    // The budget: a file, else the scene's own `checks.perf`, else the defaults.
    let block = match budget_file {
        Some(p) => Some(
            serde_json::from_str::<Value>(&std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?)
                .map_err(|e| format!("{}: {e}", p.display()))?,
        ),
        None => {
            let text = std::fs::read_to_string(scene).map_err(|e| format!("{}: {e}", scene.display()))?;
            serde_json::from_str::<Value>(&text).ok().and_then(|v| v.get("checks").and_then(|c| c.get("perf")).cloned())
        }
    };
    let (budget, block_players, block_secs, block_windows) = match &block {
        Some(b) => perf::parse_budget(b)?,
        None => (perf::Budget::default(), 4, 1.0, 2),
    };
    let has_block = block.is_some();
    let players = players.unwrap_or(block_players);
    let secs = secs.unwrap_or(if has_block { block_secs.max(1.0) } else { 3.0 });
    let windows = windows.unwrap_or(if has_block { block_windows.max(3) } else { 3 });
    let m = perf::measure(scene, players, secs, windows)?;
    let verdicts = perf::evaluate(&m, &budget);
    if envelope::capturing() {
        println!("{}", perf::to_json(&m, &verdicts));
    } else {
        print!("{}", perf::render(&m, &verdicts));
    }
    if verdicts.iter().all(|v| v.ok) {
        Ok(())
    } else {
        Err(String::new())
    }
}

pub(crate) fn run_race_track(out: &Path, spec: red_engine2::tools::racetrack::TrackSpec) -> Result<(), String> {
    use red_engine2::tools::racetrack;
    let scene = racetrack::build(&spec)?;
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(out, serde_json::to_string(&scene).map_err(|e| e.to_string())?).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("wrote {}: {}", out.display(), racetrack::summary(&spec, &scene));
    Ok(())
}

pub(crate) fn run_race_test(scene: &Path, bots: usize, level: f32, secs: f32, drivers: Option<&str>) -> Result<(), String> {
    use red_engine2::tools::racetest;
    let drivers = match drivers {
        Some(list) => racetest::parse_drivers(list)?,
        None => Vec::new(),
    };
    let report = racetest::run(scene, bots, level, secs, &drivers)?;
    if envelope::capturing() {
        println!("{}", racetest::to_json(&report));
    } else {
        print!("{}", racetest::render(&report));
    }
    if report.all_finished() {
        Ok(())
    } else {
        Err(String::new())
    }
}

pub(crate) fn run_net_identity(out: &Path, names: &[String]) -> Result<(), String> {
    let key_path = out.join("key.pem");
    if key_path.exists() {
        return Err(format!("{} already exists: not overwriting a server identity (delete it deliberately to replace it)", key_path.display()));
    }
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let gen = red_engine2::net::quic::ServerIdentity::generate(names)?;
    std::fs::write(out.join("cert.pem"), &gen.cert_pem).map_err(|e| format!("cert.pem: {e}"))?;
    write_private(&key_path, &gen.key_pem)?;
    println!("wrote {} and {} (keep the key private; never commit it)", out.join("cert.pem").display(), key_path.display());
    println!("fingerprint: {}", gen.identity.fingerprint());
    println!("serve:  red_server --tls-cert {} --tls-key {} --public", out.join("cert.pem").display(), key_path.display());
    println!("join:   re2 --connect HOST:PORT --server-fingerprint {} MAP.json", gen.identity.fingerprint());
    Ok(())
}

/// Writes a secret file readable by its owner only (Unix mode 0600; on Windows the file inherits the directory's ACL).
fn write_private(path: &Path, text: &str) -> Result<(), String> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    std::io::Write::write_all(&mut f, text.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))
}

pub(crate) fn run_net_test(scene: &Path, profile: &[String], players: usize, secs: f64, seed: u64, transport: &str) -> Result<(), String> {
    let quic = match transport {
        "quic" => true,
        "udp" => false,
        other => return Err(format!("--transport must be udp or quic, not '{other}'")),
    };
    use red_engine2::net::netsim::{self, LinkProfile};
    use red_engine2::tools::nettest;
    let mut profiles: Vec<LinkProfile> = Vec::new();
    for name in profile {
        if name == "all" {
            profiles.extend(netsim::PROFILES.iter().copied());
        } else {
            profiles.push(netsim::profile(name).ok_or_else(|| {
                format!("unknown link profile '{name}' (profiles: {}, all)", netsim::PROFILES.iter().map(|p| p.name).collect::<Vec<_>>().join(", "))
            })?);
        }
    }
    let reports = nettest::run(scene, &nettest::Options { profiles, players, secs, seed, quic })?;
    if envelope::capturing() {
        println!("{}", nettest::to_json(&reports));
    } else {
        print!("{}", nettest::render(&reports));
    }
    if reports.iter().all(nettest::ProfileReport::ok) {
        Ok(())
    } else {
        Err(String::new())
    }
}

pub(crate) fn run_sim(
    scene: &Path,
    scenario: Option<&Path>,
    only: Option<&str>,
    trace: Option<&Path>,
    checkpoint_every: u32,
    dump_every: u32,
) -> Result<(), String> {
    use red_engine2::sim::scenario::RecordOptions;
    let record = trace.map(|_| RecordOptions { map_hash: 0, checkpoint_every: checkpoint_every.max(1), dump_every });
    let (report, recorded) = simrun::run(scene, scenario, only, record)?;
    let mut json = report.to_json();
    if let (Some(path), Some(t)) = (trace, recorded) {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, serde_json::to_string(&t.to_json()).unwrap_or_default()).map_err(|e| format!("{}: {e}", path.display()))?;
        json["trace"] = serde_json::json!(path.display().to_string());
        if !envelope::capturing() {
            println!(
                "recorded {} ticks, {} checkpoints, {} inputs to {} — replay it: red_engine2 replay {}",
                t.final_tick,
                t.checkpoints.len(),
                t.entries.len(),
                path.display(),
                path.display()
            );
        }
    }
    if envelope::capturing() {
        println!("{json}");
    } else {
        print!("{}", report.render());
    }
    if report.all_passed() {
        Ok(())
    } else {
        Err(String::new())
    }
}

pub(crate) fn run_replay(trace: &Path, scene: Option<&Path>, against: Option<&Path>) -> Result<(), String> {
    if let Some(other) = against {
        let (same, text) = simrun::compare_files(trace, other)?;
        if envelope::capturing() {
            println!("{}", serde_json::json!({"ok": same, "detail": text}));
        } else {
            println!("{} {text}", if same { "OK" } else { "DIVERGED" });
        }
        return if same { Ok(()) } else { Err(String::new()) };
    }
    let outcome = simrun::replay_file(trace, scene)?;
    if envelope::capturing() {
        println!("{}", outcome.to_json());
    } else {
        print!("{}", outcome.render());
    }
    if outcome.ok() {
        Ok(())
    } else {
        Err(String::new())
    }
}
