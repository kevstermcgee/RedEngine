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
    ("brief", "a ~1 KB summary: binaries, workflow, commands, where to look next (the cheapest first read)"),
    ("overview", "what the engine is + the topics below"),
    ("commands", "every CLI command and its flags (generated from the real CLI)"),
    ("objects", "every object `type` with its fields and a working example"),
    ("scene", "top-level scene keys: meta, camera, lights, zones, prefabs, checks, ..."),
    ("lint", "every lint code: what it means and how to fix it"),
    ("physics", "player size/speed/step rules that decide what is walkable (live constants)"),
    ("conventions", "coordinates, origins, facing, naming — the things that cause silent mistakes"),
    ("glossary", "project vocabulary: props vs prefabs, zones, body band, the four maps, the tire-iron naming trap, ..."),
    ("decisions", "the architecture decision records (docs/adr): why the engine is built this way, one line each"),
    ("diagnostics", "the `--json` envelope every command can return, and every stable diagnostic code with its fix"),
    ("rules", "game logic as data: `vars` + `rules` (when / who / if / once / do), volumes, actions, expressions; run headless"),
    ("sim", "headless play-throughs (`sim`, scenarios in `checks.sim`) and match traces (`replay`, checksums, first divergent tick)"),
    ("multiplayer", "hosting and playing online: keys, lobby and rounds, UPnP, net-test, perf, package"),
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
            (
                "openings",
                "[{kind,at,width,height,sill,trim,glass}]",
                "kind door|window|arch; `at` = distance along the wall from `from` to the opening's center; doors must be >= 2.05 tall and >= 0.9 wide",
            ),
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
    ("schema_version", "optional whole number, currently 1 (omitted = 1); a newer number is refused with a message"),
    ("recipe", "{title, summary, teaches, tips} — what a known-good example map is for (`recipe` lists them)"),
    ("meta", "{fps, duration, resolution:[w,h]} — video/frame settings; resolution also sets `frame`/`tour` size"),
    ("background", "{sky_top, sky_bottom} gradient, or {color} flat"),
    ("ambient", "{color, intensity} flat fill light (0.1-0.4 typical)"),
    ("camera", "{fov, position, target, near, far} REQUIRED; for walkable maps position.xz is the player spawn"),
    ("post", "{ao, outline, ao_radius, enabled} clarity pass: contact shadows + silhouette outlines"),
    ("lights", "<= 16; {id, type: directional|point, color, intensity, direction|position, range, cast_shadows, shadow_radius, shadow_center}; one directional may cast shadows"),
    ("zones", "[{id, rect:[x0,z0,x1,z1], y, kind}] named rooms; give lint/reach/tour/plan names to talk about"),
    ("spawns", "[{id, position:[x,y,z], yaw_deg, group}] multiplayer spawn points (`red_server --spawn-group`); none = the camera position"),
    ("portals", "[{id, between:[zoneA,zoneB], center:[x,z], width, height, open}] doorway connectivity between zones (network interest, roadmap)"),
    ("interest", "{cell_size, note} network-interest settings (roadmap; rooms are the cells)"),
    ("vars", "{name: number|bool} game variables rules read and write (`describe rules`); built-ins: time, tick, players"),
    ("rules", "[{id, when, who, if, once, cooldown, do}] game logic as data: triggers, conditions, actions (`describe rules`)"),
    ("weapons", "{bat: {damage}, revolver: {damage, ammo: \"infinite\" | {loaded, capacity, reserve}}} the demo weapons' numbers; limited ammo is a scene edit"),
    ("match", "{min_players, countdown_secs, round_secs, results_secs, score_to_win, join_in_progress, ready_check} turns on the server's lobby -> countdown -> round -> results -> rematch flow (`describe multiplayer`); absent = open play"),
    ("prefabs", "scene-local prefab definitions {name: {params, objects, tags, desc, extends, collide, mount}} — shadow built-ins"),
    ("checks", "expectations `verify` runs: {lint, reach, walk, objects, views, sim, perf} — see SPEC; `perf` = budgets for tick time, bandwidth, promoted props (`red_engine2 perf`)"),
    ("objects", "the scene graph: array of objects (see `describe objects`)"),
    ("x-*, _*, notes, $comment", "the extension namespace: always allowed, never interpreted — put notes and tool data here. ANY OTHER unknown key is an error with a did-you-mean"),
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
        (
            "body_headroom_m",
            format!("{}", player::PLAYER_HEADROOM),
            "anything lower than this above the floor blocks walking (door headers, beams); doors must be at least this tall",
        ),
        ("walk_speed_mps", format!("{}", player::WALK_SPEED), ""),
        ("sprint_speed_mps", format!("{}", player::SPRINT_SPEED), ""),
        ("eye_height_m", format!("{}", player::STAND_EYE_HEIGHT), "camera height when standing (crouch: 1.05)"),
        ("jump_height_m", format!("{:.2}", player::JUMP_HEIGHT), "can hop onto low props"),
        ("step_up_m", "0.35".to_string(), "colliders whose top is <= 0.35 above the feet are stepped ONTO (slab edges, low props); taller ones block"),
        (
            "floors",
            "ground_height_at".to_string(),
            "a surface is walkable only if within ~0.35 of the current foot height — a second-floor slab is unreachable except via stairs",
        ),
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

/// The binaries the crate builds and what each is for (`describe brief`).
pub const BINARIES: &[(&str, &str)] = &[
    ("red_engine2", "analyze / edit / render maps (this CLI); builds without graphics for lint/reach/walk/verify"),
    ("re2", "play a map in a window (first person); `--connect HOST:PORT` joins a server"),
    ("red_server", "headless authoritative UDP multiplayer server (no window, GPU or audio: `--no-default-features`)"),
    ("red_bot", "headless scripted client: proves multiplayer without a window (JSON output)"),
];

/// The ~1 KB entry summary as JSON: what an AI should read before anything else.
pub fn brief_json(commands: &Value) -> Value {
    let names: Vec<&str> = commands.as_array().into_iter().flatten().filter_map(|c| c["name"].as_str()).collect();
    json!({
        "engine": "Red Engine 2: maps are JSON scenes; you never need to read Rust",
        "binaries": BINARIES.iter().map(|(n, d)| json!({"name": n, "about": d})).collect::<Vec<_>>(),
        "workflow": "recipe/catalog -> add/set/move (validated) -> lint -> plan/tour (look) -> verify",
        "commands": names,
        "global_flags": [{"flag": "--json", "about": "wrap any command's result in the stable envelope {schema, command, ok, exit, data, diagnostics, stderr}"}],
        "errors": "`path: message` with a stable code and, where possible, a did-you-mean fix (describe diagnostics)",
        "topics": TOPICS.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
        "next": ["describe <topic>", "search \"<question>\"", "catalog <word>", "recipe", "SPEC.md (scene language)", "AGENTS.md (workflow)"],
    })
}

fn brief_text(commands: &Value) -> String {
    let b = brief_json(commands);
    let list = |k: &str| b[k].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join(" ");
    let mut out = format!("{}.\n", b["engine"].as_str().unwrap_or(""));
    out.push_str("Binaries:\n");
    for (n, d) in BINARIES {
        out.push_str(&format!("  {n:<12} {d}\n"));
    }
    out.push_str(&format!("Workflow: {}\n", b["workflow"].as_str().unwrap_or("")));
    out.push_str(&format!("Commands: {}\n", list("commands")));
    out.push_str("Every command takes --json: one stable envelope {schema, command, ok, exit, data, diagnostics, stderr}.\n");
    out.push_str("Errors are `path: message` with a stable code and a did-you-mean fix (`describe diagnostics`).\n");
    out.push_str(&format!("Topics (describe <topic>): {}\n", list("topics")));
    out.push_str("Next: search \"<question>\" | catalog <word> | recipe | SPEC.md (scene language) | AGENTS.md (workflow)\n");
    out
}

/// A complete `vars` + `rules` + `checks.sim` example (parsed by a test, so `describe rules` cannot drift from the parser).
pub const RULES_EXAMPLE: &str = r##"{"camera":{"position":[0,1.7,-6],"target":[0,1,0]},
 "zones":[{"id":"goal","rect":[4,-2,6,2],"y":0}],
 "spawns":[{"id":"start","position":[-6,0,0],"yaw_deg":90}],
 "vars":{"score":0},
 "rules":[
  {"id":"take_coin","when":{"enter":{"object":"coin","pad":0.4}},"once":true,"do":[{"add":["score",1]},{"hide":"coin"},{"emit":"coin"}]},
  {"id":"win","when":{"enter":{"zone":"goal"}},"if":"score >= 1","do":[{"emit":"victory"},{"end":"victory"}]}],
 "checks":{"sim":[{"name":"coin then goal","players":[{"id":"p1","spawn":"start"}],
   "script":[{"player":"p1","walk":"0,0; 5,0"}],
   "expect":[{"event":"coin","count":1},{"ended":"victory"},{"var":"score","eq":1}]}]},
 "objects":[{"id":"floor","type":"plane","size":[20,10],"position":[0,0.01,0]},
            {"id":"coin","type":"cylinder","radius":0.25,"height":0.08,"position":[0,0.45,0],"collide":false}]}"##;

fn rules_text() -> String {
    let mut out = String::from(
        "Game rules are data in the scene: `vars` (numbers/bools) and `rules`. Everything a rule names is validated when the scene loads.\n\n\
         rule = { id, when, who?, if?, once?, cooldown?, do }\n\
         \x20 when      exactly one of: {enter: VOLUME} {exit: VOLUME} {event: name} {every: secs} {after: secs} {start: true}\n\
         \x20 who       any (default) | human | rat\n\
         \x20 if        expression over the vars (and built-ins time, tick, players): `score >= 3 && !has_key`\n\
         \x20 once      fire at most once per match;  cooldown: minimum seconds between firings\n\
         \x20 do        actions, in order:\n",
    );
    for (name, help) in crate::sim::rules::ACTIONS {
        out.push_str(&format!("      {name:<9} {help}\n"));
    }
    out.push_str(
        "VOLUME = {zone: id [, height]} | {object: top-level id [, pad]} | {box: [x0,y0,z0,x1,y1,z1]}   (pad grows it, metres)\n\
         Expressions: numbers, true/false, variables, + - * / %, < <= > >= == !=, && || !, parentheses. x/0 = 0 (never NaN).\n\
         An unknown variable/zone/object/spawn/event is a validate error with a did-you-mean.\n\
         Rules run in the authoritative simulation (server, `sim`), deterministically; state (vars, hidden objects, outcome) is\n\
         part of the match checksum. Offline `re2` runs the same rule state machine: hide/show changes rendering, teleport/impulse\n\
         are applied, pickup/drop/shot/hit are injected, and vars/events/outcome appear in a generic HUD. Online clients use the same\n\
         HUD from a repeated bounded authoritative state, so loss, reconnect and late join recover it. Prove a rule with `checks.sim`\n\
         (see `describe sim`).\n\nExample scene:\n",
    );
    out.push_str(RULES_EXAMPLE);
    out.push('\n');
    out
}

fn multiplayer_text() -> String {
    String::from(
        "HOST     red_server --map maps/main.json [--port 27015] [--key SECRET|auto] [--lobby] [--upnp] [--record trace.json]\n\
         \x20 --key     joining needs the key: clients PROVE they know it (HMAC challenge/response), it is never sent, and every datagram after the\n\
         \x20           handshake carries an authentication tag, so forged / replayed / injected packets are dropped (ADR 0028). `auto` makes 128 random\n\
         \x20           bits and prints them. NOT encrypted: traffic is readable; use a long random key, not a short word.\n\
         \x20 --lobby   the lobby flow with defaults; a scene `\"match\": {min_players, countdown_secs, round_secs, results_secs, score_to_win,\n\
         \x20           join_in_progress, ready_check}` turns it on with the map's own settings; `--min-players --countdown-secs --round-secs\n\
         \x20           --results-secs --score-to-win` override either. Without any of these: open play (join = play).\n\
         \x20 --upnp    open the UDP port on a home router (UPnP), renew it, remove it on exit. `red_engine2 portmap status|enable|remove|keep`.\n\
         \x20 Every setting is also an env var (RED_KEY, RED_LOBBY, RED_UPNP, RED_ROUND_SECS, ...); docs/HOSTING.md has Docker and systemd.\n\
         PLAY     re2 --connect HOST:PORT [--key K] [--name N] map.json      (or `re2 map.json`, then PLAY ONLINE / the O key opens the connect form)\n\
         \x20 The lobby shows the roster (names, character, ping, ready); R ready, C character, Esc leave. Countdown -> round (HUD: ping, timer,\n\
         \x20 scoreboard) -> results -> everyone pressing Ready again is a rematch. Late joiners play at once or watch until the next round.\n\
         BOT      red_bot --server HOST:PORT [--key K] [--name N] [--ready]   (a scripted headless client that also readies up)\n\
         PROVE IT\n\
         \x20 red_engine2 net-test scene.json --profile bad|all   real server + clients behind a seeded bursty-lossy laggy proxy; judged on what a\n\
         \x20                                                    player notices (disconnects, prediction, remote players gliding, bandwidth)\n\
         \x20 red_engine2 perf scene.json                          sim/server tick percentiles, bytes per client, promoted props vs `checks.perf`\n\
         \x20 red_engine2 sim scene.json                           scripted headless play-throughs of the rules;  replay trace.json = first divergent tick\n\
         SHIP     red_engine2 package out.zip / package --verify out.zip   reproducible zip + SHA-256 manifest; headless binaries proven graphics-free\n\
         NOT DONE lag compensation for hitscan, payload encryption, spectator camera, teams, kick/ban.\n",
    )
}

fn sim_text() -> String {
    String::from(
        "red_engine2 sim <scene> [--scenario file.json] [--only name] [--trace out.json] [--checkpoint-every 1] [--dump-every 60]\n\
         \x20 Plays scripted players through the real authoritative simulation (no window, no GPU, no socket) and checks what happened.\n\
         \x20 Scenarios live in the scene's `checks.sim` (so `verify` runs them) or in a file. Exit 1 if any fails.\n\n\
         scenario = { name, players, script, expect, spawn_group?, max_seconds? (30), settle_seconds? (0.5) }\n\
         \x20 players  [{id, character: human|rat, spawn?: spawn id}]\n\
         \x20 script   [{player, walk: \"x,z; x,z\" | wait: secs | hold: {forward, strafe, sprint, crouch, jump, yaw_deg, seconds}, until_event?: name}]\n\
         \x20          each player's steps run in order; players run in parallel; a walk that gets stuck fails the scenario\n\
         \x20 expect   [{event: name, count|min|max} {no_event: name} {var: name, eq|ne|gt|gte|lt|lte: n} {ended: outcome} {not_ended: true}\n\
         \x20           {hidden|shown: object id} {player: id, near: [x,z], tol?, y?}]\n\
         The run ends when a rule `end`s the match, when every script is done (+ settle), or at max_seconds.\n\n\
         red_engine2 replay <trace.json> [--scene map.json] [--against other.json]\n\
         \x20 A trace records a match: header (engine, tick rate, map hash, seed, platform), every join/leave/input/impulse in order, game events,\n\
         \x20 a checksum of players / props / rules after every N ticks, and periodic state dumps. `replay` re-runs it with no renderer or\n\
         \x20 socket and reports the FIRST tick where players, props or rules differ, with a compact state diff. Exact (bit) checksums are\n\
         \x20 compared on the same platform; across platforms a millimetre-quantised `coarse` checksum separates float noise from a real desync.\n\
         \x20 Record one with `sim --trace`, or from a live match: `red_server --record out.json` (Ctrl-C to finish). `--dump-every 1` gives an\n\
         \x20 exact state diff at the divergent tick.\n",
    )
}

fn diagnostics_text() -> String {
    let mut out = String::from(
        "Every command accepts the global `--json` flag and then prints exactly one JSON document:\n\
         {\"schema\": 1, \"command\": \"lint\", \"ok\": false, \"exit\": 1,\n \
         \"data\": <what the command printed: parsed JSON, or {\"text\": ...}>,\n \
         \"diagnostics\": [{\"code\", \"path\", \"message\", \"fix\"?}], \"stderr\": \"...\"}\n\
         `ok` is exit == 0; a failing command can still carry `data` (lint lists its findings).\n\nDiagnostic codes:\n",
    );
    for (c, d, f) in crate::tools::envelope::CODES {
        out.push_str(&format!("  {c:<14} {d}\n{:<17}fix: {f}\n", ""));
    }
    out
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
                    (Some(true), _) => {
                        if opt {
                            format!("[{n}]")
                        } else {
                            format!("<{n}>")
                        }
                    }
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
            "brief" => brief_json(commands),
            "rules" => json!({
                "actions": crate::sim::rules::ACTIONS.iter().map(|(n, h)| json!({"action": n, "help": h})).collect::<Vec<_>>(),
                "when": ["enter", "exit", "event", "every", "after", "start"],
                "volumes": ["zone", "object", "box"],
                "builtin_vars": crate::sim::rules::BUILTIN_VARS,
                "example": serde_json::from_str::<Value>(RULES_EXAMPLE).unwrap_or(Value::Null),
            }),
            "sim" => json!({"text": sim_text()}),
            "multiplayer" => json!({"text": multiplayer_text()}),
            "diagnostics" => {
                json!({"envelope_schema": crate::tools::envelope::ENVELOPE_SCHEMA, "codes": crate::tools::envelope::CODES.iter().map(|(c, d, f)| json!({"code": c, "about": d, "fix": f})).collect::<Vec<_>>()})
            }
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
        "brief" => out.push_str(&brief_text(commands)),
        "diagnostics" => out.push_str(&diagnostics_text()),
        "rules" => out.push_str(&rules_text()),
        "sim" => out.push_str(&sim_text()),
        "multiplayer" => out.push_str(&multiplayer_text()),
        "overview" => {
            let (props, prefabs) = (crate::props::PropKind::ALL.len(), crate::prefabs::builtin().0.defs.len());
            out.push_str("Red Engine 2: maps are JSON scenes. `re2 <map>` plays one; `red_engine2` validates, analyzes, edits and renders them.\n");
            out.push_str("You should never need to read Rust: everything is reachable through these commands.\n\n");
            out.push_str(&format!(
                "Building blocks: {props} props (Rust-made, real collision) + {prefabs} prefabs (JSON, parametric) + primitives + wall/fence/stairs macros.\n"
            ));
            out.push_str(&format!("Known-good starting points: {} recipes (`red_engine2 recipe`).\n\n", crate::tools::recipes::all().len()));
            out.push_str("Workflow:  recipe/catalog -> add/set/move (auto-validated) -> lint -> plan/tour/frame (LOOK) -> verify\n\n");
            out.push_str("Commands:\n");
            out.push_str(&commands_text(commands, true));
            out.push_str("\nTopics (`red_engine2 describe <topic>`):\n");
            for (n, d) in TOPICS {
                out.push_str(&format!("  {n:<12} {d}\n"));
            }
            out.push_str(
                "\nAlso: `red_engine2 search <words>` finds docs/assets/symbols; `red_engine2 src find|show|refs` explores the Rust without reading it.\n",
            );
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
    format!(
        "unknown topic '{t}'{} — topics: {}",
        if hint.is_empty() { String::new() } else { format!(" (did you mean {}?)", hint[0]) },
        TOPICS.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
    )
}

#[cfg(test)]
mod tests {
    /// Every key the strict parser accepts at the scene root is described (and vice versa), so the doc cannot drift.
    #[test]
    fn describe_scene_lists_exactly_the_strict_root_keys() {
        for k in crate::strict::ROOT_KEYS {
            assert!(SCENE_KEYS.iter().any(|(d, _)| d == k), "`describe scene` does not mention the root key `{k}` (add it to SCENE_KEYS)");
        }
        for (d, _) in SCENE_KEYS.iter().filter(|(d, _)| !d.contains('*')) {
            assert!(crate::strict::ROOT_KEYS.contains(d), "`describe scene` lists `{d}` but the parser would reject it");
        }
    }

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

    /// `describe rules` shows a scene that must load, and its scenario must pass: the docs are executable.
    #[test]
    fn the_rules_example_is_a_valid_scene_whose_scenario_passes() {
        assert!(crate::schema::parse_scene(RULES_EXAMPLE).is_ok(), "{:?}", crate::schema::parse_scene(RULES_EXAMPLE).err());
        let dir = std::env::temp_dir().join("re2_describe_rules");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rules_example.json");
        std::fs::write(&path, RULES_EXAMPLE).unwrap();
        let (report, _) = crate::tools::simrun::run(&path, None, None, None).unwrap();
        assert!(report.all_passed(), "{}", report.render());
    }

    #[test]
    fn every_action_is_documented_in_the_topic() {
        let text = rules_text();
        for (name, _) in crate::sim::rules::ACTIONS {
            assert!(text.contains(name), "`describe rules` does not mention the `{name}` action");
        }
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
