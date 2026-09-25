//! Commands that change a scene file: the edit wrapper (validate before writing), scatter and line.

use super::*;

/// Load a scene file, run `f` on it, save (validating), and report what changed.
pub(crate) fn edit(path: &Path, flags: &EditFlags, f: impl FnOnce(&mut SceneFile) -> Result<String, String>) -> Result<(), String> {
    let mut file = SceneFile::load(path)?;
    let msg = f(&mut file)?;
    file.save(path, flags.dry_run, flags.force)?;
    println!("{msg}  ({})", path.display());
    println!("next: red_engine2 lint {}", path.display());
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_scatter(
    scene: &Path,
    flags: &EditFlags,
    kind: &str,
    count: usize,
    rects: &[String],
    zones: &[String],
    excludes: &[String],
    seed: u64,
    id_prefix: &str,
    color: Option<&str>,
    scale: &str,
    min_gap: f32,
    clearance: f32,
    y: f32,
    no_yaw: bool,
    lint_ignore: Option<&str>,
) -> Result<(), String> {
    let world = load_or_report(scene)?;
    let mut rs: Vec<(Vec2, Vec2)> = rects.iter().map(|r| rect(r)).collect::<Result<_, _>>()?;
    for z in zones {
        let zone = world
            .zones
            .iter()
            .find(|zn| &zn.id == z)
            .ok_or_else(|| format!("no zone '{z}' in the scene (zones: {})", world.zones.iter().map(|z| z.id.as_str()).collect::<Vec<_>>().join(", ")))?;
        rs.push((zone.min, zone.max));
    }
    let (lo, hi) = scale.split_once(':').ok_or("--scale must look like 0.9:1.2")?;
    let params = ScatterParams {
        kinds: prop_kinds(kind)?,
        count,
        rects: rs,
        excludes: excludes.iter().map(|r| rect(r)).collect::<Result<_, _>>()?,
        seed,
        id_prefix: id_prefix.to_string(),
        colors: color.map(|c| c.split(',').map(|s| s.trim().to_string()).collect()).unwrap_or_default(),
        scale: (lo.parse().map_err(|_| "bad --scale")?, hi.parse().map_err(|_| "bad --scale")?),
        min_gap,
        clearance,
        y,
        random_yaw: !no_yaw,
        lint_ignore: split_list(lint_ignore),
    };
    let objs = gen::scatter(&world, &params)?;
    edit(scene, flags, |f| {
        let n = objs.len();
        for o in objs {
            f.add("objects", o)?;
        }
        Ok(format!("scattered {n} object(s) (ids {id_prefix}_1 ...; remove them again with: rm '{id_prefix}_*')"))
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_line(
    scene: &Path,
    flags: &EditFlags,
    kind: &str,
    from: &str,
    to: &str,
    spacing: f32,
    id_prefix: &str,
    color: Option<String>,
    y: f32,
    scale: f32,
    lint_ignore: Option<&str>,
) -> Result<(), String> {
    let kinds = prop_kinds(kind)?;
    let objs = gen::line(&LineParams {
        kind: kinds[0],
        from: v2(from)?,
        to: v2(to)?,
        spacing,
        id_prefix: id_prefix.to_string(),
        color,
        y,
        scale,
        lint_ignore: split_list(lint_ignore),
    })?;
    edit(scene, flags, |f| {
        let n = objs.len();
        for o in objs {
            f.add("objects", o)?;
        }
        Ok(format!("placed {n} object(s) (ids {id_prefix}_1 ...; remove them again with: rm '{id_prefix}_*')"))
    })
}
