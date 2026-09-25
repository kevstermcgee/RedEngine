//! Small parsing helpers shared by the commands (`x,y,z` lists, rectangles, prop kind names).

use super::*;

pub(crate) fn floats(s: &str) -> Result<Vec<f64>, String> {
    s.split(',').map(|p| p.trim().parse::<f64>().map_err(|_| format!("'{p}' in '{s}' is not a number"))).collect()
}

pub(crate) fn vec3(s: &str) -> Result<[f64; 3], String> {
    let v = floats(s)?;
    if v.len() != 3 {
        return Err(format!("expected x,y,z but got '{s}'"));
    }
    Ok([v[0], v[1], v[2]])
}

pub(crate) fn v3(s: &str) -> Result<Vec3, String> {
    let v = vec3(s)?;
    Ok(Vec3::new(v[0] as f32, v[1] as f32, v[2] as f32))
}

pub(crate) fn v2(s: &str) -> Result<Vec2, String> {
    let v = floats(s)?;
    if v.len() != 2 {
        return Err(format!("expected x,z but got '{s}'"));
    }
    Ok(Vec2::new(v[0] as f32, v[1] as f32))
}

pub(crate) fn rect(s: &str) -> Result<(Vec2, Vec2), String> {
    let v = floats(s)?;
    if v.len() != 4 {
        return Err(format!("expected x0,z0,x1,z1 but got '{s}'"));
    }
    Ok((Vec2::new(v[0].min(v[2]) as f32, v[1].min(v[3]) as f32), Vec2::new(v[0].max(v[2]) as f32, v[1].max(v[3]) as f32)))
}

pub(crate) fn split_list(s: Option<&str>) -> Vec<String> {
    s.map(|v| v.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect()).unwrap_or_default()
}

pub(crate) fn prop_kinds(s: &str) -> Result<Vec<PropKind>, String> {
    s.split(',')
        .map(|n| {
            PropKind::from_name(n.trim()).ok_or_else(|| {
                let names: Vec<&str> = PropKind::ALL.iter().map(|k| k.name()).collect();
                format!("unknown prop '{n}' (run `red_engine2 props`; kinds: {})", names.join(", "))
            })
        })
        .collect()
}
