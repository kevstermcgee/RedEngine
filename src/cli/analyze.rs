//! Map analysis and checks: validate, lint, reach, plan, walk, info, diff, verify, and the headless sim/replay commands.

use super::*;

pub(crate) fn analyze(scene: &Path, cell: f32) -> Result<(MapWorld, reach::Reach, Vec<lint::Finding>), String> {
    let world = load_or_report(scene)?;
    let r = reach::compute(&world, &ReachParams { cell, ..Default::default() });
    let findings = lint::lint(&world, &r);
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

pub(crate) fn run_lint(scene: &Path, json: bool, strict: bool, cell: f32) -> Result<(), String> {
    let (world, r, findings) = analyze(scene, cell)?;
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

pub(crate) fn run_reach(scene: &Path, from: Option<&str>, to: Option<&str>, cell: f32, json: bool) -> Result<(), String> {
    let world = load_or_report(scene)?;
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
) -> Result<(), String> {
    let (world, r, findings) = analyze(scene, 0.1)?;
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
        let opts = PlanOptions { y: h, scale, bounds, labels, show_reach: !no_reach, show_findings: !no_lint };
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

pub(crate) fn run_walk(scene: &Path, path: &str, from: Option<&str>) -> Result<(), String> {
    let world = load_or_report(scene)?;
    let wps: Vec<Vec2> = path.split(';').filter(|p| !p.trim().is_empty()).map(v2).collect::<Result<_, _>>()?;
    if wps.is_empty() {
        return Err("--path needs at least one waypoint".to_string());
    }
    let start = from.map(v2).transpose()?.unwrap_or(world.spawn);
    let steps = red_engine2::tools::walk::walk(&world, start, &wps);
    if envelope::capturing() {
        println!("{}", red_engine2::tools::walk::to_json(&steps, wps.len()));
    } else {
        print!("{}", red_engine2::tools::walk::format_walk(&steps, wps.len()));
    }
    if steps.len() < wps.len() || steps.last().is_some_and(|l| !l.reached) {
        return Err(String::new());
    }
    Ok(())
}

pub(crate) fn run_info(scene: &Path, id: &str) -> Result<(), String> {
    let (world, _r, findings) = analyze(scene, 0.1)?;
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
