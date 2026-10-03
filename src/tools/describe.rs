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

/// The most bytes `describe --brief` may take: it is the first thing every session reads (`tests/ai_tasks.rs` and `preflight` enforce it).
pub const BRIEF_BUDGET: usize = 2_500;
/// The most bytes the `describe` overview may take. A new command adds a line: keep its `about` short and put the detail in `docs/AGENT_REFERENCE.md`.
pub const OVERVIEW_BUDGET: usize = 7_300;

/// Topic names and one-line descriptions for `describe`; a test renders every one.
pub const TOPICS: &[(&str, &str)] = &[
    ("brief", "a ~1 KB summary: binaries, workflow, commands, where to look next (the cheapest first read)"),
    ("overview", "what the engine is + the topics below"),
    ("commands", "every CLI command and its flags (11 KB; one command: `search <name>`)"),
    ("objects", "every object `type` with its fields and a working example"),
    ("scene", "top-level scene keys: meta, camera, lights, zones, prefabs, checks, ..."),
    ("lint", "every lint code: what it means and how to fix it"),
    ("physics", "player size/speed/step rules that decide what is walkable (live constants)"),
    ("conventions", "coordinates, origins, facing, naming — the things that cause silent mistakes"),
    ("glossary", "project vocabulary: props vs prefabs, zones, body band, the four maps, the tire-iron naming trap, ..."),
    ("decisions", "the architecture decision records (docs/adr): why the engine is built this way, one line each"),
    ("diagnostics", "the `--json` envelope every command can return, and every stable diagnostic code with its fix"),
    ("rules", "game logic as data: `vars` + `rules` (when/who/if/once/do), volumes, actions, expressions"),
    ("sim", "headless play-throughs (`sim`, `checks.sim`) and match traces (`replay`, checksums)"),
    ("multiplayer", "hosting and playing online: keys, lobby and rounds, UPnP, net-test, perf, package"),
    ("playtest", "look at the game without a screen: `playtest`, headless scripts, pictures, the state dump, `expect`"),
    ("custom-client", "a game that is not first-person: your own crate on `red_engine2::app`"),
    ("all", "everything above as one JSON document (--json; 80 KB)"),
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
    ("rotation", "[rx,ry,rz] degrees = [0,0,0]", "composed Rx*Ry*Rz (Z acts first, then Y, then X, about the WORLD axes), so [90, yaw, 0] tips a yawed object over the world X axis: a cylinder laid on its side that way points across a heading, not along it. To lay one along a heading nest it: a group with rotation [0,yaw,0] holding the cylinder at [90,0,0]. Only Y (yaw) matters for upright objects"),
    ("scale", "number or [x,y,z] = 1", "uniform or per-axis"),
    ("material", "{color, metallic, roughness, emissive, opacity}", "color #rrggbb; metallic/roughness 0..1 (defaults 0/0.6); emissive #hex glows; opacity 0..1 (default 1) blends the surface over what is behind it (glass, water)"),
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
            ("material.color", "#hex", "alias for params.color when the prefab declares a color param"),
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
        name: "terrain",
        summary: "walkable heightfield ground: dunes, a shore, a headland (not scaled spheres). position = footprint centre (x, z) and the height 0 maps to (y); no rotation/scale. Where it exists it IS the ground (no floor at y = 0 under it). On a looping world it must span exactly the wrap range.",
        fields: &[
            ("size", "[x, z], required", "footprint in metres, at least 4 x 4"),
            ("cell", "number = 1", "metres between samples (0.1 to 32); or `resolution: [nx, nz]`"),
            ("generator", "{seed, profile: [[x, height], ...], noise: {amplitude, wavelength: n | [x, z], octaves, seed, fade_x: [x0, x1]}}", "one of generator / heights / heightmap: a smooth cross-section along x plus fractal noise, deterministic, periodic on a looping axis"),
            ("heights", "[[..z-major rows..]]", "explicit heights, exactly nz rows of nx numbers"),
            ("heightmap", "path.png + elevation_scale", "a grey image relative to the scene file; white = elevation_scale metres above position.y"),
            ("palette", "[[height, \"#hex\"], ...]", "vertex colours by height (ascending); multiplies material.color"),
            ("grain", "0..0.3", "brightness speckle on the palette"),
        ],
        example: r##"{"id":"shore","type":"terrain","position":[10,0,0],"size":[130,240],"cell":1,"generator":{"seed":11,"profile":[[-55,-3.2],[-2,0.04],[30,2.2],[75,9]],"noise":{"amplitude":0.7,"wavelength":[26,44],"fade_x":[-14,8]}},"palette":[[-3.2,"#33505c"],[0.12,"#b39c76"],[0.7,"#e6d2a6"]],"material":{"roughness":0.95}}"##,
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
    ("camera", "{fov, position, target, near, far} REQUIRED; for walkable maps without `spawns`, position.xz is the player spawn"),
    ("player", "{character: human|rat|wizard|cowboy|alien|robot, fov, walk_speed, sprint_speed, crouch_multiplier, jump_speed, gravity, acceleration, air_acceleration, friction, max_speed, throw_speed}; positive acceleration enables horizontal momentum; character locks a game to one body; throw_speed (default 1 m/s) is added along the look when a carried prop is released"),
    ("jump_pads", "[{id, position:[x,y,z], size:[w,d], launch_speed}] rectangular vertical launch volumes evaluated by shared movement"),
    ("post", "{ao, outline, ao_radius, enabled} clarity pass: contact shadows + silhouette outlines"),
    ("lights", "<= 16; {id, type: directional|point, color, intensity, direction|position, range, cast_shadows, shadow_radius, shadow_center, shadow_follow}; one directional may cast shadows; shadow_follow keeps the shadow map centred under the camera (open worlds)"),
    ("zones", "[{id, rect:[x0,z0,x1,z1], y, kind}] named rooms; give lint/reach/tour/plan names to talk about"),
    ("spawns", "[{id, position:[x,y,z], yaw_deg, group}] spawn points (`red_server --spawn-group`); the first one, at its y, is where offline play, lint, reach and walks start; none = the camera position at y 0"),
    ("portals", "[{id, between:[zoneA,zoneB], center:[x,z], width, height, open}] doorway connectivity between zones (network interest, roadmap)"),
    ("interest", "{cell_size, note} network-interest settings (roadmap; rooms are the cells)"),
    ("vars", "{name: number|bool} game variables rules read and write (`describe rules`); built-ins: time, tick, players"),
    ("rules", "[{id, when, who, if, once, cooldown, do}] game logic as data: triggers, conditions, actions (`describe rules`)"),
    ("phases", "{name: [rule ids]} named level states for lint/reach/walk/verify: the rules assumed to have fired, so a gate they open is open (`describe rules`)"),
    ("fields", "[{id, zone|object|box, velocity:[x,z], lift, rate}] force volumes on loose props: a river current, a conveyor, a wind tunnel; pulls props inside toward a target speed (SPEC \"Force fields\")"),
    ("weapons", "{starting, ladder: [weapon, ...], bat: {damage}, ammo: \"infinite\" | {loaded, capacity, reserve}}; starting accepts bat or any built-in firearm (pistol machine-pistol smg carbine rifle bullpup marksman shotgun lmg scout); ammo is one supply for every firearm; ladder = Gun Game: you carry ladder[kills] and cannot switch by hand"),
    ("shooter", "{start: [weapon, ...], friendly_fire, pickups: [{weapon|ammo:true, at:[x,y,z], respawn_secs}]} a loadout shooter (ADR 2026-09-30-killchain-loadout-shooter): every player carries up to 2 guns (own magazine and reserve each), 1 melee weapon and 2 grenades; weapons lie on the map and drop from the dead; 31 weapons in `arsenal`; two teams (spawn groups team1/team2), headshots, rockets, grenades, smoke, fire; absent = the classic single-weapon arena"),
    ("combat", "{respawn_secs, spawn: round_robin|farthest, spawn_protect_secs, regen_delay_secs, regen_per_sec} how fights are paced: respawn delay, where the dead return, spawn protection, health regeneration"),
    ("bots", "{fill, skill, roster: [{name, character, skill, style}]} AI players: the server fills empty slots up to `fill` players (humans included); skill = rookie|easy|normal|hard|nightmare or 0..1; style = balanced|rusher|sniper|acrobat (`describe bots`)"),
    ("nav", "{nodes: [{id, pos:[x,y,z]}], edges: [[from, to, kind?]]} the waypoint graph bots use to route known-good ways (stairs, jumps, drops); kind = walk (default, both ways) | jump | pad | drop; `red_engine2 nav check` replays every edge with the real movement. It is an aid, not a fence: a bot not on the graph still steers itself around walls and ledges with the real movement code, so it can and will reach areas the graph does not cover. To keep a bot out of a region, block it with level geometry (a locked door, a gap it cannot cross), not by leaving that region off the graph."),
    ("match", "{min_players, countdown_secs, round_secs, results_secs, score_to_win, join_in_progress, ready_check} turns on the server's lobby -> countdown -> round -> results -> rematch flow (`describe multiplayer`); absent = open play"),
    ("race", "{laps, gates:[zone ids, first = start/finish line], countdown_secs, finish_grace_secs, line, item_boxes:[zone ids], item_respawn_secs, surfaces:[{zone, kind: dirt|mud|water}]} a kart race: gates count in order and only in the direction of travel; per-player laps and standings live in `sim::race` (SPEC \"Races\")"),
    ("prefabs", "scene-local prefab definitions {name: {params, objects, tags, desc, extends, collide, mount}} — shadow built-ins"),
    ("music", "true | false: whether the standard client starts the built-in music loop (default true for older maps; `new-game` writes false: a game asks for music, it is never a default; the N key and the pause menu's MUSIC button still toggle it). Not every game needs music: leave it `false` when it would compete with gameplay audio cues, fight a game's own tone, or just feel wrong for the map; silence is a legitimate, complete answer, not an unfinished one."),
    ("flashlight", "true | false (default false): gives the player a toggleable point light that follows the camera (the T key); a Point, not a cone — the engine has no spotlight kind. Good for a dark map that needs the player to actively light their own way rather than being lit for them."),
    ("death_text", "a string (default none, meaning \"ELIMINATED\"): the title the standard client's death screen shows while waiting to respawn. The arena-shooter wording is not right for every genre — a horror game, a race, anything else with its own tone can say what being caught/crashed/out actually means there."),
    ("teams", "true | false (default false): lets players (human or bot) be assigned to team 1 or 2 outside a loadout `shooter` match, so `who: team1`/`who: team2` can be used in `rules`. A `shooter` block already has teams regardless of this flag; this is for a non-shooter game with asymmetric roles (hide-and-seek, capture-the-flag, anything two-sided)."),
    ("hud", "{enabled, show_combat, show_crosshair, show_ping, show_scoreboard, show_round, show_events, show_help, show_rules_vars, custom_vars: [var, ...]} which on-screen display is drawn; `enabled:false` is a clean screen; defaults are all-on, or combat/crosshair/ping/scoreboard/round off when `player.mode` is peaceful"),
    ("world", "{wrap: {axis: x|z, min, max}, bounds: {x: [lo, hi], z: [lo, hi]}} an endless world: the axis loops every max-min metres (author one period; the seam is invisible), bounds are the invisible edge of the other axes"),
    ("sky", "{sun: {direction:[x,y,z] toward the sun, size_deg, color, glow}, haze, zenith, gradient_power} a sky dome shaded by view direction with a sun at infinity that sets behind the horizon instead of dipping under the ground; without it `background` is a screen-space gradient"),
    ("ocean", "{y, color_deep, color_shallow, foam_color, wave_amplitude, wave_frequency, wave_speed, roughness} an endless animated water plane to the horizon, translucent over a shore (reads the scene's first terrain), fading into the sky haze"),
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
    ("speed", "warn", "`player.throw_speed` or a rule `impulse` speed above the 14 m/s prop cap: it saturates and behaves exactly like 14 -> lower it"),
    ("carry", "info", "a `movable: true` prop is too big for a human to pick up (0.45 m^3 / 1.25 m) -> shrink it, or keep it shove-only on purpose"),
    ("drop", "warn", "walkable edge with a big fall and no railing (open stairwell) -> add a 1.05 m railing wall"),
    ("leak", "error", "the player can walk off the map: a gap in the perimeter -> close it with wall/fence (not checked when the scene has a `world` block: its bounds are the edge)"),
    ("terrain-slope", "error", "terrain steeper than 45 degrees where players can walk (inside `world.bounds`): a player climbs any slope instantly -> soften the profile/noise or move it outside the bounds"),
    ("zone", "error", "a declared zone is unreachable or partly sealed -> open a door/arch or fix the zone rect"),
    ("floor", "warn", "a floor slab nobody can reach -> connect it or remove it"),
    ("unreachable", "warn", "a prop the player can't get near -> open a path or drop it"),
    ("door-blocked", "error", "furniture in front of / a wall behind a doorway -> clear 0.9 m each side"),
    ("door", "warn", "a connection narrower than 0.9 m -> widen"),
    ("spawn", "error", "the spawn (spawns[0] at its height, else the camera xz) is inside a solid or off any floor -> move it"),
    ("light", "warn", "a lamp sits inside a wall -> move it into the room"),
    ("z-fight", "warn", "coplanar overlapping planes flicker -> offset one by 0.01 or shrink"),
    ("duplicate-id", "error", "two objects share an id -> rename one"),
    ("reach", "error", "the flood-fill from spawn failed (spawn enclosed) -> see spawn"),
    (
        "interest",
        "error",
        "a map with spawns whose zones have no portals (or whose spawn zones no portal chain joins): players in other rooms are invisible -> add portals; warns when spawn zones are more portals apart than interest.hops",
    ),
    (
        "jump-clearance",
        "warn",
        "an overhead slab out of reach standing but within this map's jump shoves a jumper sideways -> raise its underside to 2.0 m + jump apex (jump_speed^2 / 2 gravity from `player`), or lower it under 2.05 m",
    ),
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
        (
            "scene_player_tuning",
            "player{...}".to_string(),
            "a scene may lock the body every human wears (`humans_play_as`; bots wear their own) and override human FOV, walk/sprint, crouch, jump and gravity; omission keeps selection and defaults",
        ),
        (
            "jump_pads",
            "jump_pads[]".to_string(),
            "touching a pad at its foot height applies its authored upward launch speed in the same deterministic step used online",
        ),
        (
            "max_prop_speed_mps",
            format!("{}", crate::physics::MAX_SPEED),
            "no loose prop ever moves faster than this. `player.throw_speed` (schema range 0-30) and rule `impulse` speeds above it are accepted but saturate: 20 and 30 behave exactly like this cap. Plan a throw with the effective speed below",
        ),
        (
            "prop_material",
            "friction 0.7, restitution 0.2, damped".to_string(),
            "every loose prop is ONE box collider with one density (mass = volume x 120 kg/m^3, see `info <id>`); a sphere or cylinder prop is a box collider too, so nothing rolls. Sliding friction removes about 7 m/s^2; a tall barrel tips instead of sliding; a crate thrown flat travels about 8 m from a 14 m/s release",
        ),
        (
            "release_velocity",
            "own velocity + throw_speed * look".to_string(),
            "a released prop leaves with the holder's horizontal AND vertical velocity plus `player.throw_speed` (default 1 m/s) along the look direction, pitch included: one function (sim::player::release_velocity) offline and on the server, so a sprint-and-release throw is the same game everywhere",
        ),
        (
            "release_grace_ticks",
            format!("{}", crate::physics::RELEASE_GRACE_TICKS),
            "after a release the holder's body ignores that prop for this many ticks, so the arc decides where it lands, not whether you keep running into it",
        ),
        (
            "hold_pose",
            "box sweep".to_string(),
            "a carried prop sits in front of the eye and is pulled in when its own box, swept ahead at the held height, would enter fixed geometry (walls lower than the eye count); looking up lifts it, looking down lowers it about 0.4 m",
        ),
        (
            "topple_direction",
            "front = local +Z".to_string(),
            "a tall loose prop struck from the front falls toward its local -Z: with rotation yaw Y it falls toward (-sin Y, -cos Y), so a domino line along +X needs yaw -90 and each next piece sits along the previous piece's fall direction; measured (Domino Halls): sculpture_monolith lines topple at 0.8-1.8 m spacing, a 90 degree corner needs a fan of three 30 degree pieces, crates/barrels/cones slide and never topple",
        ),
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
        "engine_changes": "context <feature|file|words> (5-15 KB work packet) -> edit -> affected --quick (owners, seconds) -> affected (+dependents) -> affected --full (= CI) before pushing",
        "commands": names,
        "global_flags": [{"flag": "--json", "about": "wrap any command's result in the stable envelope {schema, command, ok, exit, data, diagnostics, stderr}"}],
        "errors": "`path: message` with a stable code and, where possible, a did-you-mean fix (describe diagnostics)",
        "topics": TOPICS.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
        "next": ["describe <topic>", "search \"<question>\"", "catalog <word>", "recipe", "SPEC.md (scene language)", "AGENTS.md (workflow)", "describe custom-client (not first-person)"],
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
    out.push_str(&format!("Changing the engine (Rust): {}\n", b["engine_changes"].as_str().unwrap_or("")));
    out.push_str(&format!("Commands: {}\n", list("commands")));
    out.push_str("Every command takes --json: one stable envelope {schema, command, ok, exit, data, diagnostics, stderr}.\n");
    out.push_str("Errors are `path: message` with a stable code and a did-you-mean fix (`describe diagnostics`).\n");
    out.push_str(&format!("Topics (describe <topic>): {}\n", list("topics")));
    out.push_str("Next: search \"<question>\" | catalog <word> | recipe | SPEC.md (scene language) | AGENTS.md (workflow)\n");
    out.push_str("Not first-person? describe custom-client: your own crate on red_engine2::app; gameplay stays in scene rules\n");
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
         \x20           {prop_enter: VOLUME} {prop_exit: VOLUME} (a loose prop's origin crosses in/out; add `prop: id` beside it for one prop)\n\
         \x20           {prop_below: [prop_id, y]} (its origin drops below y metres)\n\
         \x20 who       any (default) | human | rat | team1 | team2   (player triggers only; team1/team2 need \"teams\": true or a shooter block)\n\
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
         Built-in functions read the loose props (a prop's id, or a zone's, as the argument; a prop that is not loose is a validate error):\n",
    );
    for (_, _, help) in crate::sim::rules_expr::Func::ALL {
        out.push_str(&format!("      {help}\n"));
    }
    out.push_str(
        "Engine events a rule can react to (`when: {event}`): pickup, drop, shot, hit, kill, respawn, swing (a bat swing started),\n\
         prop_hit (a bat or bullet struck a loose prop). A prop is inside a volume when its origin is inside in x/z and its height\n\
         band overlaps in y; `tilt` is measured from how the map placed it. An unknown variable/zone/object/prop/spawn/event is a\n\
         validate error with a did-you-mean.\n\
         Edges, measured: rules run in declaration order each tick, but a body walking from one volume into the next overlaps both for\n\
         a moment, so the next volume's `enter` fires a tick or more BEFORE the previous volume's `exit`: an `exit` rule that resets a\n\
         var undoes the `enter` rule's work (key on `enter`, or use one var per volume). A player or prop that starts inside a volume\n\
         gets no `enter` (use a `start` rule). A var whose name starts with `_` is internal: the generic HUD does not show it.\n\
         Rules run in the authoritative simulation (server, `sim`), deterministically; state (vars, hidden objects, outcome, prop\n\
         occupancy) is part of the match checksum. Offline `re2` runs the same rule state machine: hide/show changes rendering, collision\n\
         changes static movement/ground collision for a top-level object, teleport/impulse/reset/place are applied, the engine events\n\
         are injected, and vars/events/outcome appear in a generic HUD. Online clients use the same HUD from a repeated bounded\n\
         authoritative state, so loss, reconnect and late join recover it. Prove a rule with `checks.sim` (see `describe sim`).\n\
         Level states: lint/reach/walk see a gate closed unless an unconditional `start` rule opens it. Name the states a rule creates with\n\
         `\"phases\": {\"gate_open\": [\"open_gate\"]}` (rule ids assumed fired; their collision/deactivate/activate effects apply); then\n\
         `checks.reach`/`checks.walk` entries take `\"phase\": \"gate_open\"`, a reach entry `\"reachable\": false` (still cut off), `lint` drops an\n\
         unreachable zone/floor/prop that some phase reaches, and `reach|lint|walk|plan --phase gate_open` looks by hand. `recipe gated_garden`.\n\nExample scene:\n",
    );
    out.push_str(RULES_EXAMPLE);
    out.push('\n');
    out
}

fn multiplayer_text() -> String {
    String::from(
        "HOST     red_server --map maps/main.json [--port 27015] [--key SECRET|auto] [--lobby] [--upnp] [--record trace.json]\n\
         \x20         [--tls-cert DIR/cert.pem --tls-key DIR/key.pem] [--max-connections 16]      (identity: red_engine2 net-identity --out DIR)\n\
         \x20 TRANSPORT with --tls-cert/--tls-key: QUIC + TLS 1.3, encrypted, the server verified by its printed fingerprint (ADR 0044). Without:\n\
         \x20           development UDP (authenticated, NOT encrypted), loopback only unless --insecure-public-udp; a public bind refuses to start.\n\
         \x20 --key     joining needs the key: clients PROVE they know it (HMAC challenge/response), it is never sent, and every datagram after the\n\
         \x20           handshake carries an authentication tag, so forged / replayed / injected packets are dropped (ADR 0028). `auto` makes 128 random\n\
         \x20           bits and prints them. On QUIC the proof is bound to the TLS connection; on development UDP use a long random key.\n\
         \x20 --lobby   the lobby flow with defaults; a scene `\"match\": {min_players, countdown_secs, round_secs, results_secs, score_to_win,\n\
         \x20           join_in_progress, ready_check}` turns it on with the map's own settings; `--min-players --countdown-secs --round-secs\n\
         \x20           --results-secs --score-to-win` override either. Without any of these: open play (join = play).\n\
         \x20 --upnp    open the UDP port on a home router (UPnP), renew it, remove it on exit. `red_engine2 portmap status|enable|remove|keep`.\n\
         \x20 Every setting is also an env var (RED_KEY, RED_LOBBY, RED_UPNP, RED_ROUND_SECS, ...); docs/HOSTING.md has Docker and systemd.\n\
         PLAY     re2 --connect HOST:PORT --server-fingerprint sha256:... [--key K] [--name N] map.json   (a loopback server needs no fingerprint;\n\
         \x20         --server-ca ca.pem --server-name host for a CA certificate; --dev-udp joins a development server elsewhere, in plaintext;\n\
         \x20         or `re2 map.json`, then PLAY ONLINE / the O key opens the connect form)\n\
                  re2 --host [--fill N] [--bot-skill L] map.json      hosts the map on a thread of the game (bots and flow from the map's blocks) and joins it (ADR 0054)
\
         \x20 The lobby shows the roster (names, character, ping, ready); R ready, Esc leave (a race lobby: arrows pick an animal). Countdown -> round (HUD: ping, timer,\n\
         \x20 scoreboard) -> results -> everyone pressing Ready again is a rematch. Late joiners play at once or watch until the next round.\n\
         BOT      red_bot --server HOST:PORT [--key K] [--name N] [--ready]   (a scripted headless client that also readies up)\n\
         PROVE IT\n\
         \x20 red_engine2 net-test scene.json --profile bad|all [--transport quic]   real server + clients behind a seeded bursty-lossy laggy proxy; judged on what a\n\
         \x20                                                    player notices (disconnects, prediction, remote players gliding, bandwidth)\n\
         \x20 red_engine2 perf scene.json                          sim/server tick percentiles, bytes per client, promoted props vs `checks.perf`\n\
         \x20 red_engine2 sim scene.json                           scripted headless play-throughs of the rules;  replay trace.json = first divergent tick\n\
         SEE IT   red_engine2 playtest MAP                    the real client, no window: pictures, a contact sheet, what was drawn (`describe playtest`)\n\
         SHIP     red_engine2 package out.zip / package --verify out.zip   reproducible zip + SHA-256 manifest; headless binaries proven graphics-free\n\
         LAG      the server rewinds the players a hitscan shot can hit by the shooter's view lag (ADR 0053); `--lag-comp-ms` caps it.\n\
         NOT DONE client certificates, server key rotation, a spectator seat (the playtest's camera is client-side), teams, kick/ban.\n",
    )
}

fn playtest_text() -> String {
    let mut s = String::from(
        "Look at the game, and assert on what the player sees, without a screen (ADR 2026-09-28-seeing-what-the-player-sees).\n\n\
         red_engine2 playtest MAP [--secs 60] [--shots 12] [--out out/playtest] [--script play.json] [--fill N] [--bot-skill L]\n\
         \x20 The real client with no window, hosting its own match: a scripted player spins, walks, aims and fires; pictures come from the player's eyes, third\n\
         \x20 person, above the map and behind another player (rendered OFFSCREEN: no focus, no visible desktop, nothing to photograph by accident). Writes\n\
         \x20 <out>/NN-name.png, <out>/contact-sheet.png and <out>/playtest.json (the state dump); exit 1 when a player was undrawn or an expectation failed.\n\
         re2 MAP --host --headless --script play.json --dump state.json      the same loop with your own script (add --shot-dir out/ for pictures)\n\
         re2 MAP --shot-at 5,10,20 --shot-dir out/                          pictures from a normal windowed session; F12 takes one now; F3 shows the counters\n\
         re2 --debug-help                                                    every RE2_* switch and hotkey\n\n\
         SCRIPT   {\"policy\": \"idle|sentry|walker\", \"steps\": [ ... ]} - one action per step, run in order; timed steps take their time\n",
    );
    for (action, what) in [
        ("wait: secs", "do nothing"),
        ("look: {yaw, pitch}", "face a direction (degrees; yaw 0 looks along -Z, clockwise from above)"),
        ("turn: deg, over: secs", "turn by an angle over a time (a spin is 360)"),
        ("hold: [keys], secs", "hold forward back left right sprint crouch (policy idle)"),
        ("jump / interact / switch: n", "tap Space, tap E, scroll the wheel"),
        (
            "approach / look_at / interact: \"id\"",
            "by object, not keys: walk up to it (within?, timeout?), face its middle, or approach+face+press E and check it is carried (offline)",
        ),
        ("fire: n | {clicks, every} | {secs}", "click n times / hold the trigger; track: true keeps aiming at the nearest visible enemy"),
        ("aim_at: \"nearest\"", "turn to the nearest remote player in line of sight"),
        ("view / policy", "first|third person; idle (still), sentry (turns and fires), walker (circles and fires)"),
        ("shot: name, camera", "save a picture: first, third, overview, follow, follow:ID, or {eye, at, fov}"),
        ("snapshot: name", "store the state dump under that name"),
        (
            "expect: {at, eq|ne|min|max|contains|exists, within?, msg?}",
            "assert on the state (a JSON pointer); within waits up to that many seconds; wait_for = the same, 20 s",
        ),
        ("say: text", "print a line"),
    ] {
        s.push_str(&format!("  {action:<58} {what}\n"));
    }
    s.push_str(
        "\nSTATE    /remote/{in_view,drawn,undrawn,standins,unposed,roster_others,hidden_by_interest,players[],pool,counters}   every way another player can fail to be drawn\n\
         \x20        /online/{connected,id,ping_ms,phase,in_round,round,roster[]}  /player/{pos,yaw_deg,pitch_deg,weapon,hp,dead}  /view  /streaks\n\
         \x20        /hud/lines[{id,text}]  /cues/{counts,recent[]}  /crosshair/{state,enemy,pickup,in_reach}  /shots[]  /snapshots  /failures[]\n\
         \x20        /rules/{vars[{name,value}],ended,last_event,hidden[]}  /props[{id,pos,tilt_deg,moved,asleep,held_by}] (offline only; null online)\n\
         EXAMPLE  {\"steps\":[{\"wait_for\":{\"at\":\"/online/in_round\",\"eq\":true}},{\"expect\":{\"at\":\"/remote/drawn\",\"eq\":7,\"msg\":\"8 fighters means 7 drawn\"}},\n\
         \x20         {\"turn\":360,\"over\":6},{\"shot\":\"spin\"},{\"expect\":{\"at\":\"/remote/undrawn\",\"eq\":0}}]}\n\
         LOUD     an undrawn player also prints `warning: player N wears BODY ...` once, counts in RE2_STATS and the F3 overlay, and `game check` verifies every\n\
         \x20        body a map's bots wear has an avatar; `lint` reports zones without portals (`interest`) and slabs a jump shoves you under (`jump-clearance`).\n",
    );
    s
}

/// `describe custom-client`: the route for a game that is not the built-in first-person client (ADR 0043).
fn custom_client_text() -> String {
    String::from(
        "A game that is not first-person (top-down, strategy, puzzle, a spectator view) is its own crate that depends on red_engine2\n\
         and uses the client layer `red_engine2::app`. Gameplay stays in the scene (`vars`/`rules`, `describe rules`), proven with\n\
         `checks.sim`; the client turns input into movement and draws what the simulation reports. Do not copy re2 or engine source.\n\n\
         Cargo.toml   [dependencies] red_engine2 = { path = \"../red-engine-2\" }   (glam is re-exported: red_engine2::glam)\n\
         LOAD + PLAY  app::LocalSession::load(path): strict validation; one player in the authoritative MatchSim (the server's simulation)\n\
         \x20            .advance(dt, |state| PlayerInput { forward, strafe, yaw, .. }): fixed 60 Hz ticks;  .step(input) = one tick (tests)\n\
         \x20            .player() .player_feet() (interpolated)  .rules() .hidden() .outcome() .hud()  .scene_mut() = presentation only\n\
         \x20            app::input_toward(&state, target_xz, arrive) walks to a point (click-to-move, scripts)\n\
         CAMERA       app::ViewCamera::top_down(focus, height, tilt_deg, yaw_deg) | ::look_at(eye, target) | FpsCamera::view()\n\
         \x20            .screen_ray(px, py, w, h)  .pick_ground(px, py, w, h, y)  .world_to_screen(p, w, h)   (yaw 0: -Z is up on screen)\n\
         WINDOW       app::run(game, WindowOptions::default()) with `impl ClientGame`: scene(), update(&InputState, Frame) -> bool,\n\
         \x20            camera(w, h); optional hidden() (session.hidden()), hud_key() + hud(w, h) (session.hud().key() / .layout(w, h)), title()\n\
         INPUT        InputState: held(KeyCode::KeyW) pressed(..) wasd() clicked(MouseButton::Left) -> (x, y) wheel(); set_key/click in tests\n\
         DRAW         app::place_object(scene, id, pos, yaw) moves a marker; objects are uploaded once: move or hide them, do not add new ones\n\
         CHECK        app::Offscreen::new(scene, w, h)?.render(scene, t, &camera, hidden, Some(&hud)) -> RGBA (or save_png), no window\n\
         OWN LOOP     app::WindowGpu + InputState + viewer::LiveRenderer::world(..).render_view(.., &camera, .., None) + HudPainter\n\
         \x20            (what re2 uses; re2 adds viewer::FpsLayers for its weapon and crosshair)\n\n\
         Worked example, a separate crate with tests: examples/external/topdown_switch (`cargo test` there; `cargo run` plays).\n\
         NOT YET: app-defined actions into rules (a `use` key), an online custom-client helper (drive net::session::NetSession yourself),\n\
         orthographic cameras, adding objects after the renderer is built. ADR 0043.\n",
    )
}

fn sim_text() -> String {
    String::from(
        "red_engine2 sim <scene> [--scenario file.json] [--only name] [--trace out.json] [--checkpoint-every 1] [--dump-every 60]\n\
         \x20 Plays scripted players through the real authoritative simulation (no window, no GPU, no socket) and checks what happened.\n\
         \x20 Scenarios live in the scene's `checks.sim` (so `verify` runs them) or in a file. Exit 1 if any fails.\n\
         \x20 `bots.fill`/`roster` are NOT simulated here — only the scripted `players` below run. To prove a bot's behavior (a monster,\n\
         \x20 an AI opponent), use `playtest` (hosts a real match with bots) or a real hosted match, not a `checks.sim` scenario.\n\n\
         scenario = { name, players, script, expect, spawn_group?, max_seconds? (30), settle_seconds? (0.5) }\n\
         \x20 players  [{id, character: human|rat|wizard|cowboy|alien|robot, spawn?: spawn id}]\n\
         \x20 script   [{player, walk: \"x,z; x,z\" | wait: secs | hold: {forward, strafe, sprint, crouch, jump, yaw_deg, pitch_deg,\n\
         \x20          look_at: [x,y,z], interact, attack, reload, switch, seconds}, until_event?: name}]\n\
         \x20          each player's steps run in order; players run in parallel; a walk that gets stuck fails the scenario.\n\
         \x20          `walk` steers straight at each waypoint (forward only, no strafing) and counts it reached within 0.25 m: put waypoints\n\
         \x20          at the foot and head of stairs, not beside a flank. `look_at` aims yaw and pitch at a world point from the eye every\n\
         \x20          tick (a prop's origin plus about half its height), instead of hand-computed yaw_deg/pitch_deg.\n\
         \x20          Pick up / drop = `hold: {look_at: [x,y,z], interact: true, seconds: 0.2}` at the prop (reach 2.3 m; a `hold` of ~0.15 s\n\
         \x20          with the same aim first sets the view). Better, by object id (top-level; the playtest script has the same three steps):\n\
         \x20          {player, approach: \"id\", within?: m, timeout?: s} done within `within` (default 60% of pickup reach) of its footprint, fails\n\
         \x20          if no closer for 2 s or after `timeout` (12 s); {player, look_at: \"id\"} faces its middle; {player, interact: \"id\"} approaches,\n\
         \x20          faces, presses E and fails with the pick-up reason if a loose prop is not then carried (or if hands are full). Straight-line\n\
         \x20          steering like `walk`: `walk` round a wall first. A bat swing = `hold: {attack: true, seconds: 0.3}` (strike lands after the windup).\n\
         \x20 expect   [{event: name, count|min|max} {no_event: name} {var: name, eq|ne: number|bool, gt|gte|lt|lte: number}\n\
         \x20           {ended: outcome} {not_ended: true} {hidden|shown: object id} {collision_disabled|collision_enabled: object id}\n\
         \x20           {player: id, near: [x,z], tol?, y?}\n\
         \x20           {prop: id, in_zone|not_in_zone: zone | below_y|y_lt|y_gt: m | tilt_gt|tilt_lt: deg | moved: bool | near: [x,z], tol?, y? | held_by: player|none}]\n\
         The run ends when a rule `end`s the match, when every script is done (+ settle), or at max_seconds. The report lists where\n\
         every player ended and every loose prop that moved (`--json`: all props, with tilt / moved / asleep / held_by).\n\n\
         red_engine2 replay <trace.json> [--scene map.json] [--against other.json]\n\
         \x20 A trace records a match: header (engine, tick rate, map hash, seed, platform), every join/leave/input and external push in order (a strike or a rule impulse is re-derived, not recorded), game events,\n\
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

/// The first sentence of a command's `about`, cut at a word boundary to `max` characters (an abbreviation such as `e.g.` does not end it). The overview lists every
/// command on a line of its own; the whole text is `describe commands` / `search <name>`.
pub fn first_sentence(about: &str, max: usize) -> String {
    let mut end = about.len();
    for (i, _) in about.match_indices(". ") {
        let last_word = about[..i].rsplit(' ').next().unwrap_or("");
        if ["e.g", "i.e", "incl", "etc", "vs"].contains(&last_word) {
            continue;
        }
        end = i;
        break;
    }
    let s = about[..end].trim_end_matches('.').trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{}...", cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head).trim_end_matches([',', ':', ';', '(']))
}

fn commands_text(commands: &Value, brief: bool) -> String {
    let mut out = String::new();
    for c in commands.as_array().into_iter().flatten() {
        let name = c["name"].as_str().unwrap_or("?");
        let about = c["about"].as_str().unwrap_or("");
        if brief {
            out.push_str(&format!("  {name:<11} {}\n", first_sentence(about, 96)));
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
                "when": ["enter", "exit", "event", "every", "after", "start", "prop_enter", "prop_exit", "prop_below"],
                "functions": crate::sim::rules_expr::Func::ALL.iter().map(|(n, _, h)| json!({"function": n, "help": h})).collect::<Vec<_>>(),
                "engine_events": crate::sim::rules::ENGINE_EVENTS,
                "volumes": ["zone", "object", "box"],
                "builtin_vars": crate::sim::rules::BUILTIN_VARS,
                "example": serde_json::from_str::<Value>(RULES_EXAMPLE).unwrap_or(Value::Null),
            }),
            "sim" => json!({"text": sim_text()}),
            "multiplayer" => json!({"text": multiplayer_text()}),
            "playtest" => json!({"text": playtest_text()}),
            "custom-client" => json!({"text": custom_client_text()}),
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
        "playtest" => out.push_str(&playtest_text()),
        "custom-client" => out.push_str(&custom_client_text()),
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
    #[test]
    fn the_sim_and_rules_topics_list_every_hold_key_trigger_and_built_in() {
        let sim = super::sim_text();
        for key in ["pitch_deg", "look_at", "interact", "attack", "reload", "switch", "held_by", "tilt_gt"] {
            assert!(sim.contains(key), "describe sim must mention `{key}`");
        }
        let rules = super::rules_text();
        for key in ["prop_enter", "prop_below", "props_in(zone)", "reset", "place", "swing", "prop_hit", "starts with `_`"] {
            assert!(rules.contains(key), "describe rules must mention `{key}`");
        }
    }

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

    #[test]
    fn a_command_line_is_its_first_sentence_capped_at_a_word() {
        assert_eq!(first_sentence("Render the full scene to an MP4. Needs ffmpeg.", 96), "Render the full scene to an MP4");
        assert_eq!(first_sentence("Search everything, e.g. docs and assets. More.", 96), "Search everything, e.g. docs and assets");
        assert_eq!(first_sentence("No full stop at all", 96), "No full stop at all");
        let long =
            "Static map checker: overlaps, floating props, stairs that lead nowhere, low ceilings, unprotected drops, perimeter leaks, unreachable rooms";
        let cut = first_sentence(long, 60);
        assert!(cut.ends_with("...") && cut.len() <= 63 && long.starts_with(cut.trim_end_matches("...")), "{cut}");
        assert!(!cut.trim_end_matches("...").ends_with(' ') && !cut.contains("unpro"), "cut between words: {cut}");
    }
}
