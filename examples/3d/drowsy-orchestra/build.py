#!/usr/bin/env python3
"""Writes drowsy-orchestra.json and its audio scores (standard library only; run it again to rewrite byte for byte).

Three chambers in a row, a wall at the end of each. Chalk props before a wall are WORDS: picking one up writes it on
that wall and the wall behaves like the word. A bell rings every ten seconds (from chamber 2 on) and you pick a law.
The Tock (clock-faced beast) sweeps a band of chamber 3 and eats a minute when it catches you.
"""
import json
from pathlib import Path

HERE = Path(__file__).parent
X = 7.0                                   # half width of the hall
WALLS = {1: 10.0, 2: 22.0, 3: 38.0}       # z of each wall
WORDS = {1: ["MELT"], 2: ["MELT", "LOW"], 3: ["MELT", "LOW", "STILL"]}
TILE_Z = {1: 4.0, 2: 14.0, 3: 26.0}
CHAMBER = {1: (0.0, 10.0), 2: (10.0, 22.0), 3: (22.0, 38.0)}
BAND = (31.0, 33.0)                       # z range the Tock sweeps
COLS = 7                                  # columns of 2 m across the band
COL_X = [-6 + 2 * i for i in range(COLS)]
WORD_COLOR = {"MELT": "@ember", "LOW": "@moss", "STILL": "@frost"}
TILE_PROP = {"MELT": "fire_extinguisher", "LOW": "traffic_cone", "STILL": "crate"}
TILE_X = {1: {"MELT": -3.5}, 2: {"MELT": -4.0, "LOW": 4.0}, 3: {"MELT": -5.0, "LOW": 5.0, "STILL": -2.5}}
LAWS = {1: "HUSH", 2: "LOUD", 3: "SLEEP"}
LAW_INPUT = {1: "attack", 2: "alt", 3: "crouch"}
MELT_SECS = "(6 + (law == 2) * 6 - (law == 1) * 2.5)"
BEAST_TICKS = "(3 + (law == 1) * 3 - (law == 2))"   # quarter-seconds per column step
MAX_LOST = 5


def text(oid, s, pos, height=0.45, color="@bone", emissive=None, yaw=180, backing=None):
    o = {"id": oid, "type": "text", "text": s, "position": pos, "height": height, "rotation": [0, yaw, 0],
         "material": {"color": color, "emissive": emissive or color}}
    if backing:
        o["backing"] = backing
    return o


def box(oid, size, pos, preset=None, **kw):
    o = {"id": oid, "type": "box", "size": size, "position": pos}
    if preset:
        o["material"] = {"preset": preset}
    o.update(kw)
    return o


def prefabs():
    return {
        "tock": {
            "desc": "the Tock: a clock-faced beast on four brass legs that eats a minute of your day",
            "params": {"face": {"default": "#eadcc2", "palette": "bone"}, "hide": {"default": "#8a3b2a", "palette": "ember"},
                       "brass": {"default": "#d9a441", "palette": "brass"}},
            "objects": [
                {"id": "body", "type": "box", "size": [1.5, 1.0, 1.1], "position": [0, 0.9, 0], "material": {"color": "$hide", "roughness": 0.8}},
                {"id": "head", "type": "cylinder", "radius": 0.85, "height": 0.4, "position": [0, 1.9, 0.25], "rotation": [90, 0, 0],
                 "material": {"color": "$face", "emissive": "$face"}},
                {"id": "rim", "type": "cylinder", "radius": 0.95, "height": 0.3, "position": [0, 1.9, 0.12], "rotation": [90, 0, 0],
                 "material": {"color": "$brass", "metallic": 0.9, "roughness": 0.3}},
                {"id": "hand_h", "type": "box", "size": [0.12, 0.6, 0.06], "position": [0, 2.1, 0.5], "material": {"color": "#1a1226"}},
                {"id": "hand_m", "type": "box", "size": [0.5, 0.1, 0.06], "position": [0.22, 1.9, 0.52], "material": {"color": "#1a1226"}},
                {"id": "leg_a", "type": "box", "size": [0.2, 0.5, 0.2], "position": [-0.55, 0.25, 0.35], "material": {"color": "$brass", "metallic": 0.8}},
                {"id": "leg_b", "type": "box", "size": [0.2, 0.5, 0.2], "position": [0.55, 0.25, 0.35], "material": {"color": "$brass", "metallic": 0.8}},
                {"id": "leg_c", "type": "box", "size": [0.2, 0.5, 0.2], "position": [-0.55, 0.25, -0.35], "material": {"color": "$brass", "metallic": 0.8}},
                {"id": "leg_d", "type": "box", "size": [0.2, 0.5, 0.2], "position": [0.55, 0.25, -0.35], "material": {"color": "$brass", "metallic": 0.8}},
            ],
        },
        "bell": {
            "desc": "the hall bell that rings every ten seconds: a brass dome with a clapper",
            "params": {"brass": {"default": "#d9a441", "palette": "brass"}, "glow": {"default": "#f6d27a", "palette": "bone"}},
            "objects": [
                {"id": "dome", "type": "sphere", "radius": 0.8, "position": [0, 0, 0], "material": {"color": "$brass", "metallic": 0.9, "roughness": 0.25}},
                {"id": "lip", "type": "cylinder", "radius": 0.9, "height": 0.12, "position": [0, -0.45, 0], "material": {"color": "$brass", "metallic": 0.9, "roughness": 0.25}},
                {"id": "clapper", "type": "sphere", "radius": 0.18, "position": [0, -0.65, 0], "material": {"color": "$glow", "emissive": "$glow"}},
                {"id": "yoke", "type": "box", "size": [0.12, 0.8, 0.12], "position": [0, 0.9, 0], "material": {"color": "$brass", "metallic": 0.7}},
            ],
        },
        "chalk_plinth": {
            "desc": "a small slate plinth with a glowing ring where a word of chalk waits",
            "params": {"slate": {"default": "#2b2740", "palette": "slate"}, "ring": {"default": "#f6d27a", "palette": "bone"}},
            "objects": [
                {"id": "base", "type": "cylinder", "radius": 0.5, "height": 0.8, "position": [0, 0.4, 0], "material": {"color": "$slate", "roughness": 0.7}},
                {"id": "ring", "type": "cylinder", "radius": 0.5, "height": 0.06, "position": [0, 0.82, 0], "material": {"color": "$ring", "emissive": "$ring"}},
            ],
        },
        "cog": {
            "desc": "a great brass cogwheel set in a wall, stopped mid-turn",
            "params": {"brass": {"default": "#d9a441", "palette": "brass"}},
            "objects": [
                {"id": "disc", "type": "cylinder", "radius": 1.2, "height": 0.2, "position": [0, 0, 0], "rotation": [90, 0, 0], "material": {"color": "$brass", "metallic": 0.9, "roughness": 0.35}},
                {"id": "hub", "type": "cylinder", "radius": 0.3, "height": 0.3, "position": [0, 0, 0.05], "rotation": [90, 0, 0], "material": {"color": "#1a1226"}},
                {"id": "t1", "type": "box", "size": [0.4, 2.7, 0.2], "position": [0, 0, 0], "material": {"color": "$brass", "metallic": 0.9, "roughness": 0.35}},
                {"id": "t2", "type": "box", "size": [2.7, 0.4, 0.2], "position": [0, 0, 0], "material": {"color": "$brass", "metallic": 0.9, "roughness": 0.35}},
            ],
        },
    }


def objects():
    o = []
    # hall shell
    o.append(box("floor", [2 * X + 0.4, 0.2, 46], [0, -0.1, 22.0], "flagstone"))
    o.append(box("wall_w", [0.4, 5, 46], [-X - 0.2, 2.5, 22.0], "plaster"))
    o.append(box("wall_e", [0.4, 5, 46], [X + 0.2, 2.5, 22.0], "plaster"))
    o.append(box("wall_back", [2 * X + 0.8, 5, 0.4], [0, 2.5, -0.9], "plaster"))
    o.append(box("wall_front", [2 * X + 0.8, 5, 0.4], [0, 2.5, 45.0], "plaster"))
    o.append(box("ceiling", [2 * X + 0.8, 0.3, 46], [0, 5.1, 22.0], "plaster"))
    # the clock door at the far end
    o.append(box("door_frame", [4.4, 3.6, 0.3], [0, 1.8, 44.6], "brass_dark", collide=False))
    o.append({"id": "door_glow", "type": "box", "size": [3.6, 3.0, 0.1], "position": [0, 1.5, 44.4], "collide": False,
              "material": {"color": "@bone", "emissive": "@bone"}})
    o.append(text("door_sign", "THE STOPPED CLOCK", [0, 3.9, 44.2], 0.4, "@brass", "@brass", yaw=180))
    o.append({"id": "cog_a", "type": "prefab", "prefab": "cog", "position": [-6.7, 3.0, 17.0], "rotation": [0, 90, 0], "collide": False})
    o.append({"id": "cog_b", "type": "prefab", "prefab": "cog", "position": [6.7, 3.0, 30.0], "rotation": [0, -90, 0], "collide": False})
    o.append({"id": "bell_obj", "type": "prefab", "prefab": "bell", "position": [0, 4.0, 3.0], "collide": False})
    for k, wz in WALLS.items():
        z0, z1 = CHAMBER[k]
        o.append(box(f"wall_{k}", [2 * X, 3.6, 0.5], [0, 1.8, wz], "gate_wall"))
        o.append(box(f"low_{k}", [2 * X, 0.55, 0.5], [0, 0.275, wz], "gate_low"))
        o.append({"id": f"puddle_{k}", "type": "box", "size": [2 * X, 0.05, 1.2], "position": [0, 0.03, wz], "collide": False,
                  "material": {"color": "@ember", "emissive": "@ember"}})
        o.append(text(f"w{k}_BLANK", "????", [0, 4.2, wz - 0.3], 0.5, "@bone", "@bone"))
        for w in WORDS[k]:
            o.append(text(f"w{k}_{w}", w, [0, 4.2, wz - 0.3], 0.8, WORD_COLOR[w], WORD_COLOR[w]))
        # law lamp text above the chamber start
        for n, name in LAWS.items():
            o.append(text(f"law{k}_{n}", f"LAW: {name}", [0, 4.3, z0 + 1.5], 0.4, "@brass", "@brass"))
        o.append(text(f"law{k}_0", "LAW: NONE", [0, 4.3, z0 + 1.5], 0.4, "@bone", "@bone"))
        for w in WORDS[k]:
            tx = TILE_X[k][w]
            tz = TILE_Z[k]
            o.append({"id": f"plinth_{k}_{w}", "type": "prefab", "prefab": "chalk_plinth", "position": [tx, 0, tz], "collide": True})
            o.append({"id": f"tile_{k}_{w}", "type": "prop", "prop": TILE_PROP[w], "movable": True, "position": [tx, 0.88, tz],
                      "material": {"color": WORD_COLOR[w], "emissive": WORD_COLOR[w]}})
            o.append(text(f"lab_{k}_{w}", w, [tx, 1.7, tz - 0.1], 0.3, WORD_COLOR[w], WORD_COLOR[w]))
    # the beast's stations (one shown at a time)
    for i, cx in enumerate(COL_X):
        o.append({"id": f"tock_{i}", "type": "prefab", "prefab": "tock", "position": [cx, 0, (BAND[0] + BAND[1]) / 2], "rotation": [0, 180, 0], "collide": False})
    o.append({"id": "band_floor", "type": "box", "size": [2 * X, 0.04, BAND[1] - BAND[0]], "position": [0, 0.02, (BAND[0] + BAND[1]) / 2],
              "collide": False, "material": {"color": "@ember", "emissive": "@ember"}})
    o.append(text("band_sign", "THE TOCK PATROLS HERE", [0, 2.6, BAND[0] - 1.2], 0.3, "@ember", "@ember"))
    return o


def lamps():
    out = []
    for i, (x, z) in enumerate([(-4, 3), (4, 3), (-4, 16), (4, 16), (-4, 28), (4, 28), (-4, 35), (4, 35), (0, 41)]):
        out.append({"id": f"l{i}", "type": "point", "position": [x, 3.6, z], "color": "#ffc27a", "intensity": 14, "range": 14})
    return out


def zones():
    z = [{"id": "c1", "rect": [-X, 0, X, 10], "y": 0, "kind": "room"},
         {"id": "c2", "rect": [-X, 10.4, X, 22], "y": 0, "kind": "room"},
         {"id": "c3", "rect": [-X, 22.4, X, 38], "y": 0, "kind": "room"},
         {"id": "exit", "rect": [-X, 38.5, X, 44.5], "y": 0, "kind": "pad"}]
    for k, wz in WALLS.items():
        z.append({"id": f"in_wall_{k}", "rect": [-X, wz - 0.9, X, wz + 0.9], "y": 0, "kind": "pad"})
    for i, cx in enumerate(COL_X):
        z.append({"id": f"col_{i}", "rect": [cx - 1, BAND[0], cx + 1, BAND[1]], "y": 0, "kind": "pad"})
    return z


def rules():
    r = []
    setup = [{"deactivate": f"low_{k}"} for k in WALLS] + [{"hide": f"puddle_{k}"} for k in WALLS]
    for k in WALLS:
        setup += [{"hide": f"w{k}_{w}"} for w in WORDS[k]]
        setup += [{"hide": f"law{k}_{n}"} for n in LAWS]
    setup += [{"hide": f"tock_{i}"} for i in range(COLS)] + [{"hide": "band_floor"}, {"hide": "band_sign"}]
    setup += [{"set": ["ch", 1]}]
    r.append({"id": "setup", "when": {"start": True}, "do": setup})

    # chambers
    r.append({"id": "enter_c1", "when": {"enter": {"zone": "c1"}}, "do": [{"set": ["ch", 1]}]})
    r.append({"id": "enter_c2", "when": {"enter": {"zone": "c2"}}, "do": [{"set": ["ch", 2]}]})
    r.append({"id": "bell_start", "when": {"enter": {"zone": "c2"}}, "once": True, "do": [{"timer": ["bell", 10]}]})
    r.append({"id": "enter_c3", "when": {"enter": {"zone": "c3"}}, "do": [{"set": ["ch", 3]}, {"set": ["beast_on", 1]}, {"set": ["bi", 0]}, {"set": ["bdir", 1]}, {"set": ["bc", 0]},
                                                                   {"show": "band_floor"}, {"show": "band_sign"}, {"show": "tock_0"}]})
    r.append({"id": "enter_exit", "when": {"enter": {"zone": "exit"}}, "do": [{"set": ["beast_on", 0]}, {"set": ["ch", 4]}]})
    r.append({"id": "win", "when": {"enter": {"zone": "exit"}}, "do": [{"emit": "victory"}, {"end": "victory"}]})

    # wall-occupancy (so a melted wall never re-forms around you)
    for k in WALLS:
        r.append({"id": f"in_wall_{k}_on", "when": {"enter": {"zone": f"in_wall_{k}"}}, "do": [{"set": [f"inwall_{k}", 1]}]})
        r.append({"id": f"in_wall_{k}_off", "when": {"exit": {"zone": f"in_wall_{k}"}}, "do": [{"set": [f"inwall_{k}", 0]}]})

    # writing a word
    for k in WALLS:
        for w in WORDS[k]:
            do = [{"add": ["words", "1 + (law == 3)"]}, {"set": [f"word_{k}", w]}] if False else [{"add": ["words", "1 + (law == 3)"]}]
            do += [{"hide": f"w{k}_{v}"} for v in WORDS[k]] + [{"hide": f"w{k}_BLANK"}, {"show": f"w{k}_{w}"}]
            if w == "MELT":
                do += [{"deactivate": f"wall_{k}"}, {"show": f"puddle_{k}"}, {"timer": [f"melt_{k}", MELT_SECS]}]
            elif w == "LOW":
                do += [{"cancel": f"melt_{k}"}, {"deactivate": f"wall_{k}"}, {"activate": f"low_{k}"}, {"hide": f"puddle_{k}"}, {"set": [f"lowed_{k}", 1]}]
            elif w == "STILL":
                do += [{"set": ["still", 1]}, {"timer": ["still", 6]}]
            do += [{"reset": "self"}, {"emit": "wrote"}]
            r.append({"id": f"write_{k}_{w}", "when": {"event": "pickup", "prop": f"tile_{k}_{w}"}, "do": do})
        # melt ends
        r.append({"id": f"melt_end_{k}", "when": {"timer": f"melt_{k}"}, "if": f"inwall_{k} == 0 && lowed_{k} == 0",
                  "do": [{"activate": f"wall_{k}"}, {"hide": f"puddle_{k}"}, {"hide": f"w{k}_MELT"}, {"show": f"w{k}_BLANK"}, {"emit": "closed"}]})
        r.append({"id": f"melt_wait_{k}", "when": {"timer": f"melt_{k}"}, "if": f"inwall_{k} == 1 && lowed_{k} == 0", "do": [{"timer": [f"melt_{k}", 0.5]}]})
    r.append({"id": "still_end", "when": {"timer": "still"}, "do": [{"set": ["still", 0]}, {"hide": "w3_STILL"}, {"show": "w3_BLANK"}]})

    # the bell and the three laws
    r.append({"id": "bell_ring", "when": {"timer": "bell"}, "do": [{"set": ["choosing", 1]}, {"set": ["bellglow", 1]}, {"timer": ["bell", 10]}, {"emit": "bell"}]})
    for n, name in LAWS.items():
        do = [{"set": ["law", n]}, {"set": ["choosing", 0]}, {"set": ["bellglow", 0]}, {"emit": "law_set"}]
        for k in WALLS:
            do += [{"hide": f"law{k}_{m}"} for m in [0, *LAWS]] + [{"show": f"law{k}_{n}"}]
        r.append({"id": f"law_{name.lower()}", "when": {"input": LAW_INPUT[n]}, "if": "choosing == 1", "do": do})
    r.append({"id": "law_first_text", "when": {"enter": {"zone": "c2"}}, "once": True, "do": [{"show": f"law{k}_0"} for k in WALLS]})

    # the Tock
    r.append({"id": "tock_tick", "when": {"every": 0.25}, "if": "beast_on == 1 && still == 0", "do": [{"add": ["bc", 1]}]})
    r.append({"id": "tock_step", "when": {"every": 0.25}, "if": f"bc >= {BEAST_TICKS}", "do": [{"set": ["bc", 0]}, {"add": ["bi", "bdir"]}]})
    r.append({"id": "tock_turn_hi", "when": {"every": 0.25}, "if": f"bi >= {COLS - 1}", "do": [{"set": ["bdir", -1]}]})
    r.append({"id": "tock_turn_lo", "when": {"every": 0.25}, "if": "bi <= 0", "do": [{"set": ["bdir", 1]}]})
    caught = [{"add": ["lost", 1]}, {"set": ["px", -1]}, {"set": ["hit", 0]}, {"teleport": "c3"}, {"emit": "caught"}]
    for i in range(COLS):
        r.append({"id": f"tock_show_{i}", "when": {"every": 0.25}, "if": f"beast_on == 1 && bi == {i}", "do": [{"show": f"tock_{i}"}]})
        r.append({"id": f"tock_hide_{i}", "when": {"every": 0.25}, "if": f"beast_on == 0 || bi != {i}", "do": [{"hide": f"tock_{i}"}]})
        r.append({"id": f"col_{i}_on", "when": {"enter": {"zone": f"col_{i}"}}, "do": [{"set": ["px", i]}]})
        r.append({"id": f"col_{i}_off", "when": {"exit": {"zone": f"col_{i}"}}, "if": f"px == {i}", "do": [{"set": ["px", -1]}]})
        r.append({"id": f"col_{i}_catch_in", "when": {"enter": {"zone": f"col_{i}"}},
                  "if": f"beast_on == 1 && still == 0 && law != 3 && bi == {i}", "do": list(caught)})
        r.append({"id": f"col_{i}_catch_out", "when": {"exit": {"zone": f"col_{i}"}}, "if": "hit == 1", "do": list(caught)})
    # the Tock steps onto a column you stand in: you are caught the moment you leave it
    r.append({"id": "tock_mark", "when": {"every": 0.25}, "if": "beast_on == 1 && still == 0 && law != 3 && px >= 0 && px == bi", "do": [{"set": ["hit", 1]}]})
    r.append({"id": "day_gone", "when": {"event": "caught"}, "if": f"lost >= {MAX_LOST}", "do": [{"end": "defeat"}]})
    return r


def ui():
    law_line = {1: "Law HUSH: slow Tock, walls close fast.",
                2: "Law LOUD: fast Tock, walls stay open.",
                3: "Law SLEEP: no Tock, words cost 2."}
    obj = [{"if": "choosing == 1",
            "text": "BELL! Pick a law: LEFT-CLICK Hush (slow Tock) | RIGHT-CLICK Loud (walls stay open) | C Sleep (no Tock, words cost 2)"},
           {"if": "ch == 1", "text": "Press E on the orange MELT chalk: it is written on the wall and the wall melts. Walk through."},
           {"if": "ch == 2 && law == 0", "text": "MELT: wall opens a while. LOW: wall stays, hop it. The bell rings every 10 s: pick a law."},
           {"if": "ch == 3 && law == 0", "text": "Cross the red band: the Tock eats a minute if it catches you. STILL freezes it."},
           {"if": "ch == 4", "text": "You made it."}]
    for n, t in law_line.items():
        obj.append({"if": f"law == {n} && ch == 3", "text": t + " Reach the clock."})
        obj.append({"if": f"law == {n}", "text": t + " MELT opens, LOW hops, STILL freezes."})
    obj.append({"text": "Reach the stopped clock with the fewest words and the fewest minutes lost."})
    return {
        "title": "Drowsy Orchestra",
        "labels": {"words": "Words written", "lost": "Minutes lost"},
        "counters": [{"var": "words", "label": "Words written"}, {"var": "lost", "of": MAX_LOST, "label": "Minutes lost"}],
        "objective": obj,
        "start": {"title": "Drowsy Orchestra",
                  "text": "The clockmaker stopped time. Walk to his stopped clock at the far end. Write words on the walls with chalk (E): the wall obeys the word. Fewer words, fewer lost minutes: a quieter run.",
                  "button": "Start"},
        "end": {"victory": {"title": "The clock hears you", "text": "{words} words written, {lost} minutes lost. Quieter next time?", "button": "Play again"},
                "default": {"title": "The day is gone", "text": "The Tock ate five minutes. {words} words written.", "button": "Try again"}},
    }


def sims():
    def go(name, script, expect, secs=60):
        return {"name": name, "spawn_group": "solo", "players": [{"id": "p1", "character": "human", "spawn": "door"}],
                "script": script, "expect": expect, "max_seconds": secs}
    p = "p1"

    def chalk(k, w):
        at = [TILE_X[k][w], 1.0, TILE_Z[k]]
        return [{"player": p, "approach": f"tile_{k}_{w}"},
                {"player": p, "hold": {"look_at": at, "seconds": 0.2}},
                {"player": p, "hold": {"look_at": at, "seconds": 0.2, "interact": True}},
                {"player": p, "wait": 0.3}]

    def wait(t):
        return {"player": p, "wait": t}

    def hop(z):
        return [{"player": p, "walk": f"0,{z - 1.2}"}, {"player": p, "hold": {"forward": 1, "jump": True, "seconds": 0.9}}]

    s = []
    s.append(go("MELT written on wall 1 melts it, and you walk through",
                chalk(1, "MELT") + [{"player": p, "walk": "0,13"}],
                [{"collision_disabled": "wall_1"}, {"shown": "w1_MELT"}, {"var": "words", "eq": 1}, {"player": p, "near": [0, 13], "tol": 1.0}, {"event": "wrote", "count": 1}], 30))
    s.append(go("the melted wall closes again by itself",
                chalk(1, "MELT") + [wait(9)],
                [{"collision_enabled": "wall_1"}, {"hidden": "w1_MELT"}, {"event": "closed", "min": 1}, {"var": "words", "eq": 1}], 30))
    s.append(go("LOW leaves a hop-high wall that stays",
                chalk(1, "MELT") + [{"player": p, "walk": "0,12"}] + chalk(2, "LOW") + [wait(9)],
                [{"collision_disabled": "wall_2"}, {"collision_enabled": "low_2"}, {"shown": "w2_LOW"}, {"var": "words", "eq": 2}], 40))
    s.append(go("bell rings after 10 s in chamber 2; left-click picks HUSH",
                chalk(1, "MELT") + [{"player": p, "walk": "0,12"}, wait(10.5), {"player": p, "hold": {"attack": True, "seconds": 0.2}}, wait(0.5)],
                [{"event": "bell", "min": 1}, {"var": "law", "eq": 1}, {"var": "choosing", "eq": 0}, {"shown": "law2_1"}, {"hidden": "law2_0"}], 40))
    s.append(go("SLEEP law makes words cost 2",
                chalk(1, "MELT") + [{"player": p, "walk": "0,12"}, wait(10.5), {"player": p, "hold": {"crouch": True, "seconds": 0.3}}, wait(0.3)]
                + chalk(2, "LOW"),
                [{"var": "law", "eq": 3}, {"var": "words", "eq": 3}], 40))
    s.append(go("win: MELT, LOW, then STILL freezes the Tock, LOW and hop the last wall",
                chalk(1, "MELT") + [{"player": p, "walk": "0,12"}] + chalk(2, "LOW") + hop(22) + [{"player": p, "walk": "0,23.5"}]
                + chalk(3, "LOW") + chalk(3, "STILL") + [{"player": p, "walk": "0,34"}] + hop(38) + [{"player": p, "walk": "0,42"}],
                [{"ended": "victory"}, {"var": "lost", "eq": 0}, {"var": "words", "eq": 4}], 90))
    s.append(go("lose: stand in the Tock's path five times and the day is gone",
                chalk(1, "MELT") + [{"player": p, "walk": "0,12"}] + chalk(2, "LOW") + hop(22) + [{"player": p, "walk": "0,23.5"}]
                + sum([[{"player": p, "walk": "0,31.5"}, wait(5), {"player": p, "walk": "0,34.5"}, wait(0.5)] for _ in range(6)], []),
                [{"ended": "defeat"}, {"var": "lost", "eq": 5}], 120))
    return s


def scene():
    return {
        "recipe": {"title": "Drowsy Orchestra",
                   "summary": "Chalk words on walls: the wall behaves like the word (MELT, LOW, STILL) while a ten-second bell makes you pick a law and a clock-faced beast eats your minutes.",
                   "teaches": ["pickup of a loose prop as a verb (a word)", "timers that restart themselves (the bell)", "a beast driven by `every` rules and zone columns"]},
        "meta": {"fps": 30, "duration": 6, "resolution": [1280, 720]},
        "style": {
            "palette": {"ember": "#ff7a3d", "moss": "#7fd36b", "frost": "#7fc8ff", "bone": "#f3e6c4", "brass": "#d9a441", "slate": "#2b2740",
                        "primary": "#2b2740", "secondary": "#d9a441", "accent": "#ff7a3d", "wood": "#4a2f27", "metal": "#d9a441",
                        "fabric": "#6a4a8a", "floor": "#25203a", "wall": "#3d3560", "trim": "#f3e6c4", "lamp": "#ffc27a"},
            "materials": {"flagstone": {"color": "@floor", "roughness": 0.6, "metallic": 0.1},
                          "plaster": {"color": "@wall", "roughness": 0.9},
                          "gate_wall": {"color": "#8f7ab8", "roughness": 0.8},
                          "gate_low": {"color": "#8f7ab8", "roughness": 0.8},
                          "brass_dark": {"color": "@brass", "metallic": 0.8, "roughness": 0.4}},
            "light": {"ambient": {"color": "#8a7ab8", "intensity": 0.9}, "lamp": {"color": "#ffc27a", "intensity": 12}},
            "sky": {"top": "#0c0818", "bottom": "#2a1a3a"},
            "post": {"ao": 0.8, "outline": 0.5},
        },
        "camera": {"fov": 75, "position": [0, 1.7, 1.0], "target": [0, 1.5, 8.0]},
        "player": {"character": "human", "walk_speed": 4, "jump_speed": 4.5},
        "post": {"ao": 0.8, "outline": 0.5},
        "lights": lamps(),
        "zones": zones(),
        "spawns": [{"id": "door", "position": [0, 0, 1.5], "yaw_deg": 0, "group": "solo"},
                   {"id": "c3", "position": [0, 0, 24.0], "yaw_deg": 0, "group": "solo"}],
        "vars": {"words": 0, "lost": 0, "law": 0, "choosing": 0, "bellglow": 0, "ch": 1, "still": 0, "beast_on": 0,
                 "bi": 0, "bdir": 1, "bc": 0, "px": -1, "hit": 0, "inwall_1": 0, "inwall_2": 0, "inwall_3": 0,
                 "lowed_1": 0, "lowed_2": 0, "lowed_3": 0},
        "prefabs": prefabs(),
        "ui": ui(),
        "rules": rules(),
        "audio": {"music": {"dawn": "audio/tick.json", "day": "audio/tick.json", "dusk": "audio/tick.json", "night": "audio/tick.json"},
                  "music_volume": 0.6,
                  "layers": [{"score": "audio/bell.json", "var": "bellglow", "above": 0.5, "fade": 0.3, "volume": 0.7}]},
        "music": False,
        "hud": {"show_rules_vars": False},
        "objects": objects(),
        "phases": {"open": [f"write_{k}_MELT" for k in WALLS]},
        "checks": {"lint": {"max_errors": 0, "max_warnings": 0, "ignore": ["floating", "interest", "headroom"]}, "sim": sims()},
    }


TICK = {"bpm": 60, "beats": 4, "bars": 8, "key": "C", "scale": "minor_pentatonic", "seed": 21, "lufs": -26,
        "reverb": {"decay": 4, "size": 1.2, "damp": 0.5, "predelay": 0.02, "mix": 0.4},
        "instruments": {
            "tock": {"seconds": 0.5, "level": 0.4, "layers": [{"sine": 1, "decay": 18}, {"sine": 2.01, "decay": 25, "gain": 0.3}]},
            "pad": {"seconds": 6, "level": 0.4, "layers": [{"sine": 1, "attack": 1.2, "release": 2.5}, {"sine": 1.005, "attack": 1.2, "release": 2.5, "gain": 0.7},
                                                         {"sine": 2, "attack": 1.5, "release": 2.5, "gain": 0.2}]}},
        "tracks": [{"inst": "tock", "play": "pattern", "pattern": "x... x... x... x...", "notes": "5 1", "octave": 4, "gain": 0.8},
                   {"inst": "pad", "play": "chords", "chords": "i VI iv v", "every": "2 bars", "octave": 2, "gain": 0.8, "send": 0.5},
                   {"inst": "tock", "play": "pattern", "pattern": ".... ..x. .... ....", "notes": "3 5 8", "octave": 5, "gain": 0.3, "pan": "spread"}]}

BELL = {"bpm": 30, "beats": 4, "bars": 2, "key": "C", "scale": "minor_pentatonic", "seed": 3, "lufs": -24,
        "reverb": {"decay": 3, "size": 1.0, "damp": 0.4, "predelay": 0.02, "mix": 0.3},
        "instruments": {"bell": {"seconds": 4, "level": 0.6, "layers": [
            {"sine": 1, "harmonics": [[2.76, 0.5], [5.4, 0.3]], "decay": 1.1, "release": 1.0}]}},
        "tracks": [{"inst": "bell", "play": "pattern", "pattern": "x... .... .... ....", "notes": "1", "octave": 4, "gain": 0.9}]}


def main():
    out = HERE / "audio"
    out.mkdir(exist_ok=True)
    (out / "tick.json").write_text(json.dumps(TICK, indent=1) + "\n")
    (out / "bell.json").write_text(json.dumps(BELL, indent=1) + "\n")
    (HERE / "drowsy-orchestra.json").write_text(json.dumps(scene(), indent=1) + "\n")


if __name__ == "__main__":
    main()
