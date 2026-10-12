#!/usr/bin/env python3
"""Honeybound builder: writes ../honeybound.game2d.json (a RedEngine 2D game) from art.py, audio.py and levels.py.

  python3 build.py          write the game
  python3 build.py stats    sizes: levels, entities, rules

Tobin, a young beekeeper, runs and jumps through eight stages in two worlds to free the Great Hive's friends from Baron Thornwick.
Everything that happens is data: prefabs, a tile map per level, and rules (`red_engine2 describe 2d`). The game's flow is one variable,
`mode`: 0 title, 1 the card before a stage, 2 playing, 3 Tobin lost a life, 4 the beacon pole was reached, 5 game over, 6 the Baron is
beaten (a friend speaks), 7 the end.
"""
import json, sys
from pathlib import Path

import art, audio, levels
from levels import T, cx, cy

HERE = Path(__file__).resolve().parent
OUT = HERE.parent / "honeybound.game2d.json"
SOLUTIONS = HERE / "solutions.json"

VIEW_W, VIEW_H = 256, 240
STAGE_LEVELS = [f() for f in levels.STAGES]
BONUS_LEVELS = [levels.BONUS[s.id]() for s in STAGE_LEVELS if s.id in levels.BONUS]
LEVEL_IDS = ["title", "card"] + [s.id for s in STAGE_LEVELS] + [b.id for b in BONUS_LEVELS] + ["ending"]
IX = {lid: i for i, lid in enumerate(LEVEL_IDS)}
CASTLES = [i for i, s in enumerate(STAGE_LEVELS) if s.theme == "castle"]

# ---- how Tobin moves --------------------------------------------------------------------------------------------------------------------------
FEEL = {"mode": "platformer", "speed": 90, "run": 150, "jump": 370, "run_jump": 40, "accel": 340, "decel": 420, "cut": 0.45, "coyote": 0.08, "buffer": 0.12}
BODY = {"type": "dynamic", "gravity": 1000, "max_fall": 360}
SMALL, BIG = [12, 14], [12, 28]


def hero(sprite, size, tags, idle=True, air=True):
    shape = {"sprite": sprite, "flip": "auto"}
    if idle:
        shape["idle"] = sprite + "_idle"
    if air:
        shape["air"] = sprite + "_air"
    return {"tag": tags, "shape": shape, "size": size, "layer": 5, "body": BODY, "collide": ["solid"], "move": {"keys": FEEL}}


def prefabs():
    P = {}
    # Tobin in every form. "tobin" is everything the camera follows; "hero" is Tobin while he can act; "vuln" can be hurt.
    P["tobin_s"] = hero("tobin", SMALL, ["tobin", "hero", "vuln"])
    P["tobin_b"] = hero("tobinbig", BIG, ["tobin", "hero", "vuln"])
    P["tobin_f"] = hero("emberbig", BIG, ["tobin", "hero", "vuln"])
    P["tobin_s_star"] = hero("tobin_star", SMALL, ["tobin", "hero", "star"], idle=False, air=False)
    P["tobin_b_star"] = hero("tobinbig_star", BIG, ["tobin", "hero", "star"], idle=False, air=False)
    P["tobin_f_star"] = hero("tobinbig_star", BIG, ["tobin", "hero", "star"], idle=False, air=False)
    P["tobin_s_blink"] = hero("tobin_blink", SMALL, ["tobin", "hero"], idle=False, air=False)
    walk = {"walk": {"speed": 70, "dir": "right"}}
    for k, (spr, size) in {"s": ("tobin", SMALL), "b": ("tobinbig", BIG), "f": ("emberbig", BIG)}.items():
        P[f"walk_{k}"] = {"tag": ["tobin", "walker"], "shape": {"sprite": spr, "flip": "auto", "air": spr + "_air"}, "size": size, "layer": 5, "body": BODY, "collide": ["solid"], "move": walk}
    P["tobin_dead"] = {"tag": "ghost", "shape": {"sprite": "tobin_dead"}, "size": SMALL, "layer": 7, "body": {"type": "dynamic", "gravity": 900, "max_fall": 400}}
    P["seed"] = {"tag": "seed", "shape": {"sprite": "seed"}, "size": [8, 8], "layer": 6, "body": {"type": "dynamic", "gravity": 1000, "bounce": 1.0, "max_fall": 300}, "collide": ["solid"], "ttl": 2.5}

    # foes: each kind has its own tag (rules say what stomping, kicking or burning it does)
    def foe(kind, sprite, size, tags, mv, body=True, layer=3, asleep=True):
        p = {"tag": [kind] + tags, "shape": {"sprite": sprite, "flip": "auto"}, "size": size, "layer": layer, "move": mv, "asleep": asleep}
        if body:
            p["body"] = {"type": "dynamic", "gravity": 1000, "max_fall": 360}
            p["collide"] = ["solid", "foe"]
        else:
            p["body"] = {"type": "dynamic"}
        return p

    P["grubble"] = foe("grubble", "grubble", [14, 14], ["foe", "enemy", "stompable"], {"walk": {"speed": 28}})
    P["dgrubble"] = foe("dgrubble", "dgrubble", [14, 14], ["foe", "enemy", "stompable"], {"walk": {"speed": 36}})
    P["snail"] = foe("snail", "snail", [14, 13], ["foe", "enemy", "stompable"], {"walk": {"speed": 22, "edges": "turn"}})
    P["spikebug"] = foe("spikebug", "spikebug", [14, 12], ["foe", "enemy", "spiky"], {"walk": {"speed": 26}})
    P["wasp"] = foe("wasp", "wasp", [14, 12], ["enemy", "stompable"], {"patrol": {"axis": "y", "range": 28, "speed": 30}}, body=False)
    P["waspx"] = foe("wasp", "wasp", [14, 12], ["enemy", "stompable"], {"patrol": {"axis": "x", "range": 40, "speed": 34}}, body=False)
    P["snapvine"] = foe("snapvine", "snapvine", [12, 22], ["enemy", "spiky"], {"patrol": {"axis": "y", "range": 12, "speed": 14}}, body=False, layer=1)
    P["shell"] = {"tag": ["shell"], "shape": {"sprite": "shell"}, "size": [14, 12], "layer": 3, "body": {"type": "dynamic", "gravity": 1000, "max_fall": 360}, "collide": ["solid"]}
    P["kshell"] = {"tag": ["kshell", "foe"], "shape": {"sprite": "shell_spin"}, "size": [14, 12], "layer": 3, "body": {"type": "dynamic", "gravity": 1000, "max_fall": 360}, "collide": ["solid"], "move": {"walk": {"speed": 230}}}
    for kind in ("grubble", "dgrubble"):
        P[f"{kind}_flat"] = {"tag": "corpse", "shape": {"sprite": f"{kind}_flat"}, "size": [14, 14], "layer": 2, "ttl": 0.6}
    for kind, sprite in (("grubble", "grubble_dead"), ("dgrubble", "dgrubble_dead"), ("snail", "snail_dead"), ("spikebug", "spikebug_dead"), ("wasp", "wasp_dead"), ("baron", "baron_dead")):
        P[f"{kind}_dead"] = {"tag": "corpse", "shape": {"sprite": sprite}, "size": [14, 14] if kind != "baron" else [28, 28], "layer": 6, "body": {"type": "dynamic", "gravity": 900, "max_fall": 400}, "ttl": 2.5}
    P["snapvine_dead"] = {"tag": "corpse", "shape": {"sprite": "snapvine"}, "size": [12, 22], "layer": 6, "body": {"type": "dynamic", "gravity": 900, "max_fall": 400}, "ttl": 1.5}
    P["cinder"] = {"tag": ["hazard"], "shape": {"sprite": "cinder"}, "size": [10, 10], "layer": 4, "move": {"patrol": {"axis": "y", "range": 64, "speed": 90}}, "body": {"type": "dynamic"}, "asleep": True}
    P["cinder_hi"] = {"tag": ["hazard"], "shape": {"sprite": "cinder"}, "size": [10, 10], "layer": 4, "move": {"patrol": {"axis": "y", "range": 104, "speed": 120}}, "body": {"type": "dynamic"}, "asleep": True}
    P["breath"] = {"tag": ["hazard"], "shape": {"sprite": "breath", "flip": "x"}, "size": [14, 6], "layer": 6, "body": {"type": "dynamic"}, "ttl": 4}
    P["baron"] = {"tag": ["baron", "hazard"], "shape": {"sprite": "baron", "flip": "auto"}, "size": [28, 28], "layer": 4, "body": {"type": "dynamic", "gravity": 900, "max_fall": 400}, "collide": ["solid", "bosswall"], "move": {"walk": {"speed": 26}}, "asleep": True}

    # items
    P["honey"] = {"tag": "honey", "shape": {"sprite": "honey"}, "size": [10, 12], "layer": 2}
    P["coinpop"] = {"tag": "fx", "shape": {"sprite": "honey_spin"}, "size": [10, 12], "layer": 6, "body": {"type": "dynamic", "gravity": 900}, "ttl": 0.45}
    P["jelly"] = {"tag": ["jelly", "item"], "shape": {"sprite": "jelly"}, "size": [14, 14], "layer": 2, "body": BODY, "collide": ["solid"], "move": {"walk": {"speed": 50, "dir": "right"}}}
    P["firebloom"] = {"tag": ["firebloom", "item"], "shape": {"sprite": "firebloom"}, "size": [14, 14], "layer": 2}
    P["starpetal"] = {"tag": ["starpetal", "item"], "shape": {"sprite": "starpetal"}, "size": [14, 14], "layer": 2, "body": BODY, "collide": ["solid"], "move": {"walk": {"speed": 64, "dir": "right"}}}
    P["clover"] = {"tag": ["clover", "item"], "shape": {"sprite": "clover"}, "size": [14, 14], "layer": 2, "body": BODY, "collide": ["solid"], "move": {"walk": {"speed": 50, "dir": "right"}}}
    P["bumper"] = {"tag": "bumper", "shape": {"rect": [14, 8], "color": "#ffffff"}, "size": [14, 8], "hidden": True, "ttl": 0.1}
    for n in ("100", "200", "400", "500", "1000", "1UP", "5000"):
        P[f"pop{n}"] = {"tag": "fx", "shape": {"text": n, "color": "#ffffff"}, "layer": 8, "move": {"drift": [0, -36]}, "body": {"type": "dynamic"}, "ttl": 0.7}

    # world pieces (per theme where the look changes)
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
    P["bridge"] = {"tag": ["solid", "bridge"], "shape": {"sprite": "bridge"}, "size": [16, 6], "body": {"type": "static"}}
    P["bosswall"] = {"tag": "bosswall", "shape": {"rect": [16, 16], "color": "#ff00ff"}, "hidden": True}
    P["edge"] = {"tag": "solid", "shape": {"rect": [16, 16], "color": "#000000"}, "hidden": True, "body": {"type": "static"}}
    P["abyss"] = {"tag": "abyss", "shape": {"rect": [16, 16], "color": "#000000"}, "hidden": True}
    P["lever"] = {"tag": "lever", "shape": {"sprite": "lever"}, "layer": 2}
    P["pole"] = {"tag": "pole", "shape": {"sprite": "pole"}, "size": [4, 16], "layer": 1}
    P["pole_top"] = {"tag": "pole", "shape": {"sprite": "pole_top"}, "size": [4, 16], "layer": 1}
    P["banner"] = {"tag": "decor", "shape": {"sprite": "banner"}, "layer": 2}
    P["hut"] = {"tag": "hut", "shape": {"sprite": "hut"}, "size": [10, 40], "layer": -1}
    P["midpoint"] = {"tag": "midpoint", "shape": {"rect": [8, 8], "color": "#ffffff"}, "size": [8, 480], "hidden": True}
    P["logmouth"] = {"tag": "logmouth", "shape": {"rect": [10, 10], "color": "#ffffff"}, "size": [10, 10], "hidden": True}
    P["logside"] = {"tag": "logside", "shape": {"sprite": "log_side"}, "size": [16, 16], "layer": 2}
    for d in ("cloud", "dcloud", "bush", "hill", "flowers", "torch", "crystal", "stars"):
        P[d] = {"tag": "decor", "shape": {"sprite": d}, "layer": -5}
    P["buzz"] = {"tag": "friend", "shape": {"sprite": "buzz"}, "layer": 3}
    P["queen"] = {"tag": "friend", "shape": {"sprite": "queen"}, "layer": 3}
    P["tobin_pose"] = {"tag": "decor", "shape": {"sprite": "tobin_idle"}, "layer": 3}
    P["tobin_pose_big"] = {"tag": "decor", "shape": {"sprite": "tobinbig_idle"}, "layer": 3}

    # one prefab per kind of lift and firebar ember the levels use
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
        "=": f"{theme}_top" if theme not in ("castle",) else f"{theme}_stone",
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
        "_": "bridge",
        "Z": "bosswall",
        "A": "abyss",
        "o": "honey",
        "g": "grubble",
        "d": "dgrubble",
        "k": "snail",
        "s": "spikebug",
        "I": "pole",
        "!": "pole_top",
    }


def level_json(L, start_prefab="tobin_s"):
    rows = L.rows()
    used = set("".join(rows)) - {"."}
    lg = {k: v for k, v in legend(L.theme).items() if k in used}
    scene = []
    if start_prefab:
        sc, sr = L.start
        scene.append({"prefab": start_prefab, "at": [cx(sc), (sr + 1) * T - SMALL[1] / 2], "id": "p"})
    solid = set("=#WBRXU?PL[]{}()-_")
    for prefab, x, y, sid in L.scene:
        if prefab == "midpoint":
            # the respawn point stands on the first solid tile of its column (the sensor is tall, so passing it anywhere counts)
            col = int(x // T)
            top = next((r for r in range(3, levels.ROWS) if L.grid[r][col] in solid and L.grid[r - 1][col] not in solid), 13)
            y = top * T - SMALL[1] / 2
        item = {"prefab": prefab, "at": [x, y]}
        if sid:
            item["id"] = sid
        scene.append(item)
    # an invisible wall at the right end of the world
    for r in range(levels.ROWS):
        scene.append({"prefab": "edge", "at": [L.w * T + 8, cy(r)]})
    return {"id": L.id, "scene": scene, "map": {"tile": T, "rows": rows, "legend": lg}, "world": [L.w * T, VIEW_H], "background": L.background, "music": L.music}


def title_level():
    L = levels.Level("title", "", 16, "meadow", "#6bb4ff", "meadow")
    L.ground(0, 15)
    L.text(2, 9, "?B?")
    L.add("hill", cx(11), 13 * T - 16)
    L.add("bush", cx(4), 13 * T - 8)
    L.add("cloud", cx(3), cy(3))
    L.add("cloud", cx(13), cy(2))
    L.add("flowers", cx(8), 13 * T - 8)
    L.add("tobin_pose", cx(3), 13 * T - 8)
    L.add("grubble", cx(13), cy(12))
    j = level_json(L, start_prefab=None)
    j["map"]["rows"] = j["map"]["rows"][:-1]
    j["map"]["legend"].pop("A", None)
    return j


def card_level():
    return {"id": "card", "scene": [{"prefab": "tobin_pose", "at": [104, 128]}], "world": [VIEW_W, VIEW_H], "background": "#000000", "music": "hush"}


def ending_level():
    L = levels.Level("ending", "", 16, "meadow", "#ffcf8a", "ending")
    L.ground(0, 15)
    L.add("hut", cx(12), 13 * T - 24)
    L.add("hill", cx(3), 13 * T - 16)
    L.add("cloud", cx(5), cy(2))
    L.add("flowers", cx(9), 13 * T - 8)
    L.add("tobin_pose_big", cx(5), 13 * T - 16)
    L.add("queen", cx(7), 13 * T - 12)
    L.add("buzz", cx(9), cy(9))
    j = level_json(L, start_prefab=None)
    j["map"]["rows"] = j["map"]["rows"][:-1]
    j["map"]["legend"].pop("A", None)
    return j


# ---- rules -------------------------------------------------------------------------------------------------------------------------------------

PLAY = "mode == 2"
KINDS = {  # what stomping, kicking or burning each kind of foe does
    "grubble": {"stomp": "grubble_flat", "dead": "grubble_dead", "score": 100},
    "dgrubble": {"stomp": "dgrubble_flat", "dead": "dgrubble_dead", "score": 100},
    "snail": {"stomp": "shell", "dead": "snail_dead", "score": 100},
    "wasp": {"stomp": "wasp_dead", "dead": "wasp_dead", "score": 200},
    "spikebug": {"stomp": None, "dead": "spikebug_dead", "score": 200},
    "snapvine": {"stomp": None, "dead": "snapvine_dead", "score": 200},
}


def pop(n, at="other"):
    return {"spawn": {"prefab": f"pop{n}", "at": at}}


def rules():
    R = []
    r = lambda rid, when, do, cond=None, **kw: R.append({"id": rid, "when": when, **({"if": cond} if cond else {}), **kw, "do": do})
    goto_card = [{"set": ["mode", 1]}, {"set": ["mode_t", "time"]}, {"set": ["talked", 0]}, {"level": "card"}]

    # --- the title and the start of a game
    r("close the pause box", {"start": True}, [{"set": ["pausebox_line", 99]}, {"set": ["buzz_line", 99]}, {"set": ["queen_line", 99]}])
    r("start a game", {"press": "action"}, [{"apply": "new_game"}], "mode == 0 && level == 0")
    r("a stage played directly starts as if from its card", {"every": 0.05}, [{"set": ["mode", 2]}, {"set": ["time_left", 400]}, {"set": ["stage", "level - 2"]}, {"emit": "reform"}], f"mode == 0 && level >= 2 && level <= {1 + len(STAGE_LEVELS)}")

    # --- the card, then the stage
    stage_time = "400 - 100 * (" + " || ".join(f"stage == {c}" for c in CASTLES) + ")"
    into_stage = [{"set": ["mode", 2]}, {"set": ["time_left", stage_time]}, {"set": ["star", 0]}, {"set": ["blink", 0]}, {"set": ["hurry", 0]}, {"level": "stage + 2"}]
    r("the card leads into the stage", {"every": 0.05}, into_stage + [{"emit": "reform"}], "mode == 1 && time > mode_t + 2.4 && checkpoint == 0")
    r("the card leads into the stage at its midpoint", {"every": 0.05}, into_stage + [{"teleport": {"target": "id:p", "to": ["cp_x", "cp_y"]}}, {"emit": "reform"}], "mode == 1 && time > mode_t + 2.4 && checkpoint == 1")
    r("world and stage numbers", {"every": 0.05}, [{"set": ["w", "floor(stage / 4) + 1"]}, {"set": ["s", "stage % 4 + 1"]}])

    # --- Tobin's form: one event puts the right prefab on him
    forms = [("tobin_s", "form == 0"), ("tobin_b", "form == 1"), ("tobin_f", "form == 2")]
    for prefab, f in forms:
        r(f"reform {prefab}", {"event": "reform"}, [{"become": {"target": "id:p", "prefab": prefab}}], f"{PLAY} && star == 0 && blink == 0 && {f}")
        r(f"reform {prefab} star", {"event": "reform"}, [{"become": {"target": "id:p", "prefab": prefab + "_star"}}], f"{PLAY} && star == 1 && {f}")
    r("reform blinking", {"event": "reform"}, [{"become": {"target": "id:p", "prefab": "tobin_s_blink"}}], f"{PLAY} && star == 0 && blink == 1")
    for k, f in (("s", "form == 0"), ("b", "form == 1"), ("f", "form == 2")):
        r(f"walk to the hut {k}", {"event": "reform"}, [{"become": {"target": "id:p", "prefab": f"walk_{k}"}}], f"mode == 4 && {f}")
    r("blinking ends", {"every": 0.1}, [{"set": ["blink", 0]}, {"emit": "reform"}], f"{PLAY} && blink == 1 && time > inv_until")
    r("star power ends", {"every": 0.1}, [{"set": ["star", 0]}, {"music": "level"}, {"emit": "reform"}], f"{PLAY} && star == 1 && time > star_until")

    # --- moving: jumps and seeds
    r("jump sound", {"press": "action"}, [{"play": "jump"}], f"{PLAY} && form == 0 && self_vy == 0")
    r("big jump sound", {"press": "action"}, [{"play": "jump_big"}], f"{PLAY} && form >= 1 && self_vy == 0")
    r("throw a seed", {"press": "secondary"}, [{"spawn": {"prefab": "seed", "at": ["self_x + self_face * 8", "self_y - 4"], "vel": ["self_face * 220", 80]}}, {"play": "seed"}], f"{PLAY} && form == 2 && count_seed < 2")
    r("a seed bursts on a wall", {"hit": ["seed", "solid"]}, [{"burst": {"at": "self", "n": 6, "color": "#ffb03a", "speed": [20, 50], "life": [0.1, 0.3]}}, {"destroy": "self"}], side="sides")

    # --- foes
    for kind, k in KINDS.items():
        r(f"star power beats a {kind}", {"touch": ["star", kind]}, [{"become": {"target": "other", "prefab": k["dead"]}}, {"velocity": {"target": "other", "v": [0, -220]}}, {"add": ["score", 200]}, pop(200), {"play": "kick"}], PLAY)
        r(f"a seed burns a {kind}", {"touch": ["seed", kind]}, [{"become": {"target": "other", "prefab": k["dead"]}}, {"velocity": {"target": "other", "v": [0, -220]}}, {"add": ["score", 200]}, pop(200), {"play": "kick"}, {"destroy": "self"}])
        r(f"a shell bowls over a {kind}", {"touch": ["kshell", kind]}, [{"become": {"target": "other", "prefab": k["dead"]}}, {"velocity": {"target": "other", "v": [0, -220]}}, {"add": ["score", 500]}, pop(500), {"play": "kick"}])
        r(f"a bump from below knocks over a {kind}", {"touch": ["bumper", kind]}, [{"become": {"target": "other", "prefab": k["dead"]}}, {"velocity": {"target": "other", "v": [0, -260]}}, {"add": ["score", 100]}, pop(100), {"play": "kick"}])
        if k["stomp"]:
            r(f"stomp a {kind}", {"touch": ["hero", kind]}, [{"become": {"target": "other", "prefab": k["stomp"]}}, {"velocity": {"target": "other", "v": [0, 0]}}, {"velocity": {"target": "self", "v": ["self_vx", -280]}}, {"add": ["score", k["score"]]}, {"add": ["stomps", 1]}, pop(k["score"]), {"play": "stomp"}], f"{PLAY} && star == 0", side="bottom")
            r(f"a {kind} hurts", {"touch": ["vuln", kind]}, [{"emit": "hurt"}], PLAY, side=["top", "left", "right"])
        else:
            r(f"a {kind} hurts from every side", {"touch": ["vuln", kind]}, [{"emit": "hurt"}], PLAY)
    r("star power beats a shell", {"touch": ["star", "kshell"]}, [{"destroy": "other"}, {"add": ["score", 200]}, pop(200), {"play": "kick"}], PLAY)
    r("stop a rolling shell from above", {"touch": ["hero", "kshell"]}, [{"become": {"target": "other", "prefab": "shell"}}, {"velocity": {"target": "other", "v": [0, 0]}}, {"velocity": {"target": "self", "v": ["self_vx", -280]}}, {"play": "stomp"}], f"{PLAY} && star == 0", side="bottom")
    r("a rolling shell hurts", {"touch": ["vuln", "kshell"]}, [{"emit": "hurt"}], PLAY, side=["top", "left", "right"])
    kick_dir = "(2 * (other_x >= self_x) - 1)"
    r("kick a shell", {"touch": ["hero", "shell"]}, [{"become": {"target": "other", "prefab": "kshell"}}, {"teleport": {"target": "other", "to": [f"self_x + {kick_dir} * 14", "other_y"]}}, {"velocity": {"target": "other", "v": [f"{kick_dir} * 230", 0]}}, {"add": ["kicks", 1]}, {"add": ["score", 400]}, pop(400), {"play": "kick"}], PLAY)
    r("two shells knock each other out", {"touch": ["kshell", "kshell"]}, [{"destroy": "self"}, {"destroy": "other"}, {"play": "kick"}])
    r("a hazard hurts", {"touch": ["vuln", "hazard"]}, [{"emit": "hurt"}], PLAY)
    r("foes and items fall out of the world", {"touch": ["foe", "abyss"]}, [{"destroy": "self"}])
    r("items fall out of the world", {"touch": ["item", "abyss"]}, [{"destroy": "self"}])

    # --- getting hurt, losing a life
    r("hurt when small: lose a life", {"event": "hurt"}, [{"apply": "lose_life", "with": {"hop": -360}}], f"{PLAY} && form == 0")
    r("hurt when big: shrink", {"event": "hurt"}, [{"set": ["form", 0]}, {"set": ["blink", 1]}, {"set": ["inv_until", "time + 2"]}, {"play": "shrink"}, {"emit": "reform"}], f"{PLAY} && form >= 1")
    r("falling into a pit", {"touch": ["hero", "abyss"]}, [{"apply": "lose_life", "with": {"hop": 0}}], PLAY)
    r("lost: the card again", {"every": 0.1}, [{"add": ["lives", -1]}] + goto_card, "mode == 3 && time > dead_at + 3.2 && lives > 1")
    r("lost the last life: game over", {"every": 0.1}, [{"set": ["lives", 0]}, {"set": ["mode", 5]}, {"set": ["mode_t", "time"]}, {"set": ["hi", "max(hi, score)"]}, {"level": "card"}, {"play": "gameover"}], "mode == 3 && time > dead_at + 3.2 && lives <= 1")
    r("game over: back to the title", {"every": 0.1}, [{"restart": True}], "mode == 5 && time > mode_t + 4.5")

    # --- the clock
    r("the clock", {"every": 0.4}, [{"add": ["time_left", -1]}], f"{PLAY} && time_left > 0")
    r("hurry", {"every": 0.4}, [{"set": ["hurry", 1]}, {"play": "hurry"}], f"{PLAY} && time_left <= 100 && hurry == 0")
    r("out of time", {"every": 0.4}, [{"apply": "lose_life", "with": {"hop": -360}}], f"{PLAY} && time_left <= 0")

    # --- blocks
    above = ["other_x", "other_y - 16"]
    bump = {"spawn": {"prefab": "bumper", "at": ["other_x", "other_y - 12"]}}
    r("a comb gives honey", {"hit": ["hero", "coinblock"]}, [{"become": {"target": "other", "prefab": "used"}}, {"spawn": {"prefab": "coinpop", "at": above, "vel": [0, -300]}}, {"apply": "gain_honey"}, bump], PLAY, side="top")
    r("a comb gives royal jelly", {"hit": ["hero", "powerblock"]}, [{"become": {"target": "other", "prefab": "used"}}, {"spawn": {"prefab": "jelly", "at": above}}, {"play": "sprout"}, bump], f"{PLAY} && form == 0", side="top")
    r("a comb gives a firebloom", {"hit": ["hero", "powerblock"]}, [{"become": {"target": "other", "prefab": "used"}}, {"spawn": {"prefab": "firebloom", "at": above}}, {"play": "sprout"}, bump], f"{PLAY} && form >= 1", side="top")
    r("a brick hides a starpetal", {"hit": ["hero", "starblock"]}, [{"become": {"target": "other", "prefab": "used"}}, {"spawn": {"prefab": "starpetal", "at": above, "vel": [0, -200]}}, {"play": "sprout"}, bump], PLAY, side="top")
    r("a comb hides a clover", {"hit": ["hero", "lifeblock"]}, [{"become": {"target": "other", "prefab": "used"}}, {"spawn": {"prefab": "clover", "at": above}}, {"play": "sprout"}, bump], PLAY, side="top")
    r("big Tobin breaks a brick", {"hit": ["hero", "brick"]}, [{"destroy": "other"}, {"burst": {"at": "other", "n": 14, "color": "#c4602c", "speed": [60, 140], "life": [0.4, 0.8], "size": 4, "gravity": 700}}, {"add": ["score", 50]}, {"play": "break"}, bump], f"{PLAY} && form >= 1", side="top")
    r("small Tobin bumps a brick", {"hit": ["hero", "brick"]}, [{"play": "bump"}, bump], f"{PLAY} && form == 0", side="top")
    r("a spent block just bumps", {"hit": ["hero", "spent"]}, [{"play": "bump"}], PLAY, side="top", cooldown=0.15)
    r("honey above a bumped block is collected", {"touch": ["bumper", "honey"]}, [{"destroy": "other"}, {"apply": "gain_honey"}])

    # --- items
    r("honey", {"touch": ["hero", "honey"]}, [{"destroy": "other"}, {"apply": "gain_honey"}], PLAY)
    r("a hundred honeydrops make a life", {"event": "honeycheck"}, [{"add": ["coins", -100]}, {"add": ["lives", 1]}, {"play": "oneup"}], "coins >= 100")
    r("royal jelly makes Tobin big", {"touch": ["hero", "jelly"]}, [{"destroy": "other"}, {"add": ["score", 1000]}, pop(1000), {"play": "powerup"}, {"set": ["form", "max(form, 1)"]}, {"emit": "reform"}], PLAY)
    r("a firebloom gives seeds", {"touch": ["hero", "firebloom"]}, [{"destroy": "other"}, {"add": ["score", 1000]}, pop(1000), {"play": "powerup"}, {"set": ["form", 2]}, {"emit": "reform"}], PLAY)
    r("a starpetal gives star power", {"touch": ["hero", "starpetal"]}, [{"destroy": "other"}, {"add": ["score", 1000]}, pop(1000), {"play": "powerup"}, {"set": ["star", 1]}, {"set": ["star_until", "time + 10"]}, {"music": "star"}, {"emit": "reform"}], PLAY)
    r("a clover is a life", {"touch": ["hero", "clover"]}, [{"destroy": "other"}, {"add": ["lives", 1]}, pop("1UP"), {"play": "oneup"}], PLAY)
    r("a starpetal bounces", {"hit": ["starpetal", "solid"]}, [{"velocity": {"target": "self", "v": ["self_vx", -320]}}], side="bottom")

    # --- logs: down into a bonus room, out through the side
    bonus_of = " + ".join(f"{IX[b.id]} * (stage == {[s.id for s in STAGE_LEVELS].index(b.id[:-1])})" for b in BONUS_LEVELS)
    exit_x = " + ".join(f"{L.exit_col * T + 16} * (stage == {i})" for i, L in enumerate(STAGE_LEVELS) if hasattr(L, "exit_col"))
    r("down a hollow log", {"touching": ["hero", "logmouth"]}, [{"play": "pipe"}, {"level": bonus_of}, {"emit": "reform"}], f"{PLAY} && held_down == 1")
    r("out of the bonus room", {"touch": ["hero", "logside"]}, [{"play": "pipe"}, {"level": "stage + 2"}, {"teleport": {"target": "id:p", "to": [exit_x, 11 * T - 8]}}, {"emit": "reform"}], PLAY)
    r("the midpoint", {"touch": ["hero", "midpoint"]}, [{"set": ["checkpoint", 1]}, {"set": ["cp_x", "other_x"]}, {"set": ["cp_y", "other_y"]}, {"destroy": "other"}], PLAY)

    # --- the beacon pole and the hut
    r("the beacon pole", {"touch": ["hero", "pole"]}, [{"set": ["mode", 4]}, {"set": ["mode_t", "time"]}, {"add": ["score", "min(5000, max(100, floor((200 - self_y) / 16) * 500))"]}, {"play": "flag"}, {"music": "hush"}, {"emit": "reform"}], PLAY)
    r("the fanfare", {"every": 0.1}, [{"play": "clear"}], "mode == 4 && time > mode_t + 1", once=False, cooldown=30)
    r("into the hut", {"touch": ["walker", "hut"]}, [{"destroy": "self"}], "mode == 4")
    r("time left becomes points", {"every": 0.05}, [{"add": ["score", "50 * min(4, time_left)"]}, {"add": ["time_left", "-min(4, time_left)"]}, {"play": "tick"}], "mode == 4 && time > mode_t + 2.2 && time_left > 0")
    r("on to the next stage", {"every": 0.1}, [{"add": ["stage", 1]}, {"set": ["checkpoint", 0]}] + goto_card, "mode == 4 && time_left <= 0 && time > mode_t + 6")

    # --- the Baron
    r("the Baron breathes fire", {"every": 2.2}, [{"spawn": {"prefab": "breath", "at": ["baron_x - 18", "baron_y - 6"], "vel": [-110, 0]}}, {"play": "breathe"}], f"{PLAY} && count_baron > 0 && abs(baron_x - p_x) < 220")
    r("the Baron hops", {"every": 2.9}, [{"velocity": {"target": "id:baron", "v": ["random(0, 1) * 52 - 26", -300]}}], f"{PLAY} && count_baron > 0")
    r("a seed scorches the Baron", {"touch": ["seed", "baron"]}, [{"destroy": "self"}, {"add": ["boss_hp", -1]}, {"burst": {"at": "other", "n": 10, "color": "#ffb03a", "speed": [30, 80], "life": [0.2, 0.5]}}, {"play": "bump"}, {"emit": "baronhit"}])
    r("the Baron falls to seeds", {"event": "baronhit"}, [{"become": {"target": "id:baron", "prefab": "baron_dead"}}, {"velocity": {"target": "id:baron", "v": [0, -200]}}, {"add": ["score", 5000]}, {"play": "bossfall"}], "boss_hp <= 0 && count_baron > 0")
    r("the Baron falls into the lava", {"touch": ["baron", "abyss"]}, [{"destroy": "self"}])
    r("the lever drops the bridge", {"touch": ["hero", "lever"]}, [{"set": ["mode", 6]}, {"set": ["mode_t", "time"]}, {"destroy": "tag:bridge"}, {"destroy": "other"}, {"play": "lever"}, {"music": "hush"}, {"add": ["score", "50 * time_left"]}], PLAY)
    r("the bridge falls", {"every": 0.1}, [{"play": "bossfall"}], "mode == 6 && count_baron > 0", once=True)
    r("a friend speaks", {"every": 0.1}, [{"set": ["talked", 1]}, {"set": ["buzz_line", 0]}], f"mode == 6 && time > mode_t + 2 && talked == 0 && stage < {CASTLES[-1]}")
    r("the Queen speaks", {"every": 0.1}, [{"set": ["talked", 1]}, {"set": ["queen_line", 0]}], f"mode == 6 && time > mode_t + 2 && talked == 0 && stage == {CASTLES[-1]}")
    r("on to the next world", {"event": "buzz_done"}, [{"add": ["stage", 1]}, {"set": ["checkpoint", 0]}] + goto_card)
    r("the end", {"event": "queen_done"}, [{"set": ["mode", 7]}, {"set": ["hi", "max(hi, score)"]}, {"level": "ending"}])

    # --- pause
    r("pause", {"press": "pause"}, [{"set": ["pausebox_line", 0]}, {"play": "pause"}], PLAY)
    r("keep the best score", {"every": 0.5}, [{"set": ["hi", "score"]}], "score > hi")
    return R


def effects():
    return {
        "new_game": {
            "about": "a fresh game from the title",
            "do": [{"set": ["lives", 3]}, {"set": ["score", 0]}, {"set": ["coins", 0]}, {"set": ["form", 0]}, {"set": ["stage", 0]}, {"set": ["checkpoint", 0]}, {"play": "start"}, {"set": ["mode", 1]}, {"set": ["mode_t", "time"]}, {"level": "card"}],
        },
        "gain_honey": {"about": "one honeydrop", "do": [{"add": ["coins", 1]}, {"add": ["score", 200]}, {"play": "honey"}, {"emit": "honeycheck"}]},
        "lose_life": {
            "about": "Tobin loses a life: he turns into his fallen pose, hops (or not, in a pit) and the music stops",
            "params": {"hop": -360},
            "do": [{"set": ["mode", 3]}, {"set": ["dead_at", "time"]}, {"set": ["form", 0]}, {"set": ["star", 0]}, {"set": ["blink", 0]}, {"become": {"target": "id:p", "prefab": "tobin_dead"}}, {"velocity": {"target": "id:p", "v": [0, "$hop"]}}, {"music": "hush"}, {"play": "lose"}],
        },
    }


# ---- the screen ------------------------------------------------------------------------------------------------------------------------------------


def ui():
    hud = "mode >= 1 && mode <= 6"
    U = [
        {"text": "TOBIN", "at": [16, 8], "show": hud},
        {"text": "{score:6}", "at": [16, 17], "show": hud},
        {"text": "HONEY", "at": [80, 8], "show": hud, "color": "#ffd23f"},
        {"text": "x{coins:2}", "at": [86, 17], "show": hud},
        {"text": "WORLD", "at": [144, 8], "show": hud},
        {"text": "{w}-{s}", "at": [150, 17], "show": hud},
        {"text": "TIME", "at": [208, 8], "show": hud},
        {"text": "{time_left:3}", "at": [212, 17], "show": "mode >= 2 && mode <= 6"},
        # title
        {"panel": {"at": [24, 30], "size": [208, 62], "color": "#3a1e0aee"}, "show": "mode == 0"},
        {"text": "HONEYBOUND", "at": [128, 40], "scale": 3, "align": "center", "color": "#ffd23f", "show": "mode == 0"},
        {"text": "TOBIN AND THE BARON OF THORNS", "at": [128, 72], "align": "center", "color": "#f4e7c1", "show": "mode == 0"},
        {"text": "PRESS SPACE TO START", "at": [128, 112], "align": "center", "show": "mode == 0 && floor(time * 2) % 2 == 0"},
        {"text": "HI SCORE {hi:6}", "at": [128, 226], "align": "center", "color": "#ffffff", "show": "mode == 0"},
        {"button": {"id": "start", "label": "START (ENTER)", "at": [88, 124], "size": [80, 14], "key": "Enter", "color": "#8a5228", "do": [{"apply": "new_game"}]}, "show": "mode == 0 && level == 0"},
        # the card
        {"text": "WORLD {w}-{s}", "at": [128, 76], "scale": 2, "align": "center", "show": "mode == 1"},
        {"text": {"pick": "stage_names", "by": "stage"}, "at": [128, 98], "align": "center", "color": "#ffd23f", "show": "mode == 1"},
        {"text": "x  {lives}", "at": [118, 125], "show": "mode == 1"},
        {"text": "ARROWS MOVE   Z JUMP", "at": [128, 170], "align": "center", "color": "#9fd8f0", "show": "mode == 1 && stage == 0"},
        {"text": "HOLD X TO RUN (AND THROW SEEDS)", "at": [128, 182], "align": "center", "color": "#9fd8f0", "show": "mode == 1 && stage == 0"},
        {"text": "DOWN ON A HOLLOW LOG: GO INSIDE", "at": [128, 194], "align": "center", "color": "#9fd8f0", "show": "mode == 1 && stage == 0"},
        {"text": "ESC PAUSES", "at": [128, 206], "align": "center", "color": "#9fd8f0", "show": "mode == 1 && stage == 0"},
        {"text": "GAME OVER", "at": [128, 110], "scale": 2, "align": "center", "color": "#ff5c5c", "show": "mode == 5"},
        # the end
        {"panel": {"at": [16, 28], "size": [224, 92], "color": "#3a1e0add"}, "show": "mode == 7"},
        {"text": "THE HIVE IS SAFE!", "at": [128, 38], "scale": 2, "align": "center", "color": "#ffd23f", "show": "mode == 7"},
        {"text": "QUEEN MELLI IS HOME AND", "at": [128, 60], "align": "center", "show": "mode == 7"},
        {"text": "THE MEADOWS HUM AGAIN.", "at": [128, 70], "align": "center", "show": "mode == 7"},
        {"text": "THANK YOU, TOBIN!", "at": [128, 80], "align": "center", "color": "#ffd23f", "show": "mode == 7"},
        {"text": "SCORE {score:6}    HI {hi:6}", "at": [128, 93], "align": "center", "color": "#9fd8f0", "show": "mode == 7"},
        {"button": {"id": "again", "label": "PLAY AGAIN (ENTER)", "at": [78, 104], "size": [100, 12], "key": "Enter", "color": "#8a5228", "do": [{"restart": True}]}, "show": "mode == 7"},
        # pause and the friends
        {"dialogue": {"id": "pausebox", "at": [48, 92], "size": [160, 40], "pause": True, "lines": [{"who": "PAUSED", "text": "PRESS Z TO PLAY ON"}]}},
        {
            "dialogue": {
                "id": "buzz",
                "at": [8, 150],
                "size": [240, 70],
                "pause": True,
                "done": "buzz_done",
                "lines": [
                    {"who": "BUZZ", "text": "TOBIN! YOU CAME FOR ME! THE BARON HAD ME LOCKED IN HIS LARDER."},
                    {"who": "BUZZ", "text": "BUT QUEEN MELLI IS NOT HERE. HE TOOK HER TO HIS ASHEN KEEP, PAST THE DUSKWOOD."},
                    {"who": "BUZZ", "text": "FOLLOW THE BEACONS EAST. I WILL TELL THE HIVE YOU ARE ON YOUR WAY!"},
                ],
            }
        },
        {
            "dialogue": {
                "id": "queen",
                "at": [8, 150],
                "size": [240, 70],
                "pause": True,
                "done": "queen_done",
                "lines": [
                    {"who": "QUEEN MELLI", "text": "BRAVE LITTLE KEEPER... THE BARON WANTED EVERY FLOWER FOR HIS THORNS."},
                    {"who": "QUEEN MELLI", "text": "YOU CROSSED HIS WHOLE KINGDOM WITH NOTHING BUT A HAT AND A FEW SEEDS."},
                    {"who": "QUEEN MELLI", "text": "COME, TOBIN. THE HIVE HAS BEEN SAVING THE SWEETEST HONEY FOR YOU."},
                ],
            }
        },
    ]
    for w in U:
        t = w.get("text")
        if isinstance(t, str):
            width = len(t) * 6 * w.get("scale", 1)
            assert width <= VIEW_W - 6, f"`{t}` is {width} px wide: the screen is {VIEW_W}"
    return U


# ---- checks: scenarios the engine plays headless -------------------------------------------------------------------------------------------


def run_script(L, finish="pole"):
    """The scripted run through a stage from its hints: hold right and run, jump where the level said, wait for lifts."""
    steps = []
    for x, kind, args in sorted(L.hints, key=lambda h: h[0]):
        if kind == "jump":
            steps.append({"hold_until": ["right", "secondary"], "until": {"var": "p_x", "gte": x}, "timeout": 20})
            steps.append({"hold": ["right", "secondary", "action"], "seconds": args.get("hold", 0.35)})
        elif kind == "wait_lift":
            steps.append({"hold_until": ["right"], "until": {"var": "p_x", "gte": x - 8}, "timeout": 20})
            steps.append({"wait_until": {"entity": args["lift"], "near": args["near"], "tol": args["tol"]}, "timeout": 12})
    if finish == "pole":
        steps.append({"hold_until": ["right", "secondary"], "until": {"var": "mode", "eq": 4}, "timeout": 30})
        steps.append({"wait_until": {"var": "mode", "eq": 1}, "timeout": 30})
    else:
        steps.append({"hold_until": ["right", "secondary"], "until": {"var": "mode", "eq": 6}, "timeout": 30})
        steps.append({"wait": 2.5})
    return steps


# Mechanics the engine proves by playing: each is a search goal for `dev.py solve` (red_engine2 solve), then a few steps and the claims.
MECHANICS = [
    {"key": "stomp", "name": "landing on a grubble flattens it and bounces Tobin up", "level": "1-1", "goal": "stomps >= 1", "progress": "stomps * 4000 + p_x", "tail": [{"wait": 0.3}],
     "expect": [{"var": "score", "gte": 100}, {"sound": "stomp"}, {"var": "mode", "eq": 2}, {"var": "lives", "eq": 3}]},
    {"key": "power", "name": "bumping a comb from below brings royal jelly, and eating it makes Tobin big", "level": "1-1", "goal": "form == 1", "progress": "form * 4000 + count_jelly * 400 + p_x", "tail": [{"wait": 0.3}],
     "expect": [{"var": "form", "eq": 1}, {"sound": "sprout"}, {"sound": "powerup"}]},
    {"key": "fire", "name": "a second comb gives big Tobin a firebloom: seeds to throw", "level": "1-1", "goal": "form == 2", "progress": "form * 4000 + count_firebloom * 1500 + count_jelly * 400 + min(p_x, 400 + form * 700) - (count_firebloom > 0) * (abs(p_x - 1032) + abs(p_y - 130))",
     "tail": [{"hold": ["secondary"], "seconds": 0.1}, {"wait": 0.2}], "expect": [{"var": "form", "eq": 2}, {"sound": "seed"}, {"sound": "powerup", "min": 2}]},
    {"key": "star", "name": "a brick hides a starpetal: star power plays its own tune", "level": "1-1", "goal": "star == 1", "progress": "star * 4000 + count_starpetal * 400 + p_x", "tail": [{"wait": 0.5}],
     "expect": [{"var": "star", "eq": 1}, {"sound": "sprout"}]},
    {"key": "shell", "name": "a stomped snail hides in its shell, and a kick sends the shell rolling", "level": "1-1", "goal": "kicks >= 1", "progress": "kicks * 4000 + count_shell * 400 + p_x", "tail": [{"wait": 0.5}],
     "expect": [{"var": "kicks", "eq": 1}, {"sound": "kick"}, {"count": "kshell", "gte": 1}]},
    {"key": "log", "name": "holding down on a hollow log leads to the honey room, and its side log comes out further along", "level": "1-1",
     "goal": "level == 2 && coins >= 10", "progress": "(level == 2) * (2000 - abs(p_x - 744) - abs(p_y - 137)) + (level == 10) * (10000 + coins * 100 + p_x)",
     "tail": [{"wait": 0.5}], "expect": [{"var": "coins", "gte": 10}, {"var": "p_x", "gt": 2500}, {"sound": "pipe", "min": 2}, {"var": "mode", "eq": 2}]},
    {"key": "midpoint", "name": "after the midpoint a lost life starts Tobin there, not at the beginning", "level": "1-1", "goal": "checkpoint == 1",
     "tail": [{"hold_until": ["right"], "until": {"var": "mode", "eq": 3}, "timeout": 40}, {"wait_until": {"var": "mode", "eq": 2}, "timeout": 10}, {"wait": 0.2}],
     "expect": [{"var": "lives", "eq": 2}, {"var": "p_x", "gte": 1500}, {"var": "checkpoint", "eq": 1}]},
]


def scenarios():
    S = []
    S.append(
        {
            "name": "the title starts a game, the card names the stage, then Tobin plays 1-1",
            "max_seconds": 10,
            "script": [{"wait": 0.5}, {"press": "action"}, {"wait": 1}, {"wait_until": {"var": "mode", "eq": 2}, "timeout": 5}, {"hold": ["right"], "seconds": 1}],
            "expect": [{"var": "level", "eq": IX["1-1"]}, {"var": "lives", "eq": 3}, {"var": "time_left", "lte": 400}, {"var": "p_x", "gt": 60}, {"sound": "start"}],
        }
    )
    sols = json.loads(SOLUTIONS.read_text()) if SOLUTIONS.exists() else {}
    for m in MECHANICS:
        if m["key"] not in sols:
            continue  # `dev.py solve` finds its run
        S.append({"name": m["name"], "level": m["level"], "max_seconds": 120, "script": sols[m["key"]] + m["tail"], "expect": m["expect"]})
    S.append(
        {
            "name": "a pit costs a life and the card comes back",
            "level": "1-1",
            "max_seconds": 30,
            "script": [{"hold_until": ["right"], "until": {"var": "mode", "eq": 3}, "timeout": 20}, {"wait_until": {"var": "mode", "eq": 1}, "timeout": 6}],
            "expect": [{"var": "lives", "eq": 2}, {"sound": "lose"}, {"var": "level", "eq": IX["card"]}],
        }
    )
    lose_one = [{"hold_until": ["right"], "until": {"var": "mode", "eq": 3}, "timeout": 30}, {"wait_until": {"var": "mode", "ne": 3}, "timeout": 6}, {"wait_until": {"var": "mode", "ne": 1}, "timeout": 6}]
    S.append(
        {
            "name": "losing the last life is game over, then the title again with the best score kept",
            "max_seconds": 120,
            "script": [{"wait": 0.3}, {"press": "action"}, {"wait_until": {"var": "mode", "eq": 2}, "timeout": 5}] + lose_one * 3 + [{"wait_until": {"var": "mode", "eq": 0}, "timeout": 8}, {"wait": 0.5}],
            "expect": [{"var": "level", "eq": 0}, {"var": "lives", "eq": 3}, {"sound": "gameover"}, {"var": "hi", "gte": 0}],
        }
    )
    sols = json.loads(SOLUTIONS.read_text()) if SOLUTIONS.exists() else {}
    for i, L in enumerate(STAGE_LEVELS):
        castle_stage = L.theme == "castle"
        last = i == len(STAGE_LEVELS) - 1
        if castle_stage:
            # the lever, then the friend's three lines (Z moves on), then the next card (or the end)
            tail = [{"wait_until": {"var": "talked", "eq": 1}, "timeout": 8}, {"wait": 0.2}] + [{"press": "action"}, {"wait": 0.3}] * 3 + [{"wait": 0.5}]
            expect = [{"sound": "lever"}, {"count": "baron", "eq": 0}] + ([{"var": "mode", "eq": 7}, {"var": "level", "eq": IX["ending"]}] if last else [{"var": "mode", "eq": 1}, {"var": "stage", "eq": i + 1}])
        else:
            tail = [{"wait_until": {"var": "mode", "eq": 1}, "timeout": 30}]
            expect = [{"var": "mode", "eq": 1}, {"var": "level", "eq": IX["card"]}, {"var": "stage", "eq": i + 1}, {"sound": "clear"}]
        script = sols.get(L.id) or run_script(L, finish="lever" if castle_stage else "pole")
        S.append(
            {
                "name": f"{L.id} {L.name.lower()} can be finished",
                "level": L.id,
                "max_seconds": 300,
                **({"smoke": True} if L.id == "1-1" else {}),
                "script": script + tail,
                "expect": expect + [{"var": "lives", "eq": 3}],
            }
        )
    S.append(
        {
            "name": "pausing stops the world until Z",
            "level": "1-1",
            "max_seconds": 10,
            "script": [{"hold": ["right"], "seconds": 0.5}, {"press": "pause"}, {"hold": ["right"], "seconds": 1.0}, {"press": "action"}, {"hold": ["right"], "seconds": 0.2}],
            "expect": [{"var": "p_x", "lt": 130}, {"sound": "pause"}, {"var": "pausebox_line", "gte": 1}],
        }
    )
    return S


def build():
    sprites = art.all_sprites()
    P = prefabs()
    game = {
        "game2d": 1,
        "id": "honeybound",
        "title": "Honeybound",
        "description": "A side-scrolling platformer: Tobin the young beekeeper runs, stomps and bounces through eight stages to free the Great Hive from Baron Thornwick.",
        "capabilities": {"presentation": "2d", "platforms": ["windows", "linux"], "networking": "offline", "input": ["keyboard", "mouse"], "persistence": ["progress"]},
        "view": {"width": VIEW_W, "height": VIEW_H, "background": "#6bb4ff", "scale": "integer", "camera": {"follow": "tobin", "lerp": 1, "forward_only": True}},
        "sprites": sprites,
        "sounds": audio.sounds(),
        "music": audio.music(),
        "tables": {"stage_names": [L.name for L in STAGE_LEVELS]},
        "vars": {
            "mode": 0, "lives": 3, "score": 0, "coins": 0, "hi": 0, "form": 0, "star": 0, "blink": 0, "inv_until": 0, "star_until": 0, "time_left": 400,
            "stage": 0, "checkpoint": 0, "cp_x": 0, "cp_y": 0, "mode_t": 0, "dead_at": 0, "hurry": 0, "w": 1, "s": 1, "talked": 0,
            "boss_hp": 5, "stomps": 0, "kicks": 0,
        },
        "persist": ["hi"],
        "prefabs": P,
        "levels": [title_level(), card_level()] + [level_json(L) for L in STAGE_LEVELS] + [level_json(L) for L in BONUS_LEVELS] + [ending_level()],
        "ui": ui(),
        "effects": effects(),
        "rules": rules(),
        "checks": {"scenarios": scenarios()},
    }
    # The Baron's toughness per keep: set when the stage starts.
    hp = {STAGE_LEVELS.index(L): L.boss_hp for L in STAGE_LEVELS if hasattr(L, "boss_hp")}
    game["rules"].insert(0, {"id": "the Baron's strength", "when": {"every": 0.05}, "if": "mode == 1", "do": [{"set": ["boss_hp", " + ".join(f"{v} * (stage == {k})" for k, v in hp.items())]}]})
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
        print(f"{len(game['prefabs'])} prefabs, {len(game['rules'])} rules, {len(game['sprites'])} sprites, {len(text) // 1024} KB")
        return
    OUT.write_text(json.dumps(game, indent=1) + "\n")
    print(f"wrote {OUT} ({len(text) // 1024} KB compact)")


if __name__ == "__main__":
    main()
