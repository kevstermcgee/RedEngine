//! The 2D authoring reference: what `red_engine2 describe 2d` prints. One page that an AI reads instead of any source, kept true by the tests below (every key the parser
//! accepts must appear in it, every key it mentions must be real).

/// The reference text.
pub const REFERENCE: &str = r##"FILE: NAME.game2d.json (JSON; begin it with "game2d": 1). Positions are CENTRES in virtual-screen pixels; y grows downward.
KEYS: game2d id title description capabilities view sprites sounds music vars persist prefabs scene|map ui rules checks
capabilities: {"presentation":"2d","platforms":["web"],"networking":"offline","input":["keyboard","mouse"],"persistence":["settings","progress"]}
  declare what the game uses: buttons/click/pointer need mouse; keys/press need keyboard; `persist` needs progress; a `music` action needs settings.
  `red_engine2 capabilities` prints what is built; web is 2D-only and offline-only; touch and gamepad exist but are unverified; windows/linux 2D is prepared, not built.
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
rules: [{"id":"x","when":{TRIGGER},"if":"lives > 0","once":true,"cooldown":1,"do":[ACTIONS]}]
  TRIGGER (exactly one): start | every:seconds | after:seconds | touch:[tagA,tagB] (on first overlap) | touching:[a,b] (every tick) | press:action | click:tag ("*" = nothing) | event:name | end:"win"|"lose"|"any"
  ACTIONS: set:[var,value] add:[var,value] emit:name spawn:{prefab,at,vel,count} destroy:"self"|"other"|"tag:T"|"id:I" play:sound music:"on"|"off"|"toggle" burst:{at,n,color,speed,life,size,gravity}
    shake:px end:"win"|"lose" restart:true reset_save:true velocity:{target,v:[x,y]} teleport:{target,to}     `at`: "self"|"other"|"pointer"|[x,y]|{"x":[min,max],"y":[min,max]}
  values and `if` are expressions: numbers, variables, + - * / %, == != < <= > >=, && || ! (division by zero is 0). Rules run in order, each seeing the changes before it.
input actions: left right up down action secondary pause. Keys: arrows/WASD, Space/Z/J, Shift/X/K, Esc/P. Mouse/touch: `click` and buttons (a button `key` is any KeyboardEvent.code: Enter, KeyM, Digit1).
  A gamepad's stick/d-pad and A/B/Start map to the same actions.
checks: {"scenarios":[{"name":"...","seed":1,"max_seconds":30,"smoke":true,"script":[STEPS],"expect":[EXPECT]}],"browser":[{"name":"...","keys":["ArrowRight"],"click":[x,y],"ms":400,"changes":["p_x"],"persists":["music_on"]}]}
  STEPS: wait:s | hold:[actions],seconds:s | hold_until:[actions],until:EXPECT,timeout:s | press:action | click:[x,y] | button:id | point:[x,y] | approach:tag,seconds:s | wait_until:EXPECT,timeout:s
  EXPECT: {var,eq|ne|gt|gte|lt|lte:n} {ended:"win"|"lose"} {not_ended:true} {count:tag,eq..} {entity:id,near:[x,y],tol:px} {event:name,min,max} {sound:name,min} {hash:"..."}
  Every scenario needs an `expect` (one that asserts nothing proves nothing). `smoke:true` marks the playthrough the browser replays and must match hash for hash.
LOOP: validate G -> sim G [--only NAME --every SECONDS] -> verify G -> frame G out.png [--scenario NAME --t SECONDS --size 1280x720] (LOOK) -> web verify G -> publish G
PROVES: validate = well formed, names resolve. verify = simulation (scripted play, deterministic) + render (frames not blank) + audio waveform. web verify = a real headless browser:
  pixels equal native, scenarios replay hash for hash, real keys/clicks change state, saves survive reload and bad storage does not break play, audio starts after a gesture, console clean.
NOT PROVEN by any of it: that it is fun, that it sounds good, touch or gamepad, other browsers.
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
    fn the_reference_is_short_enough_to_read_in_one_go() {
        assert!(REFERENCE.len() < 9_000, "{} bytes: an AI reads this on every 2D task; trim it", REFERENCE.len());
    }
}
