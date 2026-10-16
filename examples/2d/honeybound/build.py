#!/usr/bin/env python3
"""Honeybound builder: writes ../honeybound.game2d.json (a RedEngine 2D game) from art.py, cast.py, bosses.py, audio.py, levels.py, gen.py and story.py.

  python3 build.py          write the game
  python3 build.py stats    sizes: levels, entities, rules

Pip, the smallest bee of the Great Hive, walks (Pip cannot fly yet) through 32 stages in eight worlds to win back the eight shards of the Heartcomb
from Baron Thornwick and his seven lieutenants, before the first snow freezes the hive. Everything that happens is data: prefabs, a tile map per level and
rules (`red_engine2 describe 2d`). The flow is one variable, `mode`:

  0 title   1 the card before a stage   2 playing   3 a life lost   4 the Bloom Gate reached   5 game over   6 a boss beaten (a friend speaks)
  7 the end   8 the prologue

The pause menu is `menu` (the game's `pause_if` stands the world still while it is open); the bosses are `bs` (their state), `bt` (when it began),
`bhp`/`bmax`, `bkind` (which boss: the world) and `den` (0 not yet in the boss's den, 1 fighting, 2 beaten).
"""
import json
import sys
from pathlib import Path

import art
import audio
import gen
import levels
import story
from levels import T, cx, cy

def hive_look():
    """The inside of the hive (the prologue) and the ending's snowy meadow: two more looks for the backdrop and tiles."""
    art.BACKS.setdefault("hive_inside", ("#7a3a08", "#ffb84a", "#e09b30", "#c8862a", "#b0701e", "#9a6018", None, None, "flowers"))
    art.BACKS.setdefault("ending", ("#8ab0d8", "#f8e8d8", "#e8f0f8", "#d0dcea", "#a8c0d8", "#98b0c8", "#ffffff", "#e8eef8", "pines"))
    art.THEMES.setdefault("hive_inside", art.THEMES["meadow"] | {"moss": "#e0952a", "moss2": "#b06a1a", "tip": "#ffd06a", "soil": "#8a4a14", "soil2": "#5a2e0a", "soil3": "#a8601e"})
    art.THEMES.setdefault("ending", art.THEMES["frost"])


hive_look()
HERE = Path(__file__).resolve().parent
OUT = HERE.parent / "honeybound.game2d.json"
SOLUTIONS = HERE / "solutions.json"

VIEW_W, VIEW_H = 480, 270
TOP = 30  # the play field's grid starts this far down: a band of sky for the HUD
FLOOR_DEN = TOP + 12 * T  # the floor of every boss's den
STAGE_LEVELS = [f() for f in levels.STAGES] + [L for w in range(3, 9) for L in gen.world_stages(w)]
BONUS_LEVELS = [levels.BONUS[s.id]() for s in STAGE_LEVELS if s.id in levels.BONUS]
SPECIAL = ["title", "card", "hive", "gameover"]
LEVEL_IDS = SPECIAL + [s.id for s in STAGE_LEVELS] + [b.id for b in BONUS_LEVELS] + ["ending"]
IX = {lid: i for i, lid in enumerate(LEVEL_IDS)}
FIRST = len(SPECIAL)  # level index of stage 0
KEEPS = [i for i, s in enumerate(STAGE_LEVELS) if s.theme == "keep"]
N_STAGES = len(STAGE_LEVELS)

# ---- how Pip moves ---------------------------------------------------------------------------------------------------------------------------------
# Snappy on the ground (quick to start, quick to stop), a variable jump, generous coyote time and jump buffering, and the flutter: holding jump while
# falling slows the fall for most of a second. Pip's wings are small, but they count.
FEEL = {"mode": "platformer", "speed": 92, "run": 156, "jump": 370, "run_jump": 40, "accel": 560, "decel": 760, "cut": 0.42, "coyote": 0.1, "buffer": 0.14,
        "glide": 46, "glide_time": 0.85}
BODY = {"type": "dynamic", "gravity": 1000, "max_fall": 380}
SMALL, BIG = [11, 14], [12, 22]
PLAY = "mode == 2 && menu == 0"

BOSSES = ["grubmaw", "duskwing", "burr", "rime", "sandmaw", "gloam", "vespa", "baron"]  # bkind 1..8


def hero(sprite, size, tags, idle=True, air=True):
    shape = {"sprite": sprite, "flip": "auto"}
    if idle:
        shape["idle"] = sprite + "_idle"
    if air:
        shape["air"] = sprite + "_air"
    return {"tag": tags, "shape": shape, "size": size, "layer": 5, "body": BODY, "collide": ["solid"], "move": {"keys": FEEL}}


def boss_prefab(sprite, size, role, mv=None, gravity=True, collide=True, flip="auto"):
    """One pose of a boss. role: hazard (touching it hurts), stomp (landing on it hits it, its sides hurt), weak (landing on it hits it, harmless), safe."""
    tags = {"hazard": ["boss", "hazard"], "stomp": ["boss", "bstomp"], "weak": ["boss", "bweak"], "safe": ["boss"]}[role]
    p = {"tag": tags, "shape": {"sprite": sprite, "flip": flip}, "size": size, "layer": 4}
    p["body"] = {"type": "dynamic", "gravity": 1000, "max_fall": 420} if gravity else {"type": "dynamic"}
    if collide:
        p["collide"] = ["solid"]
    if mv:
        p["move"] = mv
    return p


def prefabs():
    P = {}
    # Pip in every form. "pip" is everything the camera follows; "hero" is Pip while Pip can act; "vuln" can be hurt.
    P["pip_s"] = hero("pip", SMALL, ["pip", "hero", "vuln"])
    P["pip_b"] = hero("pipbig", BIG, ["pip", "hero", "vuln"])
    P["pip_f"] = hero("pollen", BIG, ["pip", "hero", "vuln"])
    P["pip_s_star"] = hero("pip_star", SMALL, ["pip", "hero", "star"], idle=False, air=False)
    P["pip_b_star"] = hero("pipbig_star", BIG, ["pip", "hero", "star"], idle=False, air=False)
    P["pip_f_star"] = hero("pipbig_star", BIG, ["pip", "hero", "star"], idle=False, air=False)
    P["pip_s_blink"] = hero("pip_blink", SMALL, ["pip", "hero"], idle=False, air=False)
    walk = {"walk": {"speed": 74, "dir": "right"}}
    for k, (spr, size) in {"s": ("pip", SMALL), "b": ("pipbig", BIG), "f": ("pollen", BIG)}.items():
        P[f"walk_{k}"] = {"tag": ["pip", "walker"], "shape": {"sprite": spr, "flip": "auto", "air": spr + "_air"}, "size": size, "layer": 5, "body": BODY, "collide": ["solid"], "move": walk}
    P["pip_dead"] = {"tag": "ghost", "shape": {"sprite": "pip_dazed"}, "size": SMALL, "layer": 7, "body": {"type": "dynamic", "gravity": 900, "max_fall": 400}}
    P["seed"] = {"tag": "seed", "shape": {"sprite": "seed"}, "size": [8, 8], "layer": 6, "body": {"type": "dynamic", "gravity": 1000, "bounce": 1.0, "max_fall": 300}, "collide": ["solid"], "ttl": 2.5}

    # the Baron's creatures: each kind has its own tag (rules say what stomping, kicking or burning it does)
    def foe(kind, sprite, size, tags, mv, body=True, layer=3, asleep=True):
        p = {"tag": [kind] + tags, "shape": {"sprite": sprite, "flip": "auto"}, "size": size, "layer": layer, "move": mv, "asleep": asleep}
        if body:
            p["body"] = {"type": "dynamic", "gravity": 1000, "max_fall": 360}
            p["collide"] = ["solid", "foe"]
        else:
            p["body"] = {"type": "dynamic"}
        return p

    P["grubble"] = foe("grubble", "grubble", [14, 13], ["foe", "enemy", "stompable"], {"walk": {"speed": 28}})
    P["dgrubble"] = foe("dgrubble", "dgrubble", [14, 13], ["foe", "enemy", "stompable"], {"walk": {"speed": 36}})
    P["snail"] = foe("snail", "snail", [14, 13], ["foe", "enemy", "stompable"], {"walk": {"speed": 22, "edges": "turn"}})
    P["spikebug"] = foe("spikebug", "spikebug", [14, 12], ["foe", "enemy", "spiky"], {"walk": {"speed": 26}})
    P["wasp"] = foe("wasp", "wasp", [14, 12], ["enemy", "stompable"], {"patrol": {"axis": "y", "range": 28, "speed": 30}}, body=False)
    P["waspx"] = foe("wasp", "wasp", [14, 12], ["enemy", "stompable"], {"patrol": {"axis": "x", "range": 40, "speed": 34}}, body=False)
    P["snapvine"] = foe("snapvine", "snapvine", [12, 22], ["enemy", "spiky"], {"patrol": {"axis": "y", "range": 12, "speed": 14}}, body=False, layer=1)
    P["shell"] = {"tag": ["shell"], "shape": {"sprite": "shell"}, "size": [14, 12], "layer": 3, "body": {"type": "dynamic", "gravity": 1000, "max_fall": 360}, "collide": ["solid"]}
    P["kshell"] = {"tag": ["kshell", "foe"], "shape": {"sprite": "shell_spin"}, "size": [14, 12], "layer": 3, "body": {"type": "dynamic", "gravity": 1000, "max_fall": 360}, "collide": ["solid"], "move": {"walk": {"speed": 230}}}
    for kind in ("grubble", "dgrubble"):
        P[f"{kind}_flat"] = {"tag": "corpse", "shape": {"sprite": f"{kind}_flat"}, "size": [14, 14], "layer": 2, "ttl": 0.6}
    for kind, sprite in (("grubble", "grubble_dead"), ("dgrubble", "dgrubble_dead"), ("snail", "snail_dead"), ("spikebug", "spikebug_dead"), ("wasp", "wasp_dead")):
        P[f"{kind}_dead"] = {"tag": "corpse", "shape": {"sprite": sprite}, "size": [14, 14], "layer": 6, "body": {"type": "dynamic", "gravity": 900, "max_fall": 400}, "ttl": 2.5}
    P["snapvine_dead"] = {"tag": "corpse", "shape": {"sprite": "snapvine"}, "size": [12, 22], "layer": 6, "body": {"type": "dynamic", "gravity": 900, "max_fall": 400}, "ttl": 1.5}
    P["cinder"] = {"tag": ["hazard"], "shape": {"sprite": "cinder"}, "size": [10, 10], "layer": 4, "move": {"patrol": {"axis": "y", "range": 64, "speed": 90}}, "body": {"type": "dynamic"}, "asleep": True}
    P["cinder_hi"] = {"tag": ["hazard"], "shape": {"sprite": "cinder"}, "size": [10, 10], "layer": 4, "move": {"patrol": {"axis": "y", "range": 104, "speed": 120}}, "body": {"type": "dynamic"}, "asleep": True}

    # items
    P["honey"] = {"tag": "honey", "shape": {"sprite": "honey"}, "size": [10, 12], "layer": 2}
    P["coinpop"] = {"tag": "fx", "shape": {"sprite": "honey_spin"}, "size": [10, 12], "layer": 6, "body": {"type": "dynamic", "gravity": 900}, "ttl": 0.45}
    P["jelly"] = {"tag": ["jelly", "item"], "shape": {"sprite": "jelly"}, "size": [14, 14], "layer": 2, "body": BODY, "collide": ["solid"], "move": {"walk": {"speed": 50, "dir": "right"}}}
    P["firebloom"] = {"tag": ["firebloom", "item"], "shape": {"sprite": "firebloom"}, "size": [14, 14], "layer": 2}
    P["starpetal"] = {"tag": ["starpetal", "item"], "shape": {"sprite": "starpetal"}, "size": [14, 14], "layer": 2, "body": BODY, "collide": ["solid"], "move": {"walk": {"speed": 64, "dir": "right"}}}
    P["clover"] = {"tag": ["clover", "item"], "shape": {"sprite": "clover"}, "size": [14, 14], "layer": 2, "body": BODY, "collide": ["solid"], "move": {"walk": {"speed": 50, "dir": "right"}}}
    P["shard"] = {"tag": ["shard"], "shape": {"sprite": "shard"}, "size": [16, 16], "layer": 6, "body": {"type": "dynamic", "gravity": 500, "max_fall": 120}, "collide": ["solid"],
                  "emit": {"rate": 10, "life": [0.4, 0.9], "speed": [6, 18], "angle": [200, 340], "color": "#fff27a", "size": 1}}
    P["bumper"] = {"tag": "bumper", "shape": {"rect": [14, 8], "color": "#ffffff"}, "size": [14, 8], "hidden": True, "ttl": 0.1}
    P["pop1UP"] = {"tag": "fx", "shape": {"text": "1UP", "color": "#9dff5c"}, "layer": 8, "move": {"drift": [0, -36]}, "body": {"type": "dynamic"}, "ttl": 0.8}

    # world pieces, per theme
    for theme in art.THEMES:
        P[f"{theme}_top"] = {"tag": "solid", "shape": {"sprite": f"{theme}_top"}, "body": {"type": "static"}}
        P[f"{theme}_soil"] = {"tag": "solid", "shape": {"sprite": f"{theme}_soil"}, "body": {"type": "static"}}
        P[f"{theme}_brick"] = {"tag": ["solid", "brick"], "shape": {"sprite": f"{theme}_brick"}, "body": {"type": "static"}}
        P[f"{theme}_stone"] = {"tag": "solid", "shape": {"sprite": f"{theme}_stone"}, "body": {"type": "static"}}
        P[f"{theme}_starbrick"] = {"tag": ["solid", "starblock"], "shape": {"sprite": f"{theme}_brick"}, "body": {"type": "static"}}
    P["combcoin"] = {"tag": ["solid", "coinblock"], "shape": {"sprite": "comb"}, "body": {"type": "static"}}
    P["combpower"] = {"tag": ["solid", "powerblock"], "shape": {"sprite": "comb"}, "body": {"type": "static"}}
    P["comblife"] = {"tag": ["solid", "lifeblock"], "shape": {"sprite": "comb"}, "body": {"type": "static"}}
    P["used"] = {"tag": ["solid", "spent"], "shape": {"sprite": "used"}, "body": {"type": "static"}}
    P["hard"] = {"tag": ["solid", "spent"], "shape": {"sprite": "hard"}, "body": {"type": "static"}}
    for part in ("tl", "tr", "bl", "br"):
        P[f"log_{part}"] = {"tag": ["solid", "spent"], "shape": {"sprite": f"log_{part}"}, "layer": 2, "body": {"type": "static"}}
    P["leaf_l"] = {"tag": "solid", "shape": {"sprite": "leaf_l"}, "body": {"type": "static"}}
    P["leaf_m"] = {"tag": "solid", "shape": {"sprite": "leaf_m"}, "body": {"type": "static"}}
    P["leaf_r"] = {"tag": "solid", "shape": {"sprite": "leaf_r"}, "body": {"type": "static"}}
    P["trunk"] = {"tag": "decor", "shape": {"sprite": "trunk"}, "layer": -2}
    P["lava"] = {"tag": ["lava", "abyss"], "shape": {"sprite": "lava"}, "size": [16, 12], "layer": 4}
    P["lava_deep"] = {"tag": "decor", "shape": {"sprite": "lava_deep"}, "layer": 4}
    P["pit_shade"] = {"tag": "decor", "shape": {"sprite": "pit_shade"}, "layer": -1}
    P["edge"] = {"tag": "solid", "shape": {"rect": [16, 16], "color": "#000000"}, "hidden": True, "body": {"type": "static"}}
    P["abyss"] = {"tag": "abyss", "shape": {"rect": [16, 16], "color": "#000000"}, "hidden": True}
    P["bloom"] = {"tag": "goal", "shape": {"sprite": "bloom"}, "size": [20, 64], "layer": 1}
    P["bloom_open"] = {"tag": "decor", "shape": {"sprite": "bloom_open"}, "size": [20, 64], "layer": 1,
                       "emit": {"rate": 14, "life": [0.6, 1.4], "speed": [10, 30], "angle": [200, 340], "color": "#fff27a", "size": 1}}
    P["midpoint"] = {"tag": "midpoint", "shape": {"rect": [8, 8], "color": "#ffffff"}, "size": [8, 480], "hidden": True}
    P["logmouth"] = {"tag": "logmouth", "shape": {"rect": [10, 10], "color": "#ffffff"}, "size": [10, 10], "hidden": True}
    P["logside"] = {"tag": "logside", "shape": {"sprite": "log_side"}, "size": [16, 16], "layer": 2}
    for d in ("flowers", "mushroom", "pebbles", "torch", "crystal", "stars") + tuple(n for n in art.decor() if n.startswith("tuft")):
        P[d] = {"tag": "decor", "shape": {"sprite": d}, "layer": -1}
    # the hive, the road, the prisoners
    P["queen"] = {"tag": "npc", "shape": {"sprite": "queen"}, "size": [20, 24], "layer": 3}
    P["pip_pose"] = {"tag": "npc", "shape": {"sprite": "pip_idle"}, "size": SMALL, "layer": 3}
    P["pip_flying"] = {"tag": "npc", "shape": {"sprite": "pipbig_air", "flip": "auto"}, "size": BIG, "layer": 6, "move": {"patrol": {"axis": "x", "range": 150, "speed": 60}}, "body": {"type": "dynamic"}}
    for k in range(1, 8):
        P[f"friend{k}"] = {"tag": "npc", "shape": {"sprite": f"friend{k}"}, "size": [16, 18], "layer": 3}
        P[f"friend{k}_fly"] = {"tag": "npc", "shape": {"sprite": f"friend{k}", "flip": "auto"}, "size": [16, 18], "layer": 3,
                               "move": {"orbit": {"radius": 18 + 4 * k, "speed": 40 + 6 * k, "phase": 50 * k}}}
    P["cocoon"] = {"tag": ["cocoon"], "shape": {"sprite": "cocoon"}, "size": [16, 20], "layer": 3}
    for folk in ("bramwell", "ladybug", "cricket", "caterpillar", "firefly"):
        P[folk] = {"tag": "folk", "shape": {"sprite": folk}, "size": [16, 18], "layer": 3}
    P["heartcomb"] = {"tag": "npc", "shape": {"sprite": "heartcomb"}, "size": [40, 40], "layer": 3, "emit": {"rate": 8, "life": [0.6, 1.2], "speed": [6, 16], "angle": [0, 360], "color": "#fff27a", "size": 1}}
    P["snowfall"] = {"tag": "decor", "shape": {"rect": [2, 2], "color": "#ffffff"}, "hidden": True,
                     "emit": {"rate": 6, "life": [3, 5], "speed": [14, 24], "angle": [80, 110], "color": "#ffffffcc", "size": 1}}

    # bosses: their dens, their poses, their projectiles
    P["densensor"] = {"tag": "densensor", "shape": {"rect": [16, 16], "color": "#ffffff"}, "size": [16, 300], "hidden": True}
    P["mark"] = {"tag": "mark", "shape": {"rect": [4, 4], "color": "#ffffff"}, "size": [4, 4], "hidden": True}
    P["gate"] = {"tag": ["solid", "gate"], "shape": {"sprite": "gate_tall"}, "size": [16, 48], "layer": 3, "body": {"type": "static"}}
    walkto = lambda v: {"walk": {"speed": v}}
    P["grubmaw_wait"] = boss_prefab("grubmaw", [30, 22], "stomp", flip="auto")
    P["grubmaw_walk"] = boss_prefab("grubmaw", [30, 22], "stomp", walkto(34))
    P["grubmaw_fast"] = boss_prefab("grubmaw", [30, 22], "stomp", walkto(52))
    P["grubmaw_rage"] = boss_prefab("grubmaw", [30, 22], "stomp", walkto(70))
    P["grubmaw_hurt"] = boss_prefab("grubmaw_hurt", [30, 22], "safe")
    P["grubmaw_dead"] = boss_prefab("grubmaw_dead", [30, 22], "safe", collide=False) | {"ttl": 3}
    P["duskwing_wait"] = boss_prefab("duskwing", [26, 18], "hazard", gravity=False, collide=False)
    P["duskwing_fly"] = boss_prefab("duskwing", [26, 18], "hazard", {"patrol": {"axis": "x", "range": 120, "speed": 70}}, gravity=False, collide=False)
    P["duskwing_dive"] = boss_prefab("duskwing_dive", [20, 20], "hazard", gravity=False)
    P["duskwing_rise"] = boss_prefab("duskwing", [26, 18], "hazard", gravity=False, collide=False)
    P["duskwing_rest"] = boss_prefab("duskwing_rest", [30, 12], "weak")
    P["duskwing_hurt"] = boss_prefab("duskwing_hurt", [30, 12], "safe")
    P["duskwing_dead"] = boss_prefab("duskwing_dead", [26, 18], "safe", collide=False) | {"ttl": 3}
    P["dust"] = {"tag": ["hazard", "bproj"], "shape": {"sprite": "dust"}, "size": [6, 6], "layer": 5, "move": {"drift": [0, 52]}, "body": {"type": "dynamic"}, "ttl": 4}
    P["burr_wait"] = boss_prefab("burr", [20, 18], "hazard")
    P["burr_walk"] = boss_prefab("burr", [20, 18], "hazard", walkto(30))
    P["burr_ball"] = boss_prefab("burr_ball", [20, 20], "hazard", walkto(150))
    P["burr_ball_fast"] = boss_prefab("burr_ball", [20, 20], "hazard", walkto(195))
    P["burr_dizzy"] = boss_prefab("burr_dizzy", [20, 18], "weak")
    P["burr_hurt"] = boss_prefab("burr_hurt", [20, 18], "safe")
    P["burr_dead"] = boss_prefab("burr_dead", [20, 18], "safe", collide=False) | {"ttl": 3}
    P["rime_wait"] = boss_prefab("rime", [28, 16], "hazard", gravity=False, collide=False)
    P["rime_hang"] = boss_prefab("rime", [28, 16], "hazard", {"patrol": {"axis": "x", "range": 120, "speed": 50}}, gravity=False, collide=False)
    P["rime_aim"] = boss_prefab("rime", [28, 16], "hazard", gravity=False, collide=False)
    P["rime_drop"] = boss_prefab("rime", [28, 16], "hazard")
    P["rime_ground"] = boss_prefab("rime_ground", [28, 16], "weak")
    P["rime_climb"] = boss_prefab("rime", [28, 16], "hazard", gravity=False, collide=False)
    P["rime_hurt"] = boss_prefab("rime_hurt", [28, 16], "safe")
    P["rime_dead"] = boss_prefab("rime_dead", [28, 16], "safe", collide=False) | {"ttl": 3}
    P["icicle"] = {"tag": ["hazard", "bproj", "icicle"], "shape": {"sprite": "icicle"}, "size": [6, 12], "layer": 5, "body": {"type": "dynamic", "gravity": 700, "max_fall": 360}, "collide": ["solid"], "ttl": 3}
    P["sandmaw_wait"] = boss_prefab("sand_mound", [30, 10], "safe")
    P["sand_mound"] = boss_prefab("sand_mound", [30, 10], "safe", walkto(44))
    P["sand_rumble"] = boss_prefab("sand_mound", [30, 10], "safe")
    P["sandmaw_up"] = boss_prefab("sandmaw", [26, 20], "stomp")
    P["sandmaw_hurt"] = boss_prefab("sandmaw_hurt", [26, 20], "safe")
    P["sandmaw_dead"] = boss_prefab("sandmaw_dead", [26, 20], "safe", collide=False) | {"ttl": 3}
    P["clod"] = {"tag": ["hazard", "bproj"], "shape": {"sprite": "clod"}, "size": [7, 7], "layer": 5, "body": {"type": "dynamic", "gravity": 700, "max_fall": 400}, "ttl": 3}
    P["gloam_wait"] = boss_prefab("gloam", [18, 16], "hazard", gravity=False, collide=False)
    P["gloam_hover"] = boss_prefab("gloam", [18, 16], "hazard", gravity=False, collide=False)
    P["gloam_charge"] = boss_prefab("gloam_charge", [18, 16], "hazard", gravity=False, collide=False)
    P["gloam_drowsy"] = boss_prefab("gloam_drowsy", [18, 16], "weak") | {"body": {"type": "dynamic", "gravity": 400, "max_fall": 90}}
    P["gloam_hurt"] = boss_prefab("gloam_hurt", [18, 16], "safe")
    P["gloam_dead"] = boss_prefab("gloam_dead", [18, 16], "safe", collide=False) | {"ttl": 3}
    P["spark"] = {"tag": ["hazard", "bproj"], "shape": {"sprite": "spark"}, "size": [5, 5], "layer": 5, "body": {"type": "dynamic"}, "ttl": 3,
                  "emit": {"rate": 20, "life": [0.15, 0.3], "speed": [2, 6], "angle": [0, 360], "color": "#fff27a", "size": 1}}
    P["vespa_wait"] = boss_prefab("vespa", [28, 22], "hazard", gravity=False, collide=False)
    P["vespa_hover"] = boss_prefab("vespa", [28, 22], "hazard", {"patrol": {"axis": "x", "range": 120, "speed": 72}}, gravity=False, collide=False)
    P["vespa_aim"] = boss_prefab("vespa", [28, 22], "hazard", gravity=False, collide=False)
    P["vespa_dive"] = boss_prefab("vespa_dive", [12, 26], "hazard", gravity=False)
    P["vespa_stuck"] = boss_prefab("vespa_stuck", [28, 22], "weak")
    P["vespa_rise"] = boss_prefab("vespa", [28, 22], "hazard", gravity=False, collide=False)
    P["vespa_hurt"] = boss_prefab("vespa_hurt", [28, 22], "safe")
    P["vespa_dead"] = boss_prefab("vespa_dead", [28, 22], "safe", collide=False) | {"ttl": 3}
    P["baron_wait"] = boss_prefab("baron", [42, 30], "hazard")
    P["baron_walk"] = boss_prefab("baron", [42, 30], "hazard", walkto(28))
    P["baron_stride"] = boss_prefab("baron", [42, 30], "hazard", walkto(42))
    P["baron_wind"] = boss_prefab("baron_wind", [42, 30], "hazard")
    P["baron_charge"] = boss_prefab("baron_charge", [42, 30], "hazard", walkto(215))
    P["baron_dazed"] = boss_prefab("baron_dazed", [42, 30], "weak")
    P["baron_hurt"] = boss_prefab("baron_hurt", [42, 30], "safe")
    P["baron_dead"] = boss_prefab("baron_dead", [42, 30], "safe", collide=False) | {"ttl": 3}
    P["thornseed"] = {"tag": ["hazard", "bproj"], "shape": {"sprite": "thornseed"}, "size": [7, 7], "layer": 5, "body": {"type": "dynamic", "gravity": 700, "max_fall": 400}, "ttl": 3}
    P["spike_warn"] = {"tag": ["fx"], "shape": {"sprite": "spike_warn"}, "size": [16, 6], "layer": 2, "ttl": 0.75}
    P["spike"] = {"tag": ["hazard", "bproj"], "shape": {"sprite": "spike"}, "size": [14, 14], "layer": 5, "ttl": 0.9}

    # one prefab per kind of lift and thorn whirl the levels use
    for L in STAGE_LEVELS + BONUS_LEVELS:
        for prefab, *_ in L.scene:
            if prefab.startswith("lift_") and prefab not in P:
                _, axis, rng, speed = prefab.split("_")
                P[prefab] = {"tag": "solid", "shape": {"sprite": "lift"}, "size": [48, 6], "layer": 2, "body": {"type": "dynamic"}, "move": {"patrol": {"axis": axis, "range": int(rng), "speed": int(speed)}}}
            if prefab.startswith("orb_") and prefab not in P:
                _, radius, speed, phase = prefab.split("_")
                P[prefab] = {"tag": "hazard", "shape": {"sprite": "orb"}, "size": [6, 6], "layer": 4, "move": {"orbit": {"radius": int(radius), "speed": int(speed), "phase": int(phase)}}}
    return P


def legend(theme):
    return {
        "=": f"{theme}_top" if theme != "keep" else "keep_stone",
        "#": f"{theme}_soil",
        "W": f"{theme}_stone",
        "B": f"{theme}_brick",
        "R": f"{theme}_starbrick",
        "?": "combcoin",
        "P": "combpower",
        "L": "comblife",
        "U": "used",
        "X": "hard",
        "[": "log_tl",
        "]": "log_tr",
        "{": "log_bl",
        "}": "log_br",
        "(": "leaf_l",
        "-": "leaf_m",
        ")": "leaf_r",
        "|": "trunk",
        "~": "lava",
        "%": "lava_deep",
        "A": "abyss",
        "o": "honey",
        "g": "grubble",
        "d": "dgrubble",
        "k": "snail",
        "s": "spikebug",
    }


DROP = ("cloud", "dcloud", "hill", "bush", "banner", "hut", "stars")  # scenery the parallax backdrop draws now
STAR_SKIES = ("dusk", "night", "ash")


def look_of(L):
    return {"castle": "keep"}.get(L.theme, L.theme)


def level_extras(look, ground_row=13):
    """A level's sky and parallax bands (and a starry band in the night skies; inside the hive, walls of honeycomb)."""
    out = {"sky": art.sky(look), "backdrop": art.backdrop(look, VIEW_H, TOP + ground_row * T)}
    if look == "hive_inside":
        out["backdrop"] = [{"sprites": ["comb_wall"], "y": y, "parallax": 0.1, "seed": y} for y in range(28, 260, 28)]
    if look in STAR_SKIES:
        out["backdrop"].insert(0, {"sprites": ["stars"], "y": 120, "parallax": 0.04, "spacing": [10, 40], "seed": 11})
        out["backdrop"].insert(0, {"sprites": ["stars"], "y": 70, "parallax": 0.02, "spacing": [0, 30], "seed": 12})
    return out


def level_json(L, start_prefab="pip_s"):
    L.place_decor()
    rows = L.rows()
    used = set("".join(rows)) - {"."}
    lg = {k: v for k, v in legend(look_of(L) if L.theme != "castle" else "keep").items() if k in used}
    if L.theme == "keep":
        lg = {k: v for k, v in legend("keep").items() if k in used}
    scene = []
    if start_prefab:
        sc, sr = L.start
        scene.append({"prefab": start_prefab, "at": [cx(sc), (sr + 1) * T - SMALL[1] / 2 + TOP], "id": "p"})
    solid = set("=#WBRXU?PL[]{}()-_")
    for prefab, x, y, sid in L.scene:
        if prefab.startswith(DROP):
            continue
        if prefab == "midpoint":
            # the respawn point stands on the first solid tile of its column (the sensor is tall, so passing it anywhere counts)
            col = int(x // T)
            top = next((r for r in range(3, levels.ROWS) if L.grid[r][col] in solid and L.grid[r - 1][col] not in solid), 13)
            y = top * T - SMALL[1] / 2
        item = {"prefab": prefab, "at": [x, y + TOP]}
        if sid:
            item["id"] = sid
        scene.append(item)
    if L.theme == "keep" and hasattr(L, "den"):
        w = STAGE_LEVELS.index(L) // 4 + 1
        boss = BOSSES[w - 1]
        bx = (L.w - 9) * T
        by = {"grubmaw": FLOOR_DEN - 11, "duskwing": TOP + 80, "burr": FLOOR_DEN - 9, "rime": TOP + 3 * T + 14, "sandmaw": FLOOR_DEN - 5, "gloam": TOP + 80,
              "vespa": TOP + 70, "baron": FLOOR_DEN - 15}[boss]
        scene.append({"prefab": f"{boss}_wait", "at": [bx, by], "id": "boss"})
        if L.friend:
            scene.append({"prefab": "cocoon", "at": [(L.w - 3) * T + 8, FLOOR_DEN - 10], "id": "cocoon"})
        if boss == "gloam":
            # two ledges to reach him when he sleeps on one
            for c0 in (L.den + 4, L.w - 9):
                scene.extend({"prefab": "keep_stone", "at": [cx(c), TOP + cy(9)]} for c in range(c0, c0 + 4))
    # the dark inside of every pit in the open (keeps have sap)
    for c in range(L.w):
        if L.grid[13][c] == "." and L.grid[14][c] == "." and L.theme != "keep":
            scene.append({"prefab": "pit_shade", "at": [cx(c), TOP + 13 * T + 24]})
    # an invisible wall at the right end of the world
    for r in range(levels.ROWS):
        scene.append({"prefab": "edge", "at": [L.w * T + 8, TOP + cy(r)]})
    # two rows above the grid (the HUD's band of sky; solid ceilings reach into it)
    top_ch = "W" if any(ch == "W" for ch in rows[0]) and L.theme in ("keep", "roots", "caves") else "."
    pad_row = "".join(top_ch if rows[0][c] == "W" else "." for c in range(L.w))
    rows = [pad_row, pad_row] + rows
    if top_ch == "W" and "W" not in lg:
        lg["W"] = legend(L.theme)["W"]
    j = {"id": L.id, "scene": scene, "map": {"tile": T, "origin": [0, TOP - 2 * T], "rows": rows, "legend": lg}, "world": [max(L.w * T, VIEW_W), VIEW_H],
         "background": L.background, "music": L.music}
    j.update(level_extras(look_of(L)))
    return j


def scenic(lid, look, w, music, items, ground=True):
    """A one-screen scene (title, hive, ending): ground, decor and characters, no Pip to steer."""
    L = levels.Level(lid, "", w, look, "#000000", music)
    if ground:
        L.ground(0, w - 1)
    if look != "hive_inside":
        L.decor_band(every=12, flowers=True, seed=3)
    L.decor_set = look
    for (p, col, y, sid) in items:
        L.add(p, cx(col) if isinstance(col, int) else col, y, sid)
    j = level_json(L, start_prefab=None)
    j["map"]["rows"] = j["map"]["rows"][:-1]
    j["map"]["legend"].pop("A", None)
    return j


def title_level():
    return scenic("title", "meadow", 30, "title", [("pip_pose", 9, 13 * T - 8, None), ("bloom", 22, 13 * T - 32, None), ("bramwell", 24, 13 * T - 9, None)])


def hive_level():
    j = scenic("hive", "hive_inside", 30, "hive", [("queen", 16, 13 * T - 12, None), ("pip_pose", 12, 13 * T - 8, None), ("friend1", 20, 13 * T - 9, None), ("friend3", 23, 13 * T - 9, None)])
    return j


def ending_level():
    items = [("queen", 15, 13 * T - 12, None), ("heartcomb", 15, 9 * T + 4, None), ("pip_flying", 15, 7 * T, None)]
    items += [(f"friend{k}", 4 + k * 3 - (2 if k > 3 else 0) + (6 if k > 3 else 0), 13 * T - 9, None) for k in range(1, 8)]
    items += [("snowfall", c, -20, None) for c in range(1, 30, 3)]
    return scenic("ending", "ending", 30, "ending", items)


def card_level():
    return {"id": "card", "scene": [{"prefab": "pip_pose", "at": [VIEW_W / 2 - 30, 160]}], "world": [VIEW_W, VIEW_H], "background": "#1a120c",
            "music": "hush", "sky": ["#2a1a10", "#120c08"]}


def gameover_level():
    return {"id": "gameover", "scene": [{"prefab": "mark", "at": [10, 10]}], "world": [VIEW_W, VIEW_H], "background": "#000000", "music": "hush"}


# ---- rules -------------------------------------------------------------------------------------------------------------------------------------
KINDS = {  # what stomping, kicking or burning each kind of foe does
    "grubble": {"stomp": "grubble_flat", "dead": "grubble_dead"},
    "dgrubble": {"stomp": "dgrubble_flat", "dead": "dgrubble_dead"},
    "snail": {"stomp": "shell", "dead": "snail_dead"},
    "wasp": {"stomp": "wasp_dead", "dead": "wasp_dead"},
    "spikebug": {"stomp": None, "dead": "spikebug_dead"},
    "snapvine": {"stomp": None, "dead": "snapvine_dead"},
}


def stage_sum(values):
    """An expression that is values[stage] (the rules are global; per-stage numbers become a sum of `(stage == k) * v`)."""
    return " + ".join(f"{v} * (stage == {k})" for k, v in values.items() if v) or "0"


def rules():
    R = []
    r = lambda rid, when, do, cond=None, **kw: R.append({"id": rid, "when": when, **({"if": cond} if cond else {}), **kw, "do": do})
    tick = {"every": 0.05}
    goto_card = [{"set": ["mode", 1]}, {"set": ["mt", "time"]}, {"set": ["talked", 0]}, {"level": "card"}]

    r("every conversation starts closed", {"start": True}, [{"set": [f"{d}_line", 99]} for d in ("prologue", "rescue", "final", "ending")])

    # --- the title: a menu (Continue only when there is a saved journey); up/down choose, A picks
    lo = "(1 - has_save)"
    r("title: up", {"press": "up"}, [{"set": ["tsel", f"(tsel - 1 < {lo}) * 3 + (tsel - 1 >= {lo}) * (tsel - 1)"]}, {"play": "menu"}], "mode == 0 && level == 0")
    r("title: down", {"press": "down"}, [{"set": ["tsel", f"(tsel + 1 > 3) * {lo} + (tsel + 1 <= 3) * (tsel + 1)"]}, {"play": "menu"}], "mode == 0 && level == 0")
    r("title: continue", {"press": "action"}, [{"apply": "continue_game"}], "mode == 0 && level == 0 && tsel == 0 && has_save == 1")
    r("title: a new journey", {"press": "action"}, [{"apply": "new_game"}], "mode == 0 && level == 0 && tsel == 1")
    r("title: fullscreen", {"press": "action"}, [{"window": "fullscreen"}, {"play": "select"}], "mode == 0 && level == 0 && tsel == 2")
    r("title: close", {"press": "action"}, [{"window": "close"}], "mode == 0 && level == 0 && tsel == 3")
    r("title: Start also begins", {"press": "pause"}, [{"apply": "continue_game"}], "mode == 0 && level == 0 && has_save == 1 && tsel == 0")
    r("title: Start begins a new journey", {"press": "pause"}, [{"apply": "new_game"}], "mode == 0 && level == 0 && (has_save == 0 || tsel == 1)")
    r("title: the cursor starts on what makes sense", {"event": "loaded"}, [{"set": ["tsel", "1 - has_save"]}])
    r("title: after a restart too", {"start": True}, [{"set": ["tsel", "1 - has_save"]}])
    r("a saved stage out of range is a fresh start", {"event": "loaded"}, [{"set": ["has_save", 0]}, {"set": ["sv_stage", 0]}, {"set": ["tsel", 1]}], f"sv_stage < 0 || sv_stage >= {N_STAGES} || sv_lives < 1")
    r("a stage played directly starts as if from its card", tick, [{"set": ["mode", 2]}, {"set": ["stage", f"level - {FIRST}"]}, {"apply": "stage_setup"}, {"emit": "reform"}],
      f"mode == 0 && level >= {FIRST} && level < {FIRST + N_STAGES}")

    # --- the prologue: the Queen's call
    r("the prologue speaks", tick, [{"set": ["prologue_line", 0]}, {"set": ["talked", 1]}], "mode == 8 && talked == 0 && time > mt + 1.2")
    r("the prologue ends: the road begins", {"event": "prologue_done"}, [{"set": ["stage", 0]}] + goto_card)

    # --- the card, then the stage; every card is an autosave
    into_stage = [{"set": ["mode", 2]}, {"apply": "stage_setup"}, {"level": f"stage + {FIRST}"}]
    r("the card saves the journey", tick, [{"apply": "save_now"}], "mode == 1 && time > mt + 0.2", cooldown=1.0)
    r("the card leads into the stage", tick, into_stage + [{"emit": "reform"}], "mode == 1 && time > mt + 2.4 && checkpoint == 0")
    r("the card leads into the stage at its midpoint", tick, into_stage + [{"teleport": {"target": "id:p", "to": ["cp_x", "cp_y"]}}, {"emit": "reform"}], "mode == 1 && time > mt + 2.4 && checkpoint == 1")
    r("world and stage numbers", tick, [{"set": ["w", "floor(stage / 4) + 1"]}, {"set": ["s", "stage % 4 + 1"]}])

    # --- Pip's form: one event puts the right prefab on Pip
    for prefab, f in (("pip_s", "form == 0"), ("pip_b", "form == 1"), ("pip_f", "form == 2")):
        r(f"reform {prefab}", {"event": "reform"}, [{"become": {"target": "id:p", "prefab": prefab}}], f"mode == 2 && star == 0 && blink == 0 && {f}")
        r(f"reform {prefab} star", {"event": "reform"}, [{"become": {"target": "id:p", "prefab": prefab + "_star"}}], f"mode == 2 && star == 1 && {f}")
    r("reform blinking", {"event": "reform"}, [{"become": {"target": "id:p", "prefab": "pip_s_blink"}}], "mode == 2 && star == 0 && blink == 1")
    for k, f in (("s", "form == 0"), ("b", "form == 1"), ("f", "form == 2")):
        r(f"walk on through the bloom {k}", {"event": "reform"}, [{"become": {"target": "id:p", "prefab": f"walk_{k}"}}], f"mode == 4 && {f}")
    r("blinking ends", {"every": 0.1}, [{"set": ["blink", 0]}, {"emit": "reform"}], f"{PLAY} && blink == 1 && time > inv_until")
    r("star power ends", {"every": 0.1}, [{"set": ["star", 0]}, {"music": "level"}, {"emit": "reform"}], f"{PLAY} && star == 1 && time > star_until")

    # --- moving: jumps and pollen puffs
    r("jump sound", {"press": "action"}, [{"play": "jump"}], f"{PLAY} && form == 0 && self_vy == 0")
    r("big jump sound", {"press": "action"}, [{"play": "jump_big"}], f"{PLAY} && form >= 1 && self_vy == 0")
    r("throw a pollen puff", {"press": "secondary"}, [{"spawn": {"prefab": "seed", "at": ["self_x + self_face * 8", "self_y - 4"], "vel": ["self_face * 220", 80]}}, {"play": "seed"}], f"{PLAY} && form == 2 && count_seed < 2")
    r("a puff bursts on a wall", {"hit": ["seed", "solid"]}, [{"burst": {"at": "self", "n": 6, "color": "#ffb03a", "speed": [20, 50], "life": [0.1, 0.3]}}, {"destroy": "self"}], side="sides")
    r("landing kicks up dust", {"hit": ["hero", "solid"]}, [{"burst": {"at": ["self_x", "self_y + 7"], "n": 5, "color": "#e8d8b8", "speed": [10, 30], "life": [0.15, 0.35], "gravity": -40}}], f"{PLAY} && self_vy > 300", side="bottom", cooldown=0.25)

    # --- the Baron's creatures
    for kind, k in KINDS.items():
        knock = [{"become": {"target": "other", "prefab": k["dead"]}}, {"velocity": {"target": "other", "v": [0, -220]}}, {"play": "kick"}]
        r(f"star power beats a {kind}", {"touch": ["star", kind]}, knock, PLAY)
        r(f"a puff of pollen beats a {kind}", {"touch": ["seed", kind]}, knock + [{"destroy": "self"}])
        r(f"a shell bowls over a {kind}", {"touch": ["kshell", kind]}, knock)
        r(f"a bump from below knocks over a {kind}", {"touch": ["bumper", kind]}, knock)
        if k["stomp"]:
            r(f"stomp a {kind}", {"touch": ["hero", kind]}, [{"become": {"target": "other", "prefab": k["stomp"]}}, {"velocity": {"target": "other", "v": [0, 0]}}, {"velocity": {"target": "self", "v": ["self_vx", -290]}},
                                                               {"add": ["stomps", 1]}, {"burst": {"at": "other", "n": 8, "color": "#ffe9b0", "speed": [20, 60], "life": [0.15, 0.35]}}, {"play": "stomp"}], f"{PLAY} && star == 0", side="bottom")
            r(f"a {kind} hurts", {"touch": ["vuln", kind]}, [{"emit": "hurt"}], PLAY, side=["top", "left", "right"])
        else:
            r(f"a {kind} hurts from every side", {"touch": ["vuln", kind]}, [{"emit": "hurt"}], PLAY)
    r("star power beats a shell", {"touch": ["star", "kshell"]}, [{"destroy": "other"}, {"play": "kick"}], PLAY)
    r("stop a rolling shell from above", {"touch": ["hero", "kshell"]}, [{"become": {"target": "other", "prefab": "shell"}}, {"velocity": {"target": "other", "v": [0, 0]}}, {"velocity": {"target": "self", "v": ["self_vx", -290]}}, {"play": "stomp"}], f"{PLAY} && star == 0", side="bottom")
    r("a rolling shell hurts", {"touch": ["vuln", "kshell"]}, [{"emit": "hurt"}], PLAY, side=["top", "left", "right"])
    kick_dir = "(2 * (other_x >= self_x) - 1)"
    r("kick a shell", {"touch": ["hero", "shell"]}, [{"become": {"target": "other", "prefab": "kshell"}}, {"teleport": {"target": "other", "to": [f"self_x + {kick_dir} * 14", "other_y"]}}, {"velocity": {"target": "other", "v": [f"{kick_dir} * 230", 0]}}, {"add": ["kicks", 1]}, {"play": "kick"}], PLAY)
    r("two shells knock each other out", {"touch": ["kshell", "kshell"]}, [{"destroy": "self"}, {"destroy": "other"}, {"play": "kick"}])
    r("a hazard hurts", {"touch": ["vuln", "hazard"]}, [{"emit": "hurt"}], PLAY)
    r("foes fall out of the world", {"touch": ["foe", "abyss"]}, [{"destroy": "self"}])
    r("items fall out of the world", {"touch": ["item", "abyss"]}, [{"destroy": "self"}])

    # --- getting hurt, losing a life
    r("hurt when small: lose a life", {"event": "hurt"}, [{"apply": "lose_life", "with": {"hop": -360}}], f"{PLAY} && form == 0")
    r("hurt when grown: shrink", {"event": "hurt"}, [{"set": ["form", 0]}, {"set": ["blink", 1]}, {"set": ["inv_until", "time + 2"]}, {"play": "shrink"}, {"shake": 2}, {"emit": "reform"}], f"{PLAY} && form >= 1")
    r("falling into a pit", {"touch": ["hero", "abyss"]}, [{"apply": "lose_life", "with": {"hop": 0}}], PLAY)
    r("lost: the card again", {"every": 0.1}, [{"add": ["lives", -1]}] + goto_card, "mode == 3 && time > dead_at + 2.6 && lives > 1")
    r("lost the last life: game over", {"every": 0.1}, [{"set": ["lives", 0]}, {"set": ["mode", 5]}, {"set": ["mt", "time"]}, {"set": ["sv_lives", 5]}, {"set": ["sv_cp", 0]},
                                                      {"save": True}, {"level": "gameover"}, {"play": "gameover"}], "mode == 3 && time > dead_at + 2.6 && lives <= 1")
    r("game over: back to the title", {"every": 0.1}, [{"restart": True}], "mode == 5 && time > mt + 4.5")

    # --- blocks
    above = ["other_x", "other_y - 16"]
    bump = {"spawn": {"prefab": "bumper", "at": ["other_x", "other_y - 12"]}}
    r("a comb gives honey", {"hit": ["hero", "coinblock"]}, [{"become": {"target": "other", "prefab": "used"}}, {"spawn": {"prefab": "coinpop", "at": above, "vel": [0, -300]}}, {"apply": "gain_honey"}, bump], PLAY, side="top")
    r("a comb gives royal jelly", {"hit": ["hero", "powerblock"]}, [{"become": {"target": "other", "prefab": "used"}}, {"spawn": {"prefab": "jelly", "at": above}}, {"play": "sprout"}, bump], f"{PLAY} && form == 0", side="top")
    r("a comb gives a firebloom", {"hit": ["hero", "powerblock"]}, [{"become": {"target": "other", "prefab": "used"}}, {"spawn": {"prefab": "firebloom", "at": above}}, {"play": "sprout"}, bump], f"{PLAY} && form >= 1", side="top")
    r("a wax block hides a sunpetal", {"hit": ["hero", "starblock"]}, [{"become": {"target": "other", "prefab": "used"}}, {"spawn": {"prefab": "starpetal", "at": above, "vel": [0, -200]}}, {"play": "sprout"}, bump], PLAY, side="top")
    r("a comb hides a clover", {"hit": ["hero", "lifeblock"]}, [{"become": {"target": "other", "prefab": "used"}}, {"spawn": {"prefab": "clover", "at": above}}, {"play": "sprout"}, bump], PLAY, side="top")
    r("grown Pip breaks wax", {"hit": ["hero", "brick"]}, [{"destroy": "other"}, {"burst": {"at": "other", "n": 14, "color": "#f2c46a", "speed": [60, 140], "life": [0.4, 0.8], "size": 3, "gravity": 700}}, {"play": "break"}, bump], f"{PLAY} && form >= 1", side="top")
    r("small Pip bumps wax", {"hit": ["hero", "brick"]}, [{"play": "bump"}, bump], f"{PLAY} && form == 0", side="top")
    r("a spent block just bumps", {"hit": ["hero", "spent"]}, [{"play": "bump"}], PLAY, side="top", cooldown=0.15)
    r("honey above a bumped block is collected", {"touch": ["bumper", "honey"]}, [{"destroy": "other"}, {"apply": "gain_honey"}])

    # --- items
    r("honey", {"touch": ["hero", "honey"]}, [{"destroy": "other"}, {"apply": "gain_honey"}], PLAY)
    r("a hundred honeydrops make a life", {"event": "honeycheck"}, [{"add": ["coins", -100]}, {"add": ["lives", 1]}, {"play": "oneup"}, {"spawn": {"prefab": "pop1UP", "at": ["p_x", "p_y - 16"]}}], "coins >= 100")
    r("royal jelly grows Pip", {"touch": ["hero", "jelly"]}, [{"destroy": "other"}, {"play": "powerup"}, {"set": ["form", "max(form, 1)"]}, {"emit": "reform"}], PLAY)
    r("a firebloom fills Pip with pollen", {"touch": ["hero", "firebloom"]}, [{"destroy": "other"}, {"play": "powerup"}, {"set": ["form", 2]}, {"emit": "reform"}], PLAY)
    r("a sunpetal gives sun power", {"touch": ["hero", "starpetal"]}, [{"destroy": "other"}, {"play": "powerup"}, {"set": ["star", 1]}, {"set": ["star_until", "time + 10"]}, {"music": "star"}, {"emit": "reform"}], PLAY)
    r("a clover is a life", {"touch": ["hero", "clover"]}, [{"destroy": "other"}, {"add": ["lives", 1]}, {"spawn": {"prefab": "pop1UP", "at": "other"}}, {"play": "oneup"}], PLAY)
    r("a sunpetal bounces", {"hit": ["starpetal", "solid"]}, [{"velocity": {"target": "self", "v": ["self_vx", -320]}}], side="bottom")

    # --- hollow stumps: down into a honey room, out through the side
    bonus_of = " + ".join(f"{IX[b.id]} * (stage == {[s.id for s in STAGE_LEVELS].index(b.id[:-1])})" for b in BONUS_LEVELS)
    exit_x = stage_sum({i: L.exit_col * T + 16 for i, L in enumerate(STAGE_LEVELS) if hasattr(L, "exit_col")})
    r("down a hollow stump", {"touching": ["hero", "logmouth"]}, [{"play": "pipe"}, {"level": bonus_of}, {"emit": "reform"}], f"{PLAY} && held_down == 1")
    r("out of the honey room", {"touch": ["hero", "logside"]}, [{"play": "pipe"}, {"level": f"stage + {FIRST}"}, {"teleport": {"target": "id:p", "to": [exit_x, TOP + 11 * T - 8]}}, {"emit": "reform"}], PLAY)
    r("the midpoint", {"touch": ["hero", "midpoint"]}, [{"set": ["checkpoint", 1]}, {"set": ["cp_x", "other_x"]}, {"set": ["cp_y", "other_y"]}, {"destroy": "other"},
                                                       {"set": ["sv_cp", 1]}, {"set": ["sv_cpx", "cp_x"]}, {"set": ["sv_cpy", "cp_y"]}, {"save": True}], PLAY)

    # --- the road's folk: bump into one and they say their line
    r("the road's folk speak", {"touch": ["hero", "folk"]}, [{"set": ["talk_until", "time + 5"]}, {"play": "talk"}], PLAY, cooldown=5.5)

    # --- the Bloom Gate
    r("the Bloom Gate", {"touch": ["hero", "goal"]}, [{"set": ["mode", 4]}, {"set": ["mt", "time"]}, {"become": {"target": "other", "prefab": "bloom_open"}}, {"play": "bloom"}, {"music": "hush"},
                                                      {"burst": {"at": "other", "n": 30, "color": "#fff27a", "speed": [30, 90], "life": [0.5, 1.2], "gravity": -20}}, {"emit": "reform"}], PLAY)
    r("on to the next stage", {"every": 0.1}, [{"add": ["stage", 1]}, {"set": ["checkpoint", 0]}, {"set": ["sv_cp", 0]}] + goto_card, "mode == 4 && time > mt + 3.2")

    # --- the pause menu: Start opens it, the world stands still (`pause_if`), up/down choose, A picks, B or Start goes back
    R.extend(pause_rules())
    R.extend(boss_rules())

    r("keep the furthest stage", {"every": 0.5}, [{"set": ["best", "stage"]}], f"stage > best && mode == 2")
    return R


MENU = ["RESUME", "SAVE GAME", "SAVE THIS SONG", "MUSIC", "FULLSCREEN", "QUIT TO TITLE", "CLOSE GAME"]


def pause_rules():
    R = []
    r = lambda rid, when, do, cond=None, **kw: R.append({"id": rid, "when": when, **({"if": cond} if cond else {}), **kw, "do": do})
    n = len(MENU)
    # closing first, so the press that opens it is not also the press that closes it (rules run in order and see each other's changes)
    close = [{"set": ["menu", 0]}, {"set": ["menu_tick", "tick"]}, {"play": "unpause"}]
    r("pause menu: Start closes it", {"press": "pause"}, close, "menu == 1")
    r("pause menu: B closes it", {"press": "secondary"}, close, "menu == 1")
    r("pause menu: Start opens it", {"press": "pause"}, [{"set": ["menu", 1]}, {"set": ["msel", 0]}, {"set": ["msg", 0]}, {"play": "pause"}], f"mode == 2 && menu == 0 && tick > menu_tick + 1")
    r("pause menu: up", {"press": "up"}, [{"set": ["msel", f"(msel + {n - 1}) % {n}"]}, {"play": "menu"}], "menu == 1")
    r("pause menu: down", {"press": "down"}, [{"set": ["msel", f"(msel + 1) % {n}"]}, {"play": "menu"}], "menu == 1")
    pick = lambda k: f"menu == 1 && msel == {k}"
    r("pause menu: resume", {"press": "action"}, close, pick(0))
    r("pause menu: save the game", {"press": "action"}, [{"apply": "save_now"}, {"set": ["msg", 1]}, {"set": ["msg_tick", "tick"]}, {"play": "saved"}], pick(1))
    r("pause menu: save this song", {"press": "action"}, [{"export_music": True}, {"play": "select"}], pick(2))
    r("the song was saved", {"event": "music_exported"}, [{"set": ["msg", 2]}, {"set": ["msg_tick", "tick"]}, {"play": "saved"}])
    r("the song could not be saved", {"event": "music_not_exported"}, [{"set": ["msg", 3]}, {"set": ["msg_tick", "tick"]}])
    r("pause menu: music on or off", {"press": "action"}, [{"music": "toggle"}, {"play": "select"}], pick(3))
    r("pause menu: fullscreen", {"press": "action"}, [{"window": "fullscreen"}, {"play": "select"}], pick(4))
    r("pause menu: quit to the title", {"press": "action"}, [{"apply": "save_now"}, {"restart": True}], pick(5))
    r("pause menu: close the game", {"press": "action"}, [{"apply": "save_now"}, {"window": "close"}], pick(6))
    return R


# ---- the bosses -----------------------------------------------------------------------------------------------------------------------------------------
# Each boss is a little state machine in `bs`: 0 waiting in its den, 1 its main move, 2-7 its own moves, 8 hit (blinking, can't be hit), 9 beaten.
# A move lasts until a time (`time > bt + s`) or until something happens (it lands, it hits a wall). A boss is hit by landing on it while it is
# `bstomp` or `bweak` (its pose's role), or by a pollen puff; its tell comes before the move that hurts, and its weak pose after.

HURT_RESUME = {1: 1, 2: 4, 3: 1, 4: 4, 5: 1, 6: 4, 7: 5, 8: 1}


def boss_rules():
    R = []
    r = lambda rid, when, do, cond=None, **kw: R.append({"id": rid, "when": when, **({"if": cond} if cond else {}), **kw, "do": do})
    tick = {"every": 0.05}
    B = "id:boss"
    to_pip = "(2 * (p_x > boss_x) - 1)"  # +1 when Pip is to the boss's right
    become = lambda prefab: {"become": {"target": B, "prefab": prefab}}
    vel = lambda vx, vy: {"velocity": {"target": B, "v": [vx, vy]}}

    def state(k, s, after, actions, to, extra=""):
        """Boss k in state s moves on to state `to` once `after` seconds have passed."""
        r(f"{BOSSES[k - 1]}: {s} -> {to}", tick, actions + [{"set": ["bs", to]}, {"set": ["bt", "time"]}], f"{PLAY} && den == 1 && bkind == {k} && bs == {s} && time > bt + {after}{extra}")

    def on(k, s, when, actions, to, side=None):
        """Boss k in state s moves on to state `to` when `when` happens (it lands)."""
        kw = {"side": side} if side else {}
        r(f"{BOSSES[k - 1]}: {s} -> {to} on {list(when)[0]}", when, actions + [{"set": ["bs", to]}, {"set": ["bt", "time"]}], f"{PLAY} && den == 1 && bkind == {k} && bs == {s}", **kw)

    # entering the den: the gate shuts, the music turns, the boss is named, then it wakes
    gate = [{"spawn": {"prefab": "gate", "at": ["den_l_x", TOP + 3 * T + 24 + k * 48]}} for k in range(3)]
    r("the den's gate shuts behind Pip", {"touch": ["hero", "densensor"]}, [{"destroy": "other"}, {"set": ["den", 1]}, {"set": ["bs", 0]}, {"set": ["bt", "time"]}, {"set": ["intro_until", "time + 2.6"]},
                                                                             *gate, {"play": "gate"}, {"shake": 4}, {"music": "hush"}], f"{PLAY} && den == 0")
    r("the lieutenant's theme", tick, [{"music": "boss"}], f"{PLAY} && den == 1 && bs == 0 && time > bt + 0.8 && bkind < 8", once=False, cooldown=1.0)
    r("the Baron's theme", tick, [{"music": "baron"}], f"{PLAY} && den == 1 && bs == 0 && time > bt + 0.8 && bkind == 8", once=False, cooldown=1.0)

    # waking: each boss's first move
    wake = {1: [become("grubmaw_walk"), vel(f"{to_pip} * 34", 0)], 2: [become("duskwing_fly")], 3: [become("burr_walk"), vel(f"{to_pip} * 30", 0)], 4: [become("rime_hang")],
            5: [become("sand_mound"), vel(f"{to_pip} * 44", 0)], 6: [become("gloam_hover"), {"set": ["perch", 2]}], 7: [become("vespa_hover")], 8: [become("baron_walk"), vel(f"{to_pip} * 28", 0)]}
    for k, acts in wake.items():
        state(k, 0, 1.6, acts + [{"play": "boss_roar"}, {"shake": 3}], 1)

    # --- 1 Grubmaw: walks at Pip and belly-flops (the floor shakes); landing on his head is the way. Faster as he weakens.
    state(1, 1, 2.4, [vel(f"{to_pip} * 70", -330), {"play": "whoosh"}], 2)
    on(1, 2, {"hit": ["boss", "solid"]}, [{"shake": 4}, {"play": "thud"}, {"burst": {"at": "self", "n": 16, "color": "#d8b890", "speed": [30, 90], "life": [0.3, 0.6], "gravity": 300}}], 1, side="bottom")
    state(1, 2, 2.0, [], 1)

    # --- 2 Madam Duskwing: drifts above dropping dust, then swoops at Pip and must rest on the ground (land on her then), then rises.
    r("duskwing: dust", {"every": 1.6}, [{"spawn": {"prefab": "dust", "at": ["boss_x", "boss_y + 10"]}}], f"{PLAY} && den == 1 && bkind == 2 && bs == 1")
    state(2, 1, 4.6, [become("duskwing_dive"), vel("(p_x - boss_x) * 1.2", 200), {"play": "whoosh"}], 2)
    on(2, 2, {"hit": ["boss", "solid"]}, [become("duskwing_rest"), vel(0, 0), {"play": "thud"}, {"shake": 2}], 3)
    state(2, 2, 1.8, [become("duskwing_rest"), vel(0, 0)], 3)
    state(2, 3, 2.2, [become("duskwing_rise"), vel("(den_c_x - boss_x) * 1.1", -150), {"play": "whoosh"}], 4)
    state(2, 4, 0.1, [become("duskwing_fly"), vel(0, 0)], 1, extra=f" && boss_y <= {TOP + 80}")
    state(2, 4, 2.5, [become("duskwing_fly"), vel(0, 0)], 1)

    # --- 3 Captain Burr: marches, curls into a ball of prickles and rolls wall to wall (jump it: flutter helps), then uncurls dizzy.
    state(3, 1, 2.8, [become("burr_ball"), vel(f"{to_pip} * 150", -120), {"play": "charge"}], 2, extra=" && bhp >= 2")
    state(3, 1, 2.4, [become("burr_ball_fast"), vel(f"{to_pip} * 195", -120), {"play": "charge"}], 2, extra=" && bhp < 2")
    state(3, 2, 3.4, [become("burr_dizzy"), vel(0, -120), {"play": "bump"}], 3)
    state(3, 3, 2.2, [become("burr_walk"), vel(f"{to_pip} * 30", 0)], 1)
    r("burr: the ball thuds off a wall", {"hit": ["boss", "solid"]}, [{"shake": 2}, {"play": "bump"}], f"{PLAY} && bkind == 3 && bs == 2", side="sides", cooldown=0.2)

    # --- 4 Rimeweaver: hangs from the roof dropping icicles, slides above Pip (the tell) and drops; the fall dazes her (land on her back), then she climbs.
    r("rimeweaver: icicles", {"every": 1.5}, [{"spawn": {"prefab": "icicle", "at": ["boss_x", "boss_y + 14"]}}, {"play": "zap"}], f"{PLAY} && den == 1 && bkind == 4 && bs == 1")
    r("an icicle shatters", {"hit": ["icicle", "solid"]}, [{"burst": {"at": "self", "n": 8, "color": "#d0f0ff", "speed": [20, 60], "life": [0.2, 0.4], "gravity": 300}}, {"destroy": "self"}])
    state(4, 1, 4.6, [become("rime_aim"), vel("(min(den_r_x - 48, max(den_l_x + 48, p_x + 56 * (2 * (boss_x > p_x) - 1))) - boss_x) * 1.6", 0), {"play": "charge"}], 5)
    state(4, 5, 0.6, [become("rime_drop"), vel(0, 60), {"play": "whoosh"}], 2)
    on(4, 2, {"hit": ["boss", "solid"]}, [become("rime_ground"), vel(0, 0), {"play": "thud"}, {"shake": 3}], 3, side="bottom")
    state(4, 3, 3.5, [become("rime_climb"), vel("(den_c_x - boss_x) * 1.1", -140)], 4)
    state(4, 4, 0.1, [become("rime_hang"), vel(0, 0)], 1, extra=f" && boss_y <= {TOP + 3 * T + 14}")
    state(4, 4, 2.0, [become("rime_hang"), vel(0, 0)], 1)

    # --- 5 Sandmaw: a mound that follows Pip; it stops and trembles (the tell), then he bursts out throwing sand: land on his head while he is up.
    r("sandmaw: the mound follows Pip", {"every": 0.6}, [vel(f"{to_pip} * 44", 0)], f"{PLAY} && den == 1 && bkind == 5 && bs == 1")
    state(5, 1, 2.6, [become("sand_rumble"), vel(0, 0), {"shake": 1}, {"play": "thud"}], 2)
    clods = [{"spawn": {"prefab": "clod", "at": ["boss_x", "boss_y - 10"], "vel": [vx, vy]}} for vx, vy in ((-110, -300), (0, -360), (110, -300))]
    state(5, 2, 0.7, [become("sandmaw_up"), vel(0, -230), {"play": "boss_roar"}, {"shake": 3}] + clods, 3)
    state(5, 3, 2.2, [become("sand_mound"), vel(f"{to_pip} * 44", 0), {"play": "whoosh"}], 1)

    # --- 6 Gloam: hovers at a perch, glows (the tell) and throws three sparks at Pip; then he is sleepy and sinks onto a ledge (land on him), then blinks away.
    perch_x = "(perch == 0) * (den_l_x + 88) + (perch == 1) * (den_r_x - 104) + (perch == 2) * den_c_x"
    perch_y = f"(perch == 2) * {TOP + 70} + (perch != 2) * {TOP + 96}"
    state(6, 1, 1.2, [become("gloam_charge"), {"play": "charge"}], 2)
    d = "sqrt((p_x - boss_x) * (p_x - boss_x) + (p_y - boss_y) * (p_y - boss_y) + 1)"
    sparks = [{"spawn": {"prefab": "spark", "at": "self" if False else ["boss_x", "boss_y"], "vel": [f"120 * (p_x - boss_x) / {d} + {dx}", f"120 * (p_y - boss_y) / {d} + {dy}"]}} for dx, dy in ((0, 0), (-40, -30), (40, -30))]
    state(6, 2, 0.8, [become("gloam_drowsy"), {"play": "zap"}] + sparks, 3, extra=" && bhp >= 3")
    state(6, 2, 0.6, [become("gloam_drowsy"), {"play": "zap"}] + sparks + [{"spawn": {"prefab": "spark", "at": ["boss_x", "boss_y"], "vel": [0, 120]}}], 3, extra=" && bhp < 3")
    state(6, 3, 2.6, [become("gloam_hover"), {"set": ["perch", "(perch + 1 + random(0, 1)) % 3"]}, {"play": "whoosh"}], 4)
    state(6, 4, 0.05, [{"teleport": {"target": B, "to": [perch_x, perch_y]}}, vel(0, 0), {"burst": {"at": ["boss_x", "boss_y"], "n": 12, "color": "#fff27a", "speed": [20, 60], "life": [0.2, 0.5]}}], 1)

    # --- 7 Queen Vespa: patrols high and calls her wasps; then she lines up over Pip (the tell: she stops and hums) and dives stinger first; a miss sticks her in the ground.
    r("vespa: she calls her wasps", {"every": 5.0}, [{"spawn": {"prefab": "waspx", "at": ["boss_x", "boss_y + 20"]}}, {"play": "charge"}], f"{PLAY} && den == 1 && bkind == 7 && bs == 1 && count_wasp < 2")
    state(7, 1, 3.6, [become("vespa_aim"), vel("(min(den_r_x - 40, max(den_l_x + 40, p_x)) - boss_x) * 2.2", 0), {"play": "charge"}], 2)
    state(7, 2, 0.5, [become("vespa_dive"), vel(0, 330), {"play": "whoosh"}], 3)
    on(7, 3, {"hit": ["boss", "solid"]}, [become("vespa_stuck"), vel(0, 0), {"play": "thud"}, {"shake": 4}], 4, side="bottom")
    state(7, 3, 1.6, [become("vespa_stuck"), vel(0, 0)], 4)
    state(7, 4, 2.4, [become("vespa_rise"), vel("(den_c_x - boss_x) * 1.1", -160), {"play": "whoosh"}], 5)
    state(7, 5, 0.1, [become("vespa_hover"), vel(0, 0)], 1, extra=f" && boss_y <= {TOP + 70}")
    state(7, 5, 2.0, [become("vespa_hover"), vel(0, 0)], 1)

    # --- 8 Baron Thornwick: stalks Pip throwing thorn seeds; lowers his horns (the tell) and charges; a wall stuns him (land on his back). Below half
    #     strength he strides faster and thorns burst from the floor where Pip stands (a glint marks the spot first).
    r("baron: he turns to Pip", {"every": 1.0}, [vel(f"{to_pip} * 28", 0)], f"{PLAY} && den == 1 && bkind == 8 && bs == 1")
    r("baron: thorn seeds", {"every": 2.4}, [{"spawn": {"prefab": "thornseed", "at": ["boss_x", "boss_y - 18"], "vel": ["(p_x - boss_x) * 0.8", -320]}}, {"play": "seed"}], f"{PLAY} && den == 1 && bkind == 8 && bs == 1")
    state(8, 1, 4.6, [become("baron_wind"), vel(f"{to_pip} * 1", 0), {"play": "charge"}], 2, extra=" && bhp > 3")
    state(8, 1, 3.4, [become("baron_wind"), vel(f"{to_pip} * 1", 0), {"play": "charge"}], 2, extra=" && bhp <= 3")
    state(8, 2, 0.8, [become("baron_charge"), vel(f"{to_pip} * 215", 0), {"play": "boss_roar"}], 3)
    on(8, 3, {"hit": ["boss", "solid"]}, [become("baron_dazed"), vel(0, -160), {"shake": 7}, {"play": "thud"}, {"burst": {"at": "self", "n": 20, "color": "#a02a5a", "speed": [40, 120], "life": [0.3, 0.7], "gravity": 400}}], 4, side="sides")
    state(8, 3, 3.0, [become("baron_dazed"), vel(0, 0)], 4)
    state(8, 4, 2.6, [become("baron_walk"), vel(f"{to_pip} * 28", 0)], 1, extra=" && bhp > 3")
    state(8, 4, 2.0, [become("baron_stride"), vel(f"{to_pip} * 42", 0)], 1, extra=" && bhp <= 3")
    r("baron: a glint where thorns will burst", {"every": 2.2}, [{"set": ["spike_x", "p_x"]}, {"set": ["spike_t", "time"]}, {"spawn": {"prefab": "spike_warn", "at": ["p_x", FLOOR_DEN - 3]}}],
      f"{PLAY} && den == 1 && bkind == 8 && bhp <= 3 && bs <= 2")
    r("baron: the thorns burst", tick, [{"spawn": {"prefab": "spike", "at": ["spike_x", FLOOR_DEN - 7]}}, {"set": ["spike_t", 0]}, {"play": "break"}, {"shake": 2}], f"{PLAY} && spike_t > 0 && time > spike_t + 0.75")

    # --- hits: landing on a stompable or weak pose, or a puff of pollen; then it blinks for a moment and carries on (or falls)
    hit = [{"emit": "bosshit"}]
    r("land on a boss's weak spot", {"touch": ["hero", "bstomp"]}, hit + [{"velocity": {"target": "self", "v": ["self_vx", -330]}}], f"{PLAY} && star == 0 && bs < 8", side="bottom")
    r("land on a boss who is down", {"touch": ["hero", "bweak"]}, hit + [{"velocity": {"target": "self", "v": ["self_vx", -330]}}], f"{PLAY} && bs < 8", side="bottom")
    r("a stompable boss's sides hurt", {"touch": ["vuln", "bstomp"]}, [{"emit": "hurt"}], f"{PLAY} && bs < 8", side=["top", "left", "right"])
    r("sun power hits a boss", {"touch": ["star", "boss"]}, hit, f"{PLAY} && bs < 8 && bs > 0", cooldown=1.0)
    r("pollen hits a stompable boss", {"touch": ["seed", "bstomp"]}, hit + [{"destroy": "self"}], f"{PLAY} && bs < 8")
    r("pollen hits a boss who is down", {"touch": ["seed", "bweak"]}, hit + [{"destroy": "self"}], f"{PLAY} && bs < 8")
    for k, b in enumerate(BOSSES, start=1):
        r(f"{b} is hit", {"event": "bosshit"}, [become(f"{b}_hurt")], f"bkind == {k} && bs < 8 && den == 1")
    r("a boss is hit", {"event": "bosshit"}, [{"add": ["bhp", -1]}, {"set": ["bs", 8]}, {"set": ["bt", "time"]}, {"play": "boss_hit"}, {"shake": 4},
                                              {"burst": {"at": ["boss_x", "boss_y"], "n": 18, "color": "#ff4fa3", "speed": [40, 110], "life": [0.2, 0.5]}}], "bs < 8 && den == 1")
    r("a boss falls", {"event": "bosshit"}, [{"emit": "bossdown"}], "bs == 8 && bhp <= 0 && den == 1")
    resume_pose = {1: "grubmaw_walk", 2: "duskwing_rise", 3: "burr_walk", 4: "rime_climb", 5: "sand_mound", 6: "gloam_hover", 7: "vespa_rise", 8: "baron_walk"}
    resume_vel = {1: vel(f"{to_pip} * 40", -120), 2: vel("(den_c_x - boss_x) * 1.1", -150), 3: vel(f"{to_pip} * 30", 0), 4: vel("(den_c_x - boss_x) * 1.1", -140), 5: vel(f"{to_pip} * 44", 0), 6: vel(0, 0), 7: vel("(den_c_x - boss_x) * 1.1", -160), 8: vel(f"{to_pip} * 28", 0)}
    for k, b in enumerate(BOSSES, start=1):
        pose = resume_pose[k]
        if k == 1:
            # Grubmaw speeds up as he weakens
            r("grubmaw shakes it off (hungrier)", tick, [become("grubmaw_fast"), resume_vel[1], {"set": ["bs", 1]}, {"set": ["bt", "time"]}], f"{PLAY} && den == 1 && bkind == 1 && bs == 8 && bhp == 2 && time > bt + 1.1")
            r("grubmaw shakes it off (furious)", tick, [become("grubmaw_rage"), resume_vel[1], {"set": ["bs", 1]}, {"set": ["bt", "time"]}], f"{PLAY} && den == 1 && bkind == 1 && bs == 8 && bhp == 1 && time > bt + 1.1")
            continue
        if k == 6:
            r("gloam blinks away after a hit", tick, [become(pose), {"set": ["perch", "(perch + 1 + random(0, 1)) % 3"]}, {"set": ["bs", 4]}, {"set": ["bt", "time"]}], f"{PLAY} && den == 1 && bkind == 6 && bs == 8 && bhp > 0 && time > bt + 1.0")
            continue
        r(f"{b} shakes it off", tick, [become(pose), resume_vel[k], {"set": ["bs", HURT_RESUME[k]]}, {"set": ["bt", "time"]}], f"{PLAY} && den == 1 && bkind == {k} && bs == 8 && bhp > 0 && time > bt + 1.1")
    for k, b in enumerate(BOSSES, start=1):
        r(f"{b} falls", {"event": "bossdown"}, [become(f"{b}_dead"), vel(0, -260)], f"bkind == {k}")
    r("the shard falls free", {"event": "bossdown"}, [{"set": ["bs", 9]}, {"set": ["den", 2]}, {"set": ["bt", "time"]}, {"destroy": "tag:bproj"}, {"destroy": "tag:wasp"}, {"music": "hush"}, {"play": "bossfall"}, {"shake": 8},
                                                      {"spawn": {"prefab": "shard", "at": ["den_c_x", TOP + 90], "vel": [0, -120]}}])
    r("Pip takes the shard", {"touch": ["hero", "shard"]}, [{"destroy": "other"}, {"add": ["shards", 1]}, {"set": ["mode", 6]}, {"set": ["mt", "time"]}, {"play": "shard"}, {"music": "freed"},
                                                          {"burst": {"at": "other", "n": 40, "color": "#fff27a", "speed": [30, 120], "life": [0.5, 1.2], "gravity": -30}}], PLAY)
    for k in range(1, 8):
        r(f"the cocoon opens: friend {k}", {"every": 0.1}, [{"become": {"target": "id:cocoon", "prefab": f"friend{k}"}}, {"play": "pop"},
                                                            {"burst": {"at": ["cocoon_x", "cocoon_y"], "n": 20, "color": "#7b3fa0", "speed": [30, 90], "life": [0.3, 0.7], "gravity": 300}}],
          f"mode == 6 && bkind == {k} && time > mt + 1.4", once=False, cooldown=30)
    r("a friend speaks", {"every": 0.1}, [{"set": ["talked", 1]}, {"set": ["rescue_line", 0]}], f"mode == 6 && time > mt + 2.4 && talked == 0 && bkind < 8")
    r("the last shard", {"every": 0.1}, [{"set": ["talked", 1]}, {"set": ["final_line", 0]}], f"mode == 6 && time > mt + 2.4 && talked == 0 && bkind == 8")
    r("on to the next world", {"event": "rescue_done"}, [{"add": ["stage", 1]}, {"set": ["checkpoint", 0]}, {"set": ["sv_cp", 0]}] + [{"set": ["mode", 1]}, {"set": ["mt", "time"]}, {"set": ["talked", 0]}, {"level": "card"}])
    r("home", {"event": "final_done"}, [{"set": ["mode", 7]}, {"set": ["mt", "time"]}, {"set": ["talked", 0]}, {"set": ["cleared", 1]}, {"set": ["has_save", 0]}, {"save": True}, {"level": "ending"}])
    r("the Queen at the end", {"every": 0.1}, [{"set": ["talked", 1]}, {"set": ["ending_line", 0]}], "mode == 7 && talked == 0 && time > mt + 2.5")
    r("the end: back to the title", {"press": "action"}, [{"restart": True}], "mode == 7 && talked == 2")
    r("the end card", {"event": "ending_done"}, [{"set": ["talked", 2]}, {"set": ["mt", "time"]}])
    return R


def effects():
    setup = [{"set": ["star", 0]}, {"set": ["blink", 0]}, {"set": ["den", 0]}, {"set": ["bs", 0]}, {"set": ["bkind", "floor(stage / 4) + 1"]},
             {"set": ["bmax", " + ".join(f"{hp} * (floor(stage / 4) == {w - 1})" for w, hp in gen.BOSS_HP.items())]}, {"set": ["bhp", "bmax"]}, {"set": ["spike_t", 0]}, {"set": ["talk_until", 0]}]
    return {
        "new_game": {
            "about": "a new journey from the title: the prologue, then 1-1",
            "do": [{"set": ["lives", 5]}, {"set": ["coins", 0]}, {"set": ["form", 0]}, {"set": ["stage", 0]}, {"set": ["shards", 0]}, {"set": ["checkpoint", 0]}, {"set": ["sv_cp", 0]},
                   {"play": "start"}, {"set": ["mode", 8]}, {"set": ["mt", "time"]}, {"set": ["talked", 0]}, {"level": "hive"}],
        },
        "continue_game": {
            "about": "the saved journey: its stage (and midpoint), lives, honey, form and shards",
            "do": [{"set": ["lives", "max(1, sv_lives)"]}, {"set": ["coins", "sv_coins"]}, {"set": ["form", "sv_form"]}, {"set": ["stage", "sv_stage"]}, {"set": ["shards", "sv_shards"]},
                   {"set": ["checkpoint", "sv_cp"]}, {"set": ["cp_x", "sv_cpx"]}, {"set": ["cp_y", "sv_cpy"]}, {"play": "start"}, {"set": ["mode", 1]}, {"set": ["mt", "time"]}, {"set": ["talked", 0]}, {"level": "card"}],
        },
        "save_now": {
            "about": "write the journey to the save file now",
            "do": [{"set": ["has_save", 1]}, {"set": ["sv_stage", "stage"]}, {"set": ["sv_lives", "lives"]}, {"set": ["sv_coins", "coins"]}, {"set": ["sv_form", "form"]}, {"set": ["sv_shards", "shards"]},
                   {"set": ["sv_cp", "checkpoint"]}, {"set": ["sv_cpx", "cp_x"]}, {"set": ["sv_cpy", "cp_y"]}, {"save": True}],
        },
        "stage_setup": {"about": "what every stage starts with: no powers running, the boss of its world rested", "do": setup},
        "gain_honey": {"about": "one honeydrop", "do": [{"add": ["coins", 1]}, {"play": "honey"}, {"emit": "honeycheck"}]},
        "lose_life": {
            "about": "Pip loses a life: dazed, a hop (or none, in a pit), the music stops",
            "params": {"hop": -360},
            "do": [{"set": ["mode", 3]}, {"set": ["dead_at", "time"]}, {"set": ["form", 0]}, {"set": ["star", 0]}, {"set": ["blink", 0]}, {"become": {"target": "id:p", "prefab": "pip_dead"}},
                   {"velocity": {"target": "id:p", "v": [0, "$hop"]}}, {"music": "hush"}, {"play": "lose"}],
        },
    }


# ---- the screen ------------------------------------------------------------------------------------------------------------------------------------
def ui():
    W, H = VIEW_W, VIEW_H
    hud = "mode >= 1 && mode <= 6 && mode != 5"
    play_hud = "mode >= 2 && mode <= 6 && mode != 5"
    pad, keys = "gamepad == 1", "gamepad == 0"
    U = []
    t = lambda text, x, y, show, color="#ffffff", scale=1, align="left", **kw: U.append({"text": text, "at": [x, y], "show": show, "color": color, "scale": scale, "align": align, **kw})
    img = lambda name, x, y, show, scale=1: U.append({"image": name, "at": [x, y], "show": show, "scale": scale})
    shadow = lambda text, x, y, show, color="#ffffff", scale=1, align="left": (t(text, x + 1, y + 1, show, "#2b1a14", scale, align), t(text, x, y, show, color, scale, align))
    # the HUD band: Pip and lives, honey, where we are, the shards won back
    U.append({"panel": {"at": [0, 0], "size": [W, 24], "color": "#1a100a66"}, "show": play_hud})
    img("hud_pip", 8, 6, play_hud)
    shadow("x{lives}", 22, 9, play_hud)
    img("honey", 58, 6, play_hud)
    shadow("{coins:2}", 72, 9, play_hud, "#ffd23f")
    shadow({"pick": "stage_names", "by": "stage"}, W / 2, 9, play_hud, "#fff6e0", align="center")
    shadow("{w}-{s}", W / 2 - 90, 9, play_hud, "#ffd23f", align="right")
    for k in range(8):
        img("hud_shard", W - 96 + k * 11, 6, f"{play_hud} && shards > {k}")
        img("hud_shard_empty", W - 96 + k * 11, 6, f"{play_hud} && shards <= {k}")
    # the boss: its name as it wakes, then its strength
    boss = "mode == 2 && den == 1"
    U.append({"panel": {"at": [W / 2 - 110, 36], "size": [220, 30], "color": "#1e0f22cc"}, "show": f"{boss} && time < intro_until"})
    shadow({"pick": "boss_names", "by": "bkind - 1"}, W / 2, 41, f"{boss} && time < intro_until", "#ff8ad0", align="center")
    shadow({"pick": "boss_titles", "by": "bkind - 1"}, W / 2, 53, f"{boss} && time < intro_until", "#d8c0e8", align="center")
    U.append({"bar": {"var": "bhp", "max": "bmax", "at": [W / 2 - 60, 30], "size": [120, 4], "color": "#ff4fa3", "back": "#3a1e44"}, "show": f"{boss} && time >= intro_until && bs < 9"})
    # the road's folk speak (no buttons to press: a line to read as Pip walks on)
    talk = "mode == 2 && time < talk_until"
    U.append({"panel": {"at": [W / 2 - 170, 34], "size": [340, 38], "color": "#2b1a14dd"}, "show": talk})
    t({"pick": "folk_names", "by": "stage"}, W / 2 - 162, 39, talk, "#ffd23f")
    t({"pick": "folk_lines", "by": "stage"}, W / 2 - 162, 50, talk, "#fff6e0", width=324)
    # the title
    title = "mode == 0 && level == 0"
    shadow("HONEYBOUND", W / 2, 38, title, "#ffc531", scale=4, align="center")
    shadow("PIP AND THE HEARTCOMB", W / 2, 76, title, "#fff6e0", align="center")
    U.append({"panel": {"at": [W / 2 - 140, 104], "size": [280, 94], "color": "#2b1a14cc"}, "show": title})
    items = [("CONTINUE  {sv_w}-{sv_s}", "has_save == 1"), ("A NEW JOURNEY", "1 == 1"), ("FULLSCREEN", "1 == 1"), ("CLOSE GAME", "1 == 1")]
    for k, (label, cond) in enumerate(items):
        y = 110 + k * 14
        t(label, W / 2 - 50, y, f"{title} && {cond} && tsel != {k}", "#c8b8a0")
        t(label, W / 2 - 50, y, f"{title} && {cond} && tsel == {k}", "#ffd23f")
        img("hex", W / 2 - 64, y, f"{title} && tsel == {k}")
    img("btn_dpad", W / 2 - 92, 182, f"{title} && {pad}")
    t("CHOOSE", W / 2 - 78, 184, f"{title} && {pad}", "#fff6e0")
    img("btn_a", W / 2 + 10, 182, f"{title} && {pad}")
    t("SELECT", W / 2 + 24, 184, f"{title} && {pad}", "#fff6e0")
    t("ARROWS CHOOSE   SPACE SELECT   F FULLSCREEN", W / 2, 184, f"{title} && {keys}", "#fff6e0", align="center")
    t("SHARDS WON BACK {sv_shards}/8", W / 2, 252, f"{title} && has_save == 1", "#ffd23f", align="center")
    t("THE HIVE IS WARM. THANK YOU, PIP.", W / 2, 252, f"{title} && has_save == 0 && cleared == 1", "#ffd23f", align="center")
    # the card before a stage
    card = "mode == 1"
    shadow("WORLD {w}-{s}", W / 2, 92, card, "#ffd23f", scale=2, align="center")
    shadow({"pick": "stage_names", "by": "stage"}, W / 2, 118, card, "#fff6e0", align="center")
    t("x  {lives}", W / 2 - 16, 157, card)
    t({"pick": "world_lines", "by": "w - 1"}, W / 2, 200, f"{card} && s == 1", "#c8b8a0", align="center", width=380)
    for k in range(8):
        img("hud_shard", W / 2 - 44 + k * 11, 228, f"{card} && shards > {k}")
        img("hud_shard_empty", W / 2 - 44 + k * 11, 228, f"{card} && shards <= {k}")
    # how to play, on the first stage's card
    first = f"{card} && stage == 0"
    img("btn_dpad", W / 2 - 150, 176, f"{first} && {pad}")
    t("MOVE", W / 2 - 136, 178, f"{first} && {pad}")
    img("btn_a", W / 2 - 96, 176, f"{first} && {pad}")
    t("JUMP (HOLD: FLUTTER)", W / 2 - 82, 178, f"{first} && {pad}")
    img("btn_x", W / 2 + 46, 176, f"{first} && {pad}")
    t("RUN", W / 2 + 60, 178, f"{first} && {pad}")
    img("btn_start", W / 2 + 90, 177, f"{first} && {pad}")
    t("MENU", W / 2 + 108, 178, f"{first} && {pad}")
    t("ARROWS MOVE   Z JUMP (HOLD: FLUTTER)   X RUN   ESC MENU", W / 2, 178, f"{first} && {keys}", align="center")
    # game over: a clean message, nothing else
    t("GAME OVER", W / 2, 118, "mode == 5", "#ff8a7a", scale=3, align="center")
    t("THE JOURNEY WILL GO ON FROM WORLD {w}-{s}", W / 2, 152, "mode == 5 && time > mt + 1", "#c8b8a0", align="center")
    # the pause menu
    m = "menu == 1"
    U.append({"panel": {"at": [0, 0], "size": [W, H], "color": "#100a0699"}, "show": m})
    U.append({"panel": {"at": [W / 2 - 110, 46], "size": [220, 198], "color": "#2b1a14ee"}, "show": m})
    U.append({"panel": {"at": [W / 2 - 110, 46], "size": [220, 2], "color": "#ffc531"}, "show": m})
    t("PAUSED", W / 2, 54, m, "#ffc531", scale=2, align="center")
    labels = ["RESUME", "SAVE GAME", "SAVE THIS SONG", "MUSIC: ON", "FULLSCREEN", "QUIT TO TITLE", "CLOSE GAME"]
    for k, label in enumerate(labels):
        y = 78 + k * 15
        extra = " && music_on == 1" if k == 3 else ""
        t(label, W / 2 - 70, y, f"{m} && msel != {k}{extra}", "#c8b8a0")
        t(label, W / 2 - 70, y, f"{m} && msel == {k}{extra}", "#ffd23f")
        if k == 3:
            t("MUSIC: OFF", W / 2 - 70, y, f"{m} && msel != 3 && music_on == 0", "#c8b8a0")
            t("MUSIC: OFF", W / 2 - 70, y, f"{m} && msel == 3 && music_on == 0", "#ffd23f")
        img("hex", W / 2 - 84, y, f"{m} && msel == {k}")
    img("note", W / 2 - 100, 186, m)
    t("{music_title}", W / 2 - 86, 187, m, "#ffe8a0")
    t("SAVED: WORLD {w}-{s}", W / 2, 202, f"{m} && msg == 1 && tick < msg_tick + 180", "#9dff5c", align="center")
    t("SONG SAVED TO YOUR MUSIC FOLDER", W / 2, 202, f"{m} && msg == 2 && tick < msg_tick + 240", "#9dff5c", align="center")
    t("THE SONG COULD NOT BE SAVED", W / 2, 202, f"{m} && msg == 3 && tick < msg_tick + 240", "#ff8a7a", align="center")
    img("btn_a", W / 2 - 92, 228, f"{m} && {pad}")
    t("SELECT", W / 2 - 78, 230, f"{m} && {pad}")
    img("btn_b", W / 2 - 26, 228, f"{m} && {pad}")
    t("BACK", W / 2 - 12, 230, f"{m} && {pad}")
    img("btn_dpad", W / 2 + 28, 228, f"{m} && {pad}")
    t("CHOOSE", W / 2 + 42, 230, f"{m} && {pad}")
    t("ARROWS CHOOSE   SPACE SELECT   ESC BACK", W / 2, 230, f"{m} && {keys}", align="center")
    # the end
    end = "mode == 7 && talked == 2"
    shadow("THE END", W / 2, 50, end, "#ffd23f", scale=3, align="center")
    shadow("THE HEARTCOMB IS WHOLE AND THE HIVE IS WARM.", W / 2, 84, end, "#fff6e0", align="center")
    shadow("AND PIP CAN FLY.", W / 2, 96, end, "#fff6e0", align="center")
    img("btn_a", W / 2 - 40, 112, f"{end} && {pad} && time > mt + 2")
    shadow("TITLE", W / 2 - 26, 114, f"{end} && {pad} && time > mt + 2")
    shadow("SPACE: TITLE", W / 2, 114, f"{end} && {keys} && time > mt + 2", align="center")
    # the conversations (A moves them on)
    box = {"at": [40, 34], "size": [400, 72], "pause": True, "color": "#fff6e0", "back": "#2b1a14ee", "accent": "#ffc531"}
    U.append({"dialogue": {"id": "prologue", **box, "done": "prologue_done", "lines": story.prologue()}})
    U.append({"dialogue": {"id": "rescue", **box, "done": "rescue_done", "lines": [{"text": {"pick": f"rescue_{k}", "by": "bkind - 1"}} for k in range(1, 4)]}})
    U.append({"dialogue": {"id": "final", **box, "done": "final_done", "lines": story.final()}})
    U.append({"dialogue": {"id": "ending", **box, "done": "ending_done", "lines": story.ending()}})
    for w in U:
        tx = w.get("text")
        if isinstance(tx, str) and "{" not in tx:
            width = len(tx) * 6 * w.get("scale", 1)
            assert width <= W - 6, f"`{tx}` is {width} px wide: the screen is {W}"
    return U


# ---- checks: scenarios the engine plays headless -------------------------------------------------------------------------------------------
MECHANICS = [
    {"key": "stomp", "name": "landing on a grubble flattens it and bounces Pip up", "level": "1-1", "goal": "stomps >= 1", "progress": "stomps * 4000 + p_x", "tail": [{"wait": 0.3}],
     "expect": [{"sound": "stomp"}, {"var": "mode", "eq": 2}, {"var": "lives", "eq": 5}, {"var": "stomps", "gte": 1}]},
    {"key": "power", "name": "bumping a comb from below brings royal jelly, and eating it grows Pip", "level": "1-1", "goal": "count_jelly >= 1", "progress": "count_jelly * 4000 - abs(p_x - 376)",
     "tail": [{"hold_until": ["right"], "until": {"var": "form", "eq": 1}, "timeout": 8}, {"wait": 0.3}],
     "expect": [{"var": "form", "eq": 1}, {"sound": "sprout"}, {"sound": "powerup"}]},
    {"key": "fire", "name": "a second comb gives grown Pip a firebloom: pollen puffs to throw", "level": "1-1", "goal": "form == 2",
     "progress": "form * 4000 + count_firebloom * 1500 + count_jelly * 400 + min(p_x, 400 + form * 700) - (count_firebloom > 0) * (abs(p_x - 1032) + abs(p_y - 160))",
     "tail": [{"hold": ["secondary"], "seconds": 0.1}, {"wait": 0.2}], "expect": [{"var": "form", "eq": 2}, {"sound": "seed"}, {"sound": "powerup", "min": 2}]},
    {"key": "star", "name": "a wax block hides a sunpetal: sun power plays its own tune", "level": "1-1", "goal": "star == 1", "progress": "star * 4000 + count_starpetal * 400 + p_x", "tail": [{"wait": 0.5}],
     "expect": [{"var": "star", "eq": 1}, {"sound": "sprout"}]},
    {"key": "shell", "name": "a stomped snail hides in its shell, and a kick sends the shell rolling", "level": "1-1", "goal": "kicks >= 1", "progress": "kicks * 4000 + count_shell * 400 + p_x", "tail": [{"wait": 0.5}],
     "expect": [{"var": "kicks", "eq": 1}, {"sound": "kick"}, {"count": "kshell", "gte": 1}]},
    {"key": "log", "name": "holding down on a hollow stump leads to the honey room, and its side stump comes out further along", "level": "1-1",
     "goal": f"level == {IX['1-1b']} && coins >= 10", "progress": f"(level == {IX['1-1']}) * (2000 - abs(p_x - 744) - abs(p_y - 167)) + (level == {IX['1-1b']}) * (10000 + coins * 100 + p_x)",
     "tail": [{"hold_until": ["right"], "until": {"var": "level", "eq": IX["1-1"]}, "timeout": 8}, {"wait": 0.5}], "expect": [{"var": "coins", "gte": 10}, {"var": "p_x", "gt": 2500}, {"sound": "pipe", "min": 2}, {"var": "mode", "eq": 2}]},
    {"key": "midpoint", "name": "after the midpoint a lost life starts Pip there, not at the beginning", "level": "1-1", "goal": "checkpoint == 1",
     "tail": [{"hold_until": ["right"], "until": {"var": "mode", "eq": 3}, "timeout": 40}, {"wait_until": {"var": "mode", "eq": 2}, "timeout": 10}, {"wait": 0.2}],
     "expect": [{"var": "lives", "eq": 4}, {"var": "p_x", "gte": 1500}, {"var": "checkpoint", "eq": 1}, {"var": "sv_cp", "eq": 1}]},
]


def scenarios():
    S = []
    sols = json.loads(SOLUTIONS.read_text()) if SOLUTIONS.exists() else {}
    S.append({
        "name": "the title starts a new journey: the Queen's call, then the card, then Pip plays 1-1",
        "max_seconds": 30,
        "script": [{"wait": 0.5}, {"press": "down"}, {"press": "up"}, {"press": "action"}, {"wait_until": {"var": "prologue_line", "eq": 0}, "timeout": 5}]
                  + [{"press": "action"}, {"wait": 0.2}] * len(story.prologue()) + [{"wait_until": {"var": "mode", "eq": 2}, "timeout": 6}, {"hold": ["right"], "seconds": 1}],
        "expect": [{"var": "level", "eq": IX["1-1"]}, {"var": "lives", "eq": 5}, {"var": "p_x", "gt": 60}, {"sound": "start"}, {"var": "has_save", "eq": 1}, {"var": "sv_stage", "eq": 0}],
    })
    for m in MECHANICS:
        if m["key"] in sols:
            S.append({"name": m["name"], "level": m["level"], "max_seconds": 120, "script": sols[m["key"]] + m["tail"], "expect": m["expect"]})
    S.append({
        "name": "holding jump while falling flutters: Pip falls slower than without it",
        "level": "1-1",
        "max_seconds": 10,
        "script": [{"wait": 0.3}, {"hold": ["action"], "seconds": 0.95}],
        "expect": [{"var": "mode", "eq": 2}, {"var": "p_y", "lt": TOP + 13 * T - 7 - 20}],
    })
    S.append({
        "name": "a pit costs a life and the card comes back",
        "level": "1-1",
        "max_seconds": 30,
        "script": [{"hold_until": ["right"], "until": {"var": "mode", "eq": 3}, "timeout": 20}, {"wait_until": {"var": "mode", "eq": 1}, "timeout": 6}],
        "expect": [{"var": "lives", "eq": 4}, {"sound": "lose"}, {"var": "level", "eq": IX["card"]}],
    })
    lose_one = [{"hold_until": ["right"], "until": {"var": "mode", "eq": 3}, "timeout": 30}, {"wait_until": {"var": "mode", "ne": 3}, "timeout": 6}, {"wait_until": {"var": "mode", "ne": 1}, "timeout": 6}]
    skip_prologue = [{"wait_until": {"var": "prologue_line", "eq": 0}, "timeout": 5}] + [{"press": "action"}, {"wait": 0.2}] * len(story.prologue())
    S.append({
        "name": "losing the last life is a clean game over, then the title offers to continue",
        "max_seconds": 200,
        "script": [{"wait": 0.3}, {"press": "action"}] + skip_prologue + [{"wait_until": {"var": "mode", "eq": 2}, "timeout": 6}] + lose_one * 5
                  + [{"wait_until": {"var": "mode", "eq": 5}, "timeout": 8}, {"wait_until": {"var": "mode", "eq": 0}, "timeout": 8}, {"wait": 0.5}],
        "expect": [{"var": "level", "eq": 0}, {"sound": "gameover"}, {"var": "has_save", "eq": 1}, {"var": "sv_lives", "eq": 5}, {"var": "tsel", "eq": 0}],
    })
    S.append({
        "name": "the pause menu stands the world still, saves the game and resumes",
        "level": "1-1",
        "max_seconds": 20,
        "script": [{"hold": ["right"], "seconds": 0.5}, {"press": "pause"}, {"hold": ["right"], "seconds": 1.0}, {"press": "down"}, {"press": "action"}, {"wait": 0.2},
                   {"press": "up"}, {"press": "action"}, {"hold": ["right"], "seconds": 0.2}],
        "expect": [{"var": "p_x", "lt": 140}, {"sound": "pause"}, {"sound": "saved"}, {"var": "menu", "eq": 0}, {"var": "has_save", "eq": 1}, {"var": "sv_stage", "eq": 0}],
    })
    S.append({
        "name": "saving, quitting to the title and continuing comes back to the same stage",
        "level": "2-1",
        "max_seconds": 30,
        "script": [{"hold": ["right"], "seconds": 0.5}, {"press": "pause"}, {"press": "down"}, {"press": "action"}, {"press": "down"}, {"press": "down"}, {"press": "down"}, {"press": "down"},
                   {"press": "action"}, {"wait": 0.5}, {"press": "action"}, {"wait_until": {"var": "mode", "eq": 2}, "timeout": 6}],
        "expect": [{"var": "stage", "eq": 4}, {"var": "level", "eq": IX["2-1"]}, {"var": "mode", "eq": 2}],
    })
    S.append({
        "name": "a roadside bug says a line when Pip bumps into them",
        "level": "1-1",
        "max_seconds": 10,
        "script": [{"hold": ["right"], "seconds": 1.2}],
        "expect": [{"sound": "talk"}, {"var": "talk_until", "gt": 0}],
    })
    for i, L in enumerate(STAGE_LEVELS):
        keep = L.theme == "keep"
        last = i == N_STAGES - 1
        if keep:
            n_lines = len(story.final()) if last else 3
            tail = [{"wait_until": {"var": "talked", "eq": 1}, "timeout": 10}, {"wait": 0.2}] + [{"press": "action"}, {"wait": 0.3}] * n_lines + [{"wait": 0.5}]
            expect = [{"sound": "shard"}, {"var": "shards", "eq": 1}, {"var": "den", "eq": 2}] + ([{"var": "mode", "eq": 7}, {"var": "level", "eq": IX["ending"]}] if last else [{"var": "mode", "eq": 1}, {"var": "stage", "eq": i + 1}])
        else:
            tail = [{"wait_until": {"var": "mode", "eq": 1}, "timeout": 30}]
            expect = [{"var": "mode", "eq": 1}, {"var": "level", "eq": IX["card"]}, {"var": "stage", "eq": i + 1}, {"sound": "bloom"}]
        if L.id not in sols:
            continue  # `dev.py solve` finds its run
        S.append({
            "name": f"{L.id} {L.name.lower()} can be finished",
            "level": L.id,
            "max_seconds": 300,
            **({"smoke": True} if L.id == "1-1" else {}),
            "script": sols[L.id] + tail,
            "expect": expect + [{"var": "lives", "eq": 5}],
        })
    return S


def gate_tall():
    g = art.all_sprites()["gate"]
    return {"rows": g["rows"] * 3, "palette": g["palette"]}


def build():
    sprites = art.all_sprites()
    sprites["gate_tall"] = gate_tall()
    P = prefabs()
    songs = audio.music()
    game = {
        "game2d": 1,
        "id": "honeybound",
        "title": "Honeybound",
        "description": "A side-scrolling platformer: Pip, the smallest bee of the Great Hive, walks eight worlds to win the Heartcomb back from Baron Thornwick before the first snow.",
        "capabilities": {"presentation": "2d", "platforms": ["windows", "linux"], "networking": "offline", "input": ["keyboard", "gamepad"], "persistence": ["settings", "progress"]},
        "view": {"width": VIEW_W, "height": VIEW_H, "background": "#69b8f0", "scale": "integer", "camera": {"follow": "pip", "lerp": 0.25, "forward_only": False, "min_x": "(den >= 1) * (den_l_x - 8)"}},
        "icon": "icon",
        "pause_if": "menu > 0",
        "sprites": sprites,
        "sounds": audio.sounds(),
        "music": songs,
        "tables": story.tables(STAGE_LEVELS),
        "vars": {
            "mode": 0, "lives": 5, "coins": 0, "form": 0, "star": 0, "blink": 0, "inv_until": 0, "star_until": 0,
            "stage": 0, "checkpoint": 0, "cp_x": 0, "cp_y": 0, "mt": 0, "dead_at": 0, "w": 1, "s": 1, "talked": 0, "stomps": 0, "kicks": 0, "shards": 0,
            "menu": 0, "msel": 0, "menu_tick": 0, "msg": 0, "msg_tick": 0, "tsel": 1, "talk_until": 0,
            "den": 0, "bs": 0, "bt": 0, "bhp": 3, "bmax": 3, "bkind": 1, "intro_until": 0, "perch": 2, "spike_x": 0, "spike_t": 0,
            "has_save": 0, "sv_stage": 0, "sv_lives": 5, "sv_coins": 0, "sv_form": 0, "sv_shards": 0, "sv_cp": 0, "sv_cpx": 0, "sv_cpy": 0, "sv_w": 1, "sv_s": 1, "best": 0, "cleared": 0,
        },
        "persist": ["has_save", "sv_stage", "sv_lives", "sv_coins", "sv_form", "sv_shards", "sv_cp", "sv_cpx", "sv_cpy", "best", "cleared"],
        "prefabs": P,
        "levels": [title_level(), card_level(), hive_level(), gameover_level()] + [level_json(L) for L in STAGE_LEVELS] + [level_json(L) for L in BONUS_LEVELS] + [ending_level()],
        "ui": ui(),
        "effects": effects(),
        "rules": rules(),
        "checks": {"scenarios": scenarios()},
    }
    # the title's "CONTINUE 3-2": the saved stage as world and stage numbers
    game["rules"].insert(0, {"id": "the saved stage's numbers", "when": {"every": 0.1}, "do": [{"set": ["sv_w", "floor(sv_stage / 4) + 1"]}, {"set": ["sv_s", "sv_stage % 4 + 1"]}]})
    for lv in game["levels"]:
        for item in lv.get("scene", []):
            assert item["prefab"] in P, (lv["id"], item["prefab"])
    return game



def main():
    game = build()
    text = json.dumps(game, separators=(",", ":"))
    if len(sys.argv) > 1 and sys.argv[1] == "stats":
        for lv in game["levels"]:
            tiles = sum(1 for row in lv.get("map", {}).get("rows", []) for ch in row if ch != ".")
            print(f"{lv['id']:>8}: {tiles} tiles, {len(lv.get('scene', []))} placed")
        print(f"{len(game['prefabs'])} prefabs, {len(game['rules'])} rules, {len(game['sprites'])} sprites, {len(game['music'])} songs, {len(text) // 1024} KB")
        return
    OUT.write_text(json.dumps(game, indent=1) + "\n")
    print(f"wrote {OUT} ({len(text) // 1024} KB compact)")


if __name__ == "__main__":
    main()
