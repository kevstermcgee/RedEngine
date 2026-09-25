//! Multiplayer spawn points, read from a scene's optional top-level `"spawns"` array:
//!
//! ```json
//! "spawns": [ { "id": "spawn_a", "position": [-22, 0, -3], "yaw_deg": 90, "group": "duel" } ]
//! ```
//!
//! `position` is `[x, y, z]` (y = the floor height), `yaw_deg` the facing (0 = looking along -Z,
//! 90 = along +X), `group` an optional label so a server can choose a subset (`--spawn-group`).
//! A scene without any gets one spawn at its camera. (The engine's `Scene` type does not carry them:
//! like `zones`, they are read from the raw JSON, so old maps stay valid.)

use serde_json::Value;

/// One spawn point.
#[derive(Debug, Clone, PartialEq)]
pub struct Spawn {
    /// Its id in the map file.
    pub id: String,
    /// World position `(x, y, z)`; y is the floor height.
    pub position: [f32; 3],
    /// Facing, degrees (0 = -Z, 90 = +X).
    pub yaw_deg: f32,
    /// Optional group label (`""` when none).
    pub group: String,
}

fn num(v: &Value) -> Option<f32> {
    v.as_f64().map(|f| f as f32)
}

/// Parses the `spawns` of a scene's JSON text. Malformed entries are reported, not skipped silently.
/// With no `spawns` array, falls back to the scene camera's position and heading.
pub fn parse_spawns(scene_json: &str) -> Result<Vec<Spawn>, String> {
    let v: Value = serde_json::from_str(scene_json).map_err(|e| format!("scene is not JSON: {e}"))?;
    if let Some(list) = v.get("spawns").and_then(Value::as_array) {
        let mut out = Vec::new();
        for (i, s) in list.iter().enumerate() {
            let p = s.get("position").and_then(Value::as_array).filter(|a| a.len() == 3).ok_or(format!("spawns[{i}].position must be [x, y, z]"))?;
            let position = [
                num(&p[0]).ok_or(format!("spawns[{i}].position[0] is not a number"))?,
                num(&p[1]).ok_or(format!("spawns[{i}].position[1] is not a number"))?,
                num(&p[2]).ok_or(format!("spawns[{i}].position[2] is not a number"))?,
            ];
            out.push(Spawn {
                id: s.get("id").and_then(Value::as_str).unwrap_or("spawn").to_string(),
                position,
                yaw_deg: s.get("yaw_deg").and_then(num).unwrap_or(0.0),
                group: s.get("group").and_then(Value::as_str).unwrap_or("").to_string(),
            });
        }
        if out.is_empty() {
            return Err("\"spawns\" is present but empty".into());
        }
        return Ok(out);
    }
    // Fallback: the camera. `camera.position` may be a plain triple (a constant track).
    let cam = v.get("camera").ok_or("no spawns and no camera")?;
    let pos = cam.get("position").and_then(Value::as_array).filter(|a| a.len() == 3).ok_or("camera.position must be [x, y, z]")?;
    let tgt = cam.get("target").and_then(Value::as_array).filter(|a| a.len() == 3).ok_or("camera.target must be [x, y, z]")?;
    let (px, pz) = (num(&pos[0]).unwrap_or(0.0), num(&pos[2]).unwrap_or(0.0));
    let (tx, tz) = (num(&tgt[0]).unwrap_or(0.0), num(&tgt[2]).unwrap_or(0.0));
    Ok(vec![Spawn { id: "camera".into(), position: [px, 0.0, pz], yaw_deg: libm::atan2f(tx - px, -(tz - pz)).to_degrees(), group: String::new() }])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_test_lab_has_four_duel_spawns_facing_each_other_and_props_spawns() {
        let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json")).unwrap();
        let s = parse_spawns(&text).unwrap();
        assert!(s.len() >= 6);
        let a = s.iter().find(|s| s.id == "spawn_a").unwrap();
        let b = s.iter().find(|s| s.id == "spawn_b").unwrap();
        // a faces +X (yaw 90), b faces -X (yaw 270): they look at each other across the hall.
        assert!(a.position[0] < b.position[0] && (a.yaw_deg - 90.0).abs() < 1e-3 && (b.yaw_deg - 270.0).abs() < 1e-3);
        assert!(s.iter().any(|s| s.group == "props"));
    }

    #[test]
    fn without_spawns_the_camera_is_the_spawn() {
        let s = parse_spawns(r#"{"camera":{"position":[0,1.7,5],"target":[0,1,0]},"objects":[]}"#).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].position, [0.0, 0.0, 5.0]);
        assert!(s[0].yaw_deg.abs() < 1e-3, "looking toward -Z is yaw 0: {}", s[0].yaw_deg);
    }

    #[test]
    fn bad_spawns_are_errors() {
        assert!(parse_spawns(r#"{"spawns":[{"position":[1,2]}]}"#).is_err());
        assert!(parse_spawns(r#"{"spawns":[]}"#).is_err());
        assert!(parse_spawns("not json").is_err());
    }
}
