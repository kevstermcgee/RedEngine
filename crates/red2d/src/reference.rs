//! The 2D authoring reference: what `red_engine2 describe 2d` prints. One page that an AI reads instead of any source, kept true by the tests below (every key the parser
//! accepts must appear in it, every key it mentions must be real).

/// The reference text.
pub const REFERENCE: &str = r##"FILE: NAME.game2d.json (JSON; begin it with "game2d": 1). Positions are CENTRES in virtual-screen pixels; y grows downward.
KEYS: game2d id title description capabilities view sprites sounds music vars persist prefabs scene|map ui effects rules checks   (3D parts, optional: models layers3d, see HYBRID)
capabilities: {"presentation":"2d","platforms":["windows","linux"],"networking":"offline","input":["keyboard","mouse"],"persistence":["settings","progress"]}
  declare what the game uses: buttons/click/pointer need mouse; keys/press need keyboard; `persist` needs progress; a `music` action needs settings.
  `red_engine2 capabilities` prints what is built; 2D games are offline, native (windows/linux) and read keyboard and mouse; a gamepad is prepared, not built.
view: {"width":320,"height":180,"background":"#1b2233","scale":"fit"|"integer","world":[640,180],"camera":{"follow":"player","lerp":0.15}}  (64..1280 px a side)
sprites: {"name":{"rows":["..aa..","aaaa"],"palette":{"a":"#ff0000"},"fps":8}} or "frames":[rows,...] for animation; "." is transparent; at most 64 px a side
sounds: {"name":{"seconds":0.2,"layers":[{"sine":880,"decay":18}]}} (a voice: `describe audio`, `audio list`)   music: {"name":{score}} (`describe audio`)
vars: {"score":0}   builtins: time tick ended(0, 1 win, 2 lose) mouse_x mouse_y music_on   plus count_<tag> and <sceneid>_x, <sceneid>_y   persist: ["best"] (saved between sessions)
prefabs: {"name":{"tag":"coin"|[tags],"shape":{"sprite":n|"rect":[w,h]|"circle":r|"text":"SCORE {score}","color":"#fc0","scale":1,"flip":"x"|"auto"},"size":[w,h],"layer":0,
  "body":{"type":"dynamic"|"static","gravity":600,"bounce":0.5,"friction":0.1,"drag":0,"max_fall":600},"collide":[tags it is stopped by],
  "move":{"keys":{"mode":"topdown"|"platformer","speed":90,"jump":250}} | {"pointer":"x"|"y"|"xy"} | {"chase":{"target":tag,"speed":30}} | {"drift":[vx,vy]} | {"wander":{"speed":20,"turn":1}} | {"patrol":{"axis":"x","range":20,"speed":40}},
  "ttl":seconds,"emit":{"rate":3,"life":[.3,.6],"speed":[4,10],"angle":[250,290],"color":"#fff"},"clamp":true,"hidden":true}}
scene: [{"prefab":"coin","at":[40,30],"id":"p","count":1}]   map: {"tile":16,"origin":[0,0],"rows":["#..#"],"legend":{"#":"wall"}} ("." is empty)
ui: {"text":"SCORE {score:3}","at":[4,4],"color":"#fff","scale":1,"align":"left"|"center"|"right","show":"ended"}  {"bar":{"var":"lives","max":3,"at":[0,0],"size":[40,6]}}
  {"panel":{"at":[0,0],"size":[100,20],"color":"#10141fdd"}}  {"button":{"id":"go","label":"GO","at":[0,0],"size":[40,12],"key":"Enter","do":[actions]}}   {v}=value {v:3}=zero-padded {v:.1}=one decimal
effects: {"heal":{"params":{"amount":1},"do":[{"add":["lives","$amount"]},{"play":"pick"}]}}  a named group of actions, used by any rule or button: {"apply":"heal","with":{"amount":2}}
  params: names (all required) or name -> default; "$name" is the argument; effects may apply effects (4 deep, no loops); an effect nothing applies is an error
rules: [{"id":"x","when":{TRIGGER},"if":"lives > 0","once":true,"cooldown":1,"do":[ACTIONS]}]
  TRIGGER (exactly one): start | every:seconds | after:seconds | touch:[tagA,tagB] (on first overlap) | touching:[a,b] (every tick) | press:action | click:tag ("*" = nothing) | event:name (built in: `loaded` fires once after saved progress is restored: rebuild the world from saved vars) | end:"win"|"lose"|"any"
  ACTIONS: set:[var,value] add:[var,value] emit:name spawn:{prefab,at,vel,count} destroy:"self"|"other"|"tag:T"|"id:I" play:sound music:"on"|"off"|"toggle" burst:{at,n,color,speed,life,size,gravity}
    shake:px end:"win"|"lose" restart:true reset_save:true velocity:{target,v:[x,y]} teleport:{target,to} apply:effect     `at`: "self"|"other"|"pointer"|[x,y]|{"x":[min,max],"y":[min,max]}
  values and `if` are expressions: numbers, variables, + - * / %, == != < <= > >=, && || ! (division by zero is 0). Rules run in order, each seeing the changes before it.
HYBRID (3D where it helps, same file, same sim, still runs natively and headless): "presentation":"hybrid", then any of `models` (named 3D objects), a prefab shape {"model":name}, a ui/layers3d
  {"view3d":{..}}, view.world3d (the whole world in perspective), {"minimap":{..}}. Choose 2D unless a 3D part adds something. `red_engine2 describe hybrid` has the format.
input actions: left right up down action secondary pause. Keys: arrows/WASD, Space/Z/J, Shift/X/K, Esc/P. Mouse: `click` and buttons (a button `key` is any KeyboardEvent.code: Enter, KeyM, Digit1).
  A gamepad's stick/d-pad and A/B/Start map to the same actions.
checks: {"scenarios":[{"name":"...","seed":1,"max_seconds":30,"smoke":true,"script":[STEPS],"expect":[EXPECT]}],
  "reach":[{"from":"p","to":"tag:goal","open":["gate"],"reachable":true,"why":"..."}]}   reach = map analysis with the game's own collision: can top-down walker `from` (scene id) touch `to` (id or tag:NAME), with `open` things assumed gone?
  STEPS: wait:s | hold:[actions],seconds:s | hold_until:[actions],until:EXPECT,timeout:s | press:action | click:[x,y] | button:id | point:[x,y] | approach:tag,seconds:s | wait_until:EXPECT,timeout:s
  EXPECT: {var,eq|ne|gt|gte|lt|lte:n} {ended:"win"|"lose"} {not_ended:true} {count:tag,eq..} {entity:id,near:[x,y],tol:px} {event:name,min,max} {sound:name,min} {hash:"..."} {reach:"tag:goal",from:"p",reachable:false} (live: from where it stands now)
  Every scenario needs an `expect` (one that asserts nothing proves nothing). `smoke:true` marks the game's main playthrough.
MECHANICS (verified, each with the scenarios that prove it; copy one instead of inventing it): `red_engine2 recipe` lists key-door timer-lose collect-then-exit health-damage checkpoint-respawn spawner-waves survive-then-escape; `recipe NAME --new g.game2d.json`.
LOOP: validate G -> sim G [--only NAME --every SECONDS] -> verify G -> frame G out.png [--scenario NAME --t SECONDS --size 1280x720] (LOOK) -> play2d G (a window, with sound)
PROVES: validate = well formed, names resolve. verify = simulation (scripted play, deterministic) + render (frames not blank) + audio waveform. play2d = the same simulation, picture and sound in a native window; progress is kept between runs.
NOT PROVEN by any of it: that it is fun, that it sounds good, or a gamepad.
"##;

/// The hybrid (2D + software 3D) reference: what `red_engine2 describe hybrid` prints.
pub const HYBRID: &str = r##"HYBRID GAMES: a 2D game (`describe 2d`) that draws some of itself in 3D. One file, one simulation, one input model; 3D only changes how things are DRAWN. Declare "presentation":"hybrid".
  It is a small software renderer (flat or toon shading, one light, depth buffer, no GPU), identical in a window and headless, so every target keeps it. Not the wgpu engine: pick a
  3D game (`describe`, not this) for real lighting, textures and big worlds. The AI chooses per game; use none of this when plain 2D reads better.
models: {"boss":{"parts":[{"shape":"box","size":[2,2,2],"color":"#a33","at":[0,0,0],"rot":[yaw,pitch,roll]}, {"shape":"sphere","radius":1,"segments":16}, {"shape":"voxels","sprite":"hero","depth":3,"cell":0.1}]}}
  shapes: box(size) sphere(radius) cylinder(radius,height) cone(radius,height) pyramid(base,height) torus(major,minor) plane(size [x,z]) voxels(a sprite pressed into 3D). +Y is up; 6000 triangles a model, 30000 in all.
  angles are degrees, a number or an expression ("time * 60").
IN THE WORLD (a model as a thing): prefab {"shape":{"model":"boss","yaw":"time*40","pitch":0,"roll":0,"scale":1,"elevation":25,"tint":"#ffffff","light":{"toon":true,"dir":[-.4,-.8,-.4],"ambient":.4},"fit":[64,64]}}
  drawn into its box (`fit`, also the collision size) from a fixed camera looking down `elevation` degrees: a boss, a pickup, a ship in an otherwise flat game. Collisions, rules and tags work as for any prefab.
VIEWPORT (a 3D scene in a rectangle): ui {"view3d":{"at":[4,4],"size":[40,40],"camera":{"eye":[0,2,5],"target":[0,1,0],"fov":45|"ortho":3},"background":"#00000000","items":[{"model":"boss","at":[0,0,0],"yaw":"time*30","scale":1}]}}
  the same object is a world layer: top-level "layers3d":[{"layer":-5,"view3d":{..}}] draws between entities by `layer` (below 0: behind most things; `at`/`size` default to the whole screen).
WORLD VIEW (everything in perspective): view {"world3d":{"plane":"ground"|"wall","pitch":55,"distance":150,"yaw":"time*5","fov":50,"ground":{"color":"#223","alt":"#334","tile":32},"light":{..},"sky":"#14182c"}}
  "ground": 2D y becomes depth (top-down, arena, maze); "wall": 2D y becomes height (side-scroller seen from the front). The camera follows `view.camera` as it does in 2D. Things become: a flat picture that
  faces the camera (default), a box on their footprint when the prefab has "height3d": N (thickness on a wall), a model, or projected text. UI stays flat on top; clicks and `at:"pointer"` are unprojected to the plane.
MINIMAP (a flat map over any game): ui {"minimap":{"at":[236,4],"size":[80,60],"colors":{"player":"#fff","enemy":"#f44"},"dot":3,"viewport":true,"background":"#0a0e18c8","border":"#5a688c"}}
  a dot per living thing with a listed tag (first match decides the colour), scaled from view.world; `viewport` outlines what the camera shows.
CHECK: `validate` names every mistake (unknown shape, missing model, equal eye and target); `frame G out.png` shows it.
"##;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game;

    #[test]
    fn every_key_the_parser_accepts_is_in_the_reference() {
        let lists: [(&str, &[&str]); 6] = [
            ("root", game::ROOT),
            ("trigger", game::TRIGGERS),
            ("action", game::ACT_KEYS),
            ("mover", game::MOVERS),
            ("step", game::STEP_KEYS),
            ("expectation", game::EXPECT_KEYS),
        ];
        for (what, keys) in lists {
            for k in keys {
                assert!(REFERENCE.contains(k), "the 2D reference does not mention the {what} `{k}`");
            }
        }
        for a in game::ACTIONS {
            assert!(REFERENCE.contains(a), "the 2D reference does not mention the input action `{a}`");
        }
        for b in game::BUILTINS {
            assert!(REFERENCE.contains(b), "the 2D reference does not mention the built-in variable `{b}`");
        }
    }

    #[test]
    fn the_hybrid_reference_names_every_3d_key_and_stays_short() {
        for k in ["models", "layers3d", "world3d", "view3d", "minimap", "height3d", "ortho", "elevation", "fit", "viewport"] {
            assert!(HYBRID.contains(k), "the hybrid reference does not mention `{k}`");
        }
        assert!(HYBRID.len() < 5_500, "{} bytes: trim it", HYBRID.len());
    }

    #[test]
    fn the_reference_is_short_enough_to_read_in_one_go() {
        assert!(REFERENCE.len() < 9_000, "{} bytes: an AI reads this on every 2D task; trim it", REFERENCE.len());
    }
}
