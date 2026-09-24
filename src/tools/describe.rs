//! `red_engine2 describe`: the engine describing itself, so an AI never has to open source or
//! guess. Everything here is either derived from live code (the CLI's own clap definitions,
//! `player.rs` constants, `PropKind::ALL`, the prefab catalogue) or a hand-kept table that a
//! unit test proves stays valid (every object type carries a runnable `example`; every lint code
//! the linter can emit must be described here).
//!
//! Topics: `overview` (default), `commands`, `objects`, `scene` (top-level keys), `lint`,
//! `physics`, `conventions`, `glossary` and `decisions` (embedded `docs/`), `all` (everything, for `--json`).

use super::search;
use crate::player;
use serde_json::{json, Value};

/// Topic names and one-line descriptions for `describe`; a test renders every one.
pub const TOPICS: &[(&str, &str)] = &[
    ("overview", "what the engine is + the topics below"),
    ("commands", "every CLI command and its flags (generated from the real CLI)"),
    ("objects", "every object `type` with its fields and a working example"),
    ("scene", "top-level scene keys: meta, camera, lights, zones, prefabs, checks, ..."),
    ("lint", "every lint code: what it means and how to fix it"),
    ("physics", "player size/speed/step rules that decide what is walkable (live constants)"),
    ("conventions", "coordinates, origins, facing, naming — the things that cause silent mistakes"),
    ("glossary", "project vocabulary: props vs prefabs, zones, body band, the four maps, the tire-iron naming trap, ..."),
    ("decisions", "the architecture decision records (docs/adr): why the engine is built this way, one line each"),
    ("all", "everything above as one JSON document (use with --json)"),
];

/// Description of one object `type`: summary, fields and a working example (test-parsed).
pub struct TypeInfo {
    pub name: &'static str,
    pub summary: &'static str,
    /// (field, type/default, description)
    pub fields: &'static [(&'static str, &'static str, &'static str)],
    /// A complete, valid object of this type; `tests::every_example_parses` proves it.
    pub example: &'static str,
}

/// Fields every object accepts (`id`, `position`, `rotation`, `scale`, `collide`, `lint_ignore`, ...).
pub const COMMON_FIELDS: &[(&str, &str, &str)] = &[
    ("id", "string, required, unique", "name every tool refers to; use prefixes (bed_master, bush_west_1) so `rm 'bush_west_*'` works"),
    ("position", "[x,y,z] = [0,0,0]", "meters, +Y up; a keyframed track is allowed"),
    ("rotation", "[rx,ry,rz] degrees = [0,0,0]", "applied X then Y then Z; only Y (yaw) is meaningful for upright objects"),
    ("scale", "number or [x,y,z] = 1", "uniform or per-axis"),
    ("material", "{color, metallic, roughness, emissive}", "color #rrggbb; metallic/roughness 0..1 (defaults 0/0.6); emissive #hex glows"),
    ("collide", "bool = true", "false = walk-through (no collider, not standable, no lint volume); on a group it covers everything inside"),
    ("lint_ignore", "[\"code\", ...]", "silence specific lint codes on this object (or \"all\")"),
];

/// Every object type with its fields and example; a test fails if the schema accepts a type not listed here.
pub const TYPES: &[TypeInfo] = &[
    TypeInfo {
        name: "box",
        summary: "cuboid — walls, floors, slabs, furniture blocks. The only primitive that blocks the player.",
        fields: &[("size", "[w,h,d] = [1,1,1]", "centered on position")],
        example: r##"{"id":"b","type":"box","size":[1,0.5,2],"position":[0,0.25,0],"material":{"color":"#8a6a3f"}}"##,
    },
    TypeInfo {
        name: "sphere",
        summary: "sphere; centered on position. Does not block the player (decor).",
        fields: &[("radius", "number = 0.5", "")],
        example: r##"{"id":"s","type":"sphere","radius":0.1,"position":[0,0.1,0],"material":{"color":"#c42b1f"}}"##,
    },
    TypeInfo {
        name: "cylinder",
        summary: "vertical cylinder, centered on position (axis Y; rotate [90,0,0] to lay it along Z). Does not block.",
        fields: &[("radius", "number = 0.5", ""), ("height", "number = 1", "total height")],
        example: r##"{"id":"c","type":"cylinder","radius":0.1,"height":0.5,"position":[0,0.25,0],"material":{"color":"#999999"}}"##,
    },
    TypeInfo {
        name: "cone",
        summary: "cone, point up, centered on position. Does not block.",
        fields: &[("radius", "number = 0.5", "base radius"), ("height", "number = 1", "")],
        example: r##"{"id":"k","type":"cone","radius":0.2,"height":0.4,"position":[0,0.2,0],"material":{"color":"#c62828"}}"##,
    },
    TypeInfo {
        name: "capsule",
        summary: "pill shape, centered on position. height is the TOTAL extent (radius included). Does not block.",
        fields: &[("radius", "number = 0.3", ""), ("height", "number = 1", "total, both caps included")],
        example: r##"{"id":"p","type":"capsule","radius":0.05,"height":0.3,"position":[0,0.15,0],"material":{"color":"#f2d43a"}}"##,
    },
    TypeInfo {
        name: "plane",
        summary: "flat quad facing +Y (floors, rugs, water). Thin; use a plane per room at y+0.01 for floors.",
        fields: &[("size", "[w,d] = [10,10]", "")],
        example: r##"{"id":"floor","type":"plane","size":[6,5],"position":[0,0.01,0],"material":{"color":"#b8a888"}}"##,
    },
    TypeInfo {
        name: "group",
        summary: "moves/rotates a set of children as one unit; children are relative to the group. Needs no material of its own.",
        fields: &[("children", "[object, ...], required", "any object types, nestable")],
        example: r##"{"id":"g","type":"group","position":[1,0,1],"children":[{"id":"g_a","type":"box","size":[1,1,1],"position":[0,0.5,0],"material":{"color":"#888888"}}]}"##,
    },
    TypeInfo {
        name: "prop",
        summary: "one of the built-in Rust-made props (see `catalog --kind prop`); proper collision; origin = middle of base, front = +Z.",
        fields: &[("prop", "name, required", "e.g. sofa, crate, tree_oak"), ("material.color", "#hex", "tints the body (foliage/blooms for plants)")],
        example: r##"{"id":"crate_1","type":"prop","prop":"crate","position":[2,0,-1],"rotation":[0,15,0],"material":{"color":"#8a6a3f"}}"##,
    },
    TypeInfo {
        name: "prefab",
        summary: "a JSON template from the catalogue (or the scene's own `prefabs`); expands to a group. `catalog` lists them.",
        fields: &[
            ("prefab", "name, required", "e.g. apple_red, chair_folding, shelf_gondola"),
            ("params", "{name: value}", "the prefab's declared params (`catalog <name>` shows them)"),
            ("collide", "bool", "override the prefab's default (small items default to walk-through)"),
        ],
        example: r##"{"id":"a1","type":"prefab","prefab":"apple_red","position":[0,0,0],"params":{"color":"#8cc63f"}}"##,
    },
    TypeInfo {
        name: "stairs",
        summary: "straight staircase. position.y = bottom height; local +Z is the run axis (bottom at -run/2, top at +run/2). Yaw only.",
        fields: &[
            ("width", "number = 1.2", ">= 1.1 recommended"),
            ("run", "number = 4", "horizontal length"),
            ("rise", "number = 3", "vertical height gained"),
            ("steps", "int = 16", "aim for step rise <= 0.20"),
        ],
        example: r##"{"id":"st","type":"stairs","width":1.2,"run":4,"rise":2.8,"steps":14,"position":[0,0,0],"material":{"color":"#a08060"}}"##,
    },
    TypeInfo {
        name: "wall",
        summary: "macro: one straight wall run with door/window/arch openings, trim and baseboard; expands to boxes. Walls meeting at corners just work.",
        fields: &[
            ("from", "[x,z], required", "start of the centerline"),
            ("to", "[x,z], required", "end of the centerline"),
            ("y", "number = 0", "floor height it stands on (upper-floor walls need their own y)"),
            ("height", "number = 2.7", ""),
            ("thickness", "number = 0.2", "exterior 0.24, partitions 0.15"),
            ("openings", "[{kind,at,width,height,sill,trim,glass}]", "kind door|window|arch; `at` = distance along the wall from `from` to the opening's center; doors must be >= 2.05 tall and >= 0.9 wide"),
            ("trim", "#hex", "frame color for every opening"),
            ("baseboard", "#hex or {color,height}", ""),
            ("extend", "bool = true", "grow the ends by half the thickness so corners close"),
        ],
        example: r##"{"id":"w1","type":"wall","from":[0,0],"to":[6,0],"height":2.7,"thickness":0.2,"openings":[{"kind":"door","at":3,"width":1.0}],"material":{"color":"#d8d0c0"}}"##,
    },
    TypeInfo {
        name: "fence",
        summary: "macro: posts + rails/panels along a polyline; gaps for gates; expands to boxes.",
        fields: &[
            ("points", "[[x,z], ...], required", "at least 2"),
            ("closed", "bool = false", "close the loop"),
            ("height", "number = 1.8", ""),
            ("post_spacing", "number = 2", ""),
            ("style", "panel|rail = panel", ""),
            ("gaps", "[{at:[x,z], width}]", "a gate opening cut into the nearest segment"),
        ],
        example: r##"{"id":"f1","type":"fence","points":[[0,0],[8,0],[8,6],[0,6]],"closed":true,"gaps":[{"at":[4,0],"width":2.4}],"material":{"color":"#b8a888"}}"##,
    },
    TypeInfo {
        name: "humanoid",
        summary: "posable ordinary-looking person (shirt, skin, hair, jeans, shoes); the human player's body in `re2`.",
        fields: &[
            ("height", "number = 1.8", ""),
            ("build", "number = 1", ""),
            ("material", "{color}", "the T-shirt colour"),
            ("skin|hair|pants|shoes", "\"#rrggbb\"", "optional colours for the rest of the figure"),
            ("pose", "{spine, head, l_shoulder, ...}", "joint angles in degrees, each may be keyframed"),
        ],
        example: r##"{"id":"h","type":"humanoid","height":1.8,"material":{"color":"#4b7fb0"},"hair":"#3a281c"}"##,
    },
    TypeInfo {
        name: "rat",
        summary: "Cheddar-style lab rat (0.43 m long + tail, ~0.17 m tall); the rat player's body in `re2`. Origin at the paws, nose toward +Z.",
        fields: &[
            ("material", "{color}", "fur colour (default a brownish grey); ears, nose, paws and tail stay pink"),
            ("pose", "{gait, stride, sway}", "gait phase (rad), stride 0 (still)..1 (scurrying), tail/head idle phase (rad); each may be keyframed"),
        ],
        example: r##"{"id":"cheddar","type":"rat","position":[2,0,3],"pose":{"gait":0.7,"stride":0.5}}"##,
    },
];

/// Top-level scene keys and what each does.
pub const SCENE_KEYS: &[(&str, &str)] = &[
    ("meta", "{fps, duration, resolution:[w,h]} — video/frame settings; resolution also sets `frame`/`tour` size"),
    ("background", "{sky_top, sky_bottom} gradient, or {color} flat"),
    ("ambient", "{color, intensity} flat fill light (0.1-0.4 typical)"),
    ("camera", "{fov, position, target, near, far} REQUIRED; for walkable maps position.xz is the player spawn"),
    ("post", "{ao, outline, ao_radius, enabled} clarity pass: contact shadows + silhouette outlines"),
    ("lights", "<= 16; {id, type: directional|point, color, intensity, direction|position, range, cast_shadows, shadow_radius, shadow_center}; one directional may cast shadows"),
    ("zones", "[{id, rect:[x0,z0,x1,z1], y, kind}] named rooms; give lint/reach/tour/plan names to talk about"),
    ("prefabs", "scene-local prefab definitions {name: {params, objects, tags, desc, extends, collide, mount}} — shadow built-ins"),
    ("checks", "expectations `verify` runs: {lint, reach, walk, objects, views} — see `describe checks` / SPEC"),
    ("objects", "the scene graph: array of objects (see `describe objects`)"),
    ("<anything else>", "ignored — scenes may carry their own notes"),
];

/// Every lint code with severity and fix; a test fails if `lint.rs` can emit a code not listed here.
pub const LINT_CODES: &[(&str, &str, &str)] = &[
    ("overlap", "error", "a prop/stairs interpenetrates another prop, wall or floor -> move one apart"),
    ("floating", "warn", "a prop isn't resting on the surface under it -> set position.y to that surface's height"),
    ("sunk", "warn", "a prop is partly below its surface -> raise position.y"),
    ("headroom", "error", "a ceiling/door header is lower than 2.05 m over walkable floor -> raise it"),
    ("stairs-top", "error", "stairs' top lands in a wall or void -> add a landing/floor slab beyond the top end"),
    ("stairs-bottom", "error", "stairs' bottom starts in a wall -> clear space at the bottom end"),
    ("stairs-narrow", "warn", "stairs narrower than 1.1 m -> widen"),
    ("stairs-steep", "warn", "step rise > 0.2 m -> more steps or longer run"),
    ("drop", "warn", "walkable edge with a big fall and no railing (open stairwell) -> add a 1.05 m railing wall"),
    ("leak", "error", "the player can walk off the map: a gap in the perimeter -> close it with wall/fence"),
    ("zone", "error", "a declared zone is unreachable or partly sealed -> open a door/arch or fix the zone rect"),
    ("floor", "warn", "a floor slab nobody can reach -> connect it or remove it"),
    ("unreachable", "warn", "a prop the player can't get near -> open a path or drop it"),
    ("door-blocked", "error", "furniture in front of / a wall behind a doorway -> clear 0.9 m each side"),
    ("door", "warn", "a connection narrower than 0.9 m -> widen"),
    ("spawn", "error", "spawn (camera xz) is inside a solid or off any floor -> move camera.position"),
    ("light", "warn", "a lamp sits inside a wall -> move it into the room"),
    ("z-fight", "warn", "coplanar overlapping planes flicker -> offset one by 0.01 or shrink"),
    ("duplicate-id", "error", "two objects share an id -> rename one"),
    ("reach", "error", "the flood-fill from spawn failed (spawn enclosed) -> see spawn"),
];

fn physics() -> Vec<(&'static str, String, &'static str)> {
    vec![
        ("player_radius_m", format!("{}", player::PLAYER_RADIUS), "collision circle; a doorway needs >= 2x this"),
        ("comfortable_door_width_m", format!("{}", player::MIN_COMFORTABLE_DOOR_WIDTH), "lint warns below this"),
        ("body_headroom_m", format!("{}", player::PLAYER_HEADROOM), "anything lower than this above the floor blocks walking (door headers, beams); doors must be at least this tall"),
        ("walk_speed_mps", format!("{}", player::WALK_SPEED), ""),
        ("sprint_speed_mps", format!("{}", player::SPRINT_SPEED), ""),
        ("eye_height_m", format!("{}", player::STAND_EYE_HEIGHT), "camera height when standing (crouch: 1.05)"),
        ("jump_height_m", format!("{:.2}", player::JUMP_HEIGHT), "can hop onto low props"),
        ("step_up_m", "0.35".to_string(), "colliders whose top is <= 0.35 above the feet are stepped ONTO (slab edges, low props); taller ones block"),
        ("floors", "ground_height_at".to_string(), "a surface is walkable only if within ~0.35 of the current foot height — a second-floor slab is unreachable except via stairs"),
    ]
}

/// The silent-mistake conventions (axes, origins, facing, floors, doors, naming, lights, ...).
pub const CONVENTIONS: &[(&str, &str)] = &[
    ("axes", "right-handed, +Y up. `plan` draws +X right, +Z down. Meters everywhere; rotations in degrees."),
    ("origin", "props and floor prefabs: origin = middle of the BASE (position.y = the surface they stand on). Wall prefabs: origin = middle of the back face."),
    ("facing", "things with a front face local +Z. rotation [0,180,0] faces -Z, [0,90,0] faces +X, [0,-90,0] faces -X."),
    ("floors", "one `plane` per room at y+0.01; a 0.2 m slab `box` between levels WITH a hole for the stairs."),
    ("upper floors", "walls on an upper floor need their own objects at that floor's `y` — they do not inherit from walls below."),
    ("doors", "openings: door >= 2.05 tall & >= 0.9 wide; keep 0.9 m clear each side; never park furniture in front."),
    ("naming", "prefix ids by feature (`bed_master`, `flowers_back_3`) so one `rm 'flowers_back_*'` re-rolls a feature."),
    ("boxes vs decor", "only `box` (and props/stairs) collide. spheres/cylinders/cones/capsules never block the player. `collide:false` makes anything walk-through."),
    ("lights", "<= 16 point lights; a lamp per room ~0.4 m below the ceiling, range 6-8; one directional sun with cast_shadows + shadow_center on the map middle. Point lights ignore walls."),
    ("clarity", "two similar tones in contact (white fridge, cream wall) read as one blob: vary lightness, use trim/baseboards."),
    ("verify", "never trust an edit you haven't run through `lint`, and never judge a layout you haven't looked at (`plan`/`tour`/`frame`)."),
];

fn types_json() -> Value {
    Value::Array(
        TYPES
            .iter()
            .map(|t| {
                json!({
                    "type": t.name, "summary": t.summary,
                    "fields": t.fields.iter().map(|(n, ty, d)| json!({"name": n, "type": ty, "desc": d})).collect::<Vec<_>>(),
                    "example": serde_json::from_str::<Value>(t.example).unwrap_or(Value::Null),
                })
            })
            .collect(),
    )
}

/// The full machine-readable self-description. `commands` comes from the CLI's clap definition.
pub fn all_json(commands: &Value) -> Value {
    let (props, prefabs) = (crate::props::PropKind::ALL.len(), crate::prefabs::builtin().0.defs.len());
    json!({
        "engine": "red_engine2 (Red Engine 2): JSON scene -> wgpu 3D; `re2` plays a map, `red_engine2` analyzes/edits/renders it",
        "counts": { "props": props, "prefabs": prefabs, "recipes": crate::tools::recipes::all().len(), "lint_codes": LINT_CODES.len() },
        "topics": TOPICS.iter().map(|(n, d)| json!({"topic": n, "about": d})).collect::<Vec<_>>(),
        "commands": commands,
        "common_fields": COMMON_FIELDS.iter().map(|(n, t, d)| json!({"name": n, "type": t, "desc": d})).collect::<Vec<_>>(),
        "objects": types_json(),
        "scene": SCENE_KEYS.iter().map(|(k, d)| json!({"key": k, "about": d})).collect::<Vec<_>>(),
        "lint": LINT_CODES.iter().map(|(c, s, d)| json!({"code": c, "severity": s, "about": d})).collect::<Vec<_>>(),
        "physics": physics().iter().map(|(k, v, d)| json!({"name": k, "value": v, "about": d})).collect::<Vec<_>>(),
        "conventions": CONVENTIONS.iter().map(|(k, d)| json!({"topic": k, "rule": d})).collect::<Vec<_>>(),
    })
}

fn commands_text(commands: &Value, brief: bool) -> String {
    let mut out = String::new();
    for c in commands.as_array().into_iter().flatten() {
        let name = c["name"].as_str().unwrap_or("?");
        let about = c["about"].as_str().unwrap_or("");
        if brief {
            out.push_str(&format!("  {name:<11} {}\n", about.split(". ").next().unwrap_or(about).trim_end_matches('.')));
            continue;
        }
        let args: Vec<String> = c["args"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|a| {
                let n = a["name"].as_str().unwrap_or("?");
                let opt = a["required"].as_bool() != Some(true);
                match (a["positional"].as_bool(), a["flag"].as_bool()) {
                    (Some(true), _) => if opt { format!("[{n}]") } else { format!("<{n}>") },
                    (_, Some(true)) => format!("[--{n}]"),
                    _ => format!("[--{n} <v>]"),
                }
            })
            .collect();
        out.push_str(&format!("{name} {}\n    {about}\n", args.join(" ")));
    }
    out
}

/// Renders one `describe` topic as text or JSON; `Err` for an unknown topic (with a did-you-mean).
pub fn render(topic: &str, commands: &Value, json_out: bool) -> Result<String, String> {
    if json_out || topic == "all" {
        let all = all_json(commands);
        let v = match topic {
            "overview" | "all" => all,
            "commands" => all["commands"].clone(),
            "objects" => json!({"common_fields": all["common_fields"], "objects": all["objects"]}),
            "scene" => all["scene"].clone(),
            "lint" => all["lint"].clone(),
            "physics" => all["physics"].clone(),
            "conventions" => all["conventions"].clone(),
            "glossary" => json!({"markdown": search::GLOSSARY}),
            "decisions" => json!({"markdown": search::ADR_INDEX}),
            other => return Err(unknown(other)),
        };
        return Ok(serde_json::to_string_pretty(&v).unwrap() + "\n");
    }
    let mut out = String::new();
    match topic {
        "overview" => {
            let (props, prefabs) = (crate::props::PropKind::ALL.len(), crate::prefabs::builtin().0.defs.len());
            out.push_str("Red Engine 2: maps are JSON scenes. `re2 <map>` plays one; `red_engine2` validates, analyzes, edits and renders them.\n");
            out.push_str("You should never need to read Rust: everything is reachable through these commands.\n\n");
            out.push_str(&format!("Building blocks: {props} props (Rust-made, real collision) + {prefabs} prefabs (JSON, parametric) + primitives + wall/fence/stairs macros.\n"));
            out.push_str(&format!("Known-good starting points: {} recipes (`red_engine2 recipe`).\n\n", crate::tools::recipes::all().len()));
            out.push_str("Workflow:  recipe/catalog -> add/set/move (auto-validated) -> lint -> plan/tour/frame (LOOK) -> verify\n\n");
            out.push_str("Commands:\n");
            out.push_str(&commands_text(commands, true));
            out.push_str("\nTopics (`red_engine2 describe <topic>`):\n");
            for (n, d) in TOPICS {
                out.push_str(&format!("  {n:<12} {d}\n"));
            }
            out.push_str("\nAlso: `red_engine2 search <words>` finds docs/assets/symbols; `red_engine2 src find|show|refs` explores the Rust without reading it.\n");
        }
        "commands" => out.push_str(&commands_text(commands, false)),
        "objects" => {
            out.push_str("Every object has:\n");
            for (n, t, d) in COMMON_FIELDS {
                out.push_str(&format!("  {n:<12} {t:<34} {d}\n"));
            }
            for t in TYPES {
                out.push_str(&format!("\n{} — {}\n", t.name, t.summary));
                for (n, ty, d) in t.fields {
                    out.push_str(&format!("    {n:<14} {ty:<32} {d}\n"));
                }
                out.push_str(&format!("  e.g. {}\n", t.example));
            }
        }
        "scene" => {
            for (k, d) in SCENE_KEYS {
                out.push_str(&format!("{k:<16} {d}\n"));
            }
        }
        "lint" => {
            for (c, s, d) in LINT_CODES {
                out.push_str(&format!("{c:<14} {s:<5} {d}\n"));
            }
            out.push_str("\nSilence one on one object with \"lint_ignore\": [\"code\"]. `lint` is for walkable maps; cinematic scenes trip `leak`/`spawn`.\n");
        }
        "physics" => {
            for (k, v, d) in physics() {
                out.push_str(&format!("{k:<26} {v:<8} {d}\n"));
            }
        }
        "conventions" => {
            for (k, d) in CONVENTIONS {
                out.push_str(&format!("{k:<15} {d}\n"));
            }
        }
        "glossary" => out.push_str(search::GLOSSARY),
        "decisions" => {
            out.push_str(search::ADR_INDEX);
            out.push_str("\nRead one with `red_engine2 search <topic> --kind adr` (or open docs/adr/NNNN-*.md).\n");
        }
        other => return Err(unknown(other)),
    }
    Ok(out)
}

fn unknown(t: &str) -> String {
    let hint = crate::prefabs::suggest(t, TOPICS.iter().map(|(n, _)| *n));
    format!("unknown topic '{t}'{} — topics: {}", if hint.is_empty() { String::new() } else { format!(" (did you mean {}?)", hint[0]) }, TOPICS.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene_with(obj: &str) -> String {
        format!(r#"{{"camera":{{"fov":50,"position":[0,2,8],"target":[0,1,0]}},"objects":[{obj}]}}"#)
    }

    #[test]
    fn every_example_parses() {
        for t in TYPES {
            let r = crate::schema::parse_scene(&scene_with(t.example));
            assert!(r.is_ok(), "example for `{}` is invalid: {:?}", t.name, r.err());
        }
    }

    #[test]
    fn every_object_type_the_schema_accepts_is_described() {
        // The schema's own "unknown type" error lists what it accepts; describe must cover each.
        let err = crate::schema::parse_scene(&scene_with(r#"{"id":"x","type":"nope"}"#)).unwrap_err().join(" ");
        let listed = err.split("expected ").nth(1).unwrap_or("");
        for name in listed.trim_end_matches(')').replace(" or ", ", ").split(',').map(str::trim).filter(|s| !s.is_empty()) {
            assert!(TYPES.iter().any(|t| t.name == name), "schema accepts type `{name}` but `describe objects` doesn't cover it");
        }
    }

    #[test]
    fn lint_table_covers_every_code_the_linter_can_emit() {
        let src = include_str!("lint.rs");
        let mut missing = Vec::new();
        for (i, _) in src.match_indices("finding(") {
            let rest = &src[i..];
            let Some(q1) = rest.find('"') else { continue };
            // the code is the first string literal after `finding(Severity::X,` — skip calls
            // whose first literal is beyond the argument list start (e.g. format! messages)
            if q1 > 40 {
                continue;
            }
            let Some(q2) = rest[q1 + 1..].find('"') else { continue };
            let code = &rest[q1 + 1..q1 + 1 + q2];
            if code.chars().all(|c| c.is_ascii_lowercase() || c == '-') && !LINT_CODES.iter().any(|(c, _, _)| *c == code) {
                missing.push(code.to_string());
            }
        }
        missing.dedup();
        assert!(missing.is_empty(), "lint codes not described in describe::LINT_CODES: {missing:?}");
    }

    #[test]
    fn overview_and_every_topic_render() {
        let cmds = json!([{"name": "lint", "about": "Static map checker. More.", "args": [{"name": "scene", "positional": true, "required": true}]}]);
        for (t, _) in TOPICS {
            assert!(render(t, &cmds, false).is_ok(), "topic {t}");
            assert!(render(t, &cmds, true).is_ok(), "topic {t} json");
        }
        assert!(render("nope", &cmds, false).is_err());
    }
}
