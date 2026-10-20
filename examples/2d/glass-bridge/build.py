#!/usr/bin/env python3
"""Glass Bridge builder: writes ../glass-bridge.game2d.json (a RedEngine 2D game).

PLUCK. A robot gardener launches itself like an arrow (arrow keys): it flies straight until a hedge stops it, and every glass string it crosses
rings for a few seconds. Make EVERY string ring at the same moment: that is the loudest sound the level can make.
Levels are grids of 10 px cells. A breadth-first solver proves each level can be won in a few shots and writes the winning scenario scripts, so the
checks in the game file come from the same grids. Levels 3-5 are searched with fixed seeds (the same game every run).

  python3 build.py          write the game
"""
import json
import random
from collections import deque
from pathlib import Path

HERE = Path(__file__).resolve().parent
OUT = HERE.parent / "glass-bridge.game2d.json"
T, TOP, W, H = 10, 20, 32, 16
SPEED = 230
RING = 9.0
NOTES = [("C", 262), ("D", 294), ("E", 330), ("G", 392), ("A", 440), ("high C", 523)]
GLASS = ["#7fe3ff", "#9dffc4", "#fff08a", "#ffb27a", "#ff8fc7", "#c9a6ff"]
DIRS = {"left": (-1, 0), "right": (1, 0), "up": (0, -1), "down": (0, 1)}


class Lvl:
    def __init__(self, name, hint):
        self.name, self.hint = name, hint
        self.open = set()          # free cells
        self.strings = []          # (k, orient 'v'|'h', x, y) top/left cell
        self.sparks = set()
        self.start = (1, 1)

    def carve(self, x0, y0, x1, y1):
        self.open |= {(x, y) for x in range(x0, x1 + 1) for y in range(y0, y1 + 1)}

    def cells(self, s):
        k, o, x, y = s
        return [(x + i, y) if o == "h" else (x, y + i) for i in range(4)]

    def center(self, s):
        k, o, x, y = s
        return (x * T + 5 + (15 if o == "h" else 0), TOP + y * T + 5 + (15 if o == "v" else 0))


def cellpos(c):
    return (c[0] * T + 5, TOP + c[1] * T + 5)


def slide(L, pos, d):
    """The cells flown through (after pos) when launched in direction d, or None when a spark is hit."""
    x, y = pos
    out = []
    while (x + d[0], y + d[1]) in L.open:
        x, y = x + d[0], y + d[1]
        if (x, y) in L.sparks:
            return None, (x, y)
        out.append((x, y))
    return out, (x, y)


def solve(L):
    """Fewest shots that cross every string; returns the list of directions."""
    cellmap = {}
    for i, s in enumerate(L.strings):
        for c in L.cells(s):
            cellmap.setdefault(c, []).append(i)
    full = (1 << len(L.strings)) - 1
    start = (L.start, 0)
    seen = {start: []}
    q = deque([start])
    while q:
        pos, mask = q.popleft()
        path = seen[(pos, mask)]
        if mask == full:
            return path
        for name, d in DIRS.items():
            cells, end = slide(L, pos, d)
            if cells is None or end == pos:
                continue
            m = mask
            for c in cells:
                for i in cellmap.get(c, []):
                    m |= 1 << i
            st = (end, m)
            if st not in seen:
                seen[st] = path + [name]
                q.append(st)
    return None


def room(name, hint, *rects):
    L = Lvl(name, hint)
    for r in rects:
        L.carve(*r)
    return L


def level1():
    L = room("1", "Press the RIGHT ARROW: fly through the strings!", (1, 6, 30, 9))
    L.start = (3, 7)
    for k, x in enumerate((10, 16, 22)):
        L.strings.append((k, "v", x, 6))
    return L


def level2():
    L = room("2", "You fly until a hedge stops you. Then fly again!", (1, 6, 29, 9), (26, 1, 29, 5))
    L.start = (3, 7)
    for k, x in zip((0, 1, 2), (8, 14, 20)):
        L.strings.append((k, "v", x, 6))
    L.strings.append((3, "h", 26, 3))
    return L


def searched(seed, nstr, shots, sparks, name, hint):
    """A pillar room whose cheapest answer is exactly `shots` shots."""
    rnd = random.Random(seed)
    while True:
        L = room(name, hint, (1, 1, 30, 14))
        L.start = (rnd.randrange(2, 30), rnd.randrange(2, 14))
        for _ in range(rnd.randrange(18, 30)):
            c = (rnd.randrange(1, 31), rnd.randrange(1, 15))
            if abs(c[0] - L.start[0]) + abs(c[1] - L.start[1]) > 1:
                L.open.discard(c)
        taken = set()
        ok = True
        for k in range(nstr):
            for _ in range(40):
                o = rnd.choice("vh")
                s = (k, o, rnd.randrange(1, 31), rnd.randrange(1, 15))
                cs = L.cells(s)
                if all(c in L.open and c not in taken and c != L.start for c in cs):
                    L.strings.append(s)
                    taken |= set(cs) | {(c[0] + dx, c[1] + dy) for c in cs for dx in (-1, 0, 1) for dy in (-1, 0, 1)}
                    break
            else:
                ok = False
        if not ok:
            continue
        for _ in range(sparks):
            c = (rnd.randrange(1, 31), rnd.randrange(1, 15))
            if c in L.open and c not in taken and c != L.start:
                L.sparks.add(c)
        if len(L.sparks) != sparks:
            continue
        sol = solve(L)
        if sol is not None and len(sol) == shots:
            return L


def all_levels():
    return [level1(), level2(),
            searched(11, 4, 3, 0, "3", "Plan your shots, then ring them all together."),
            searched(23, 5, 4, 3, "4", "Sparks send you back to the start. Avoid them!"),
            searched(35, 6, 5, 5, "5", "Last one: six strings. Make the loudest sound!")]


# ---- art ------------------------------------------------------------------------------------------------------------------------------------------
ROBOT = {"rows": [".bb.b.bb.", "..bbbbb..", ".bwwbwwb.", ".bwkbkwb.", ".bbbbbbb.", "gbbyyybbg", ".bbbbbbb.", "..b...b..", ".kk...kk."],
         "palette": {"b": "#c9d3dc", "w": "#ffffff", "k": "#22303c", "y": "#ffd23f", "g": "#5fd36b"}}
HEDGE = {"rows": ["gGgGggGGgg", "GggGGggGgG", "gGGggGgggG", "ggGgGGGggg", "GgGggGgGgG", "gGgGgggGGg", "GggGGgGggG", "gGggGgGgGg", "GgGgGGggGg", "ggGGggGgGg"],
         "palette": {"g": "#2c7a3b", "G": "#3da34e"}}
SPARK = {"rows": ["..r..r..", ".rorrorr", "rrooooro", ".rooyoor", "rooyyoor", ".roooorr", "rrorroro", "..r..r.."],
         "palette": {"r": "#e8403a", "o": "#ff8a3d", "y": "#fff3a0"}, "fps": 6}


def sounds():
    s = {"launch": {"seconds": 0.25, "level": 0.5, "layers": [{"sweep": [300, 900, 6], "decay": 9}, {"noise": "hp 0.5", "decay": 22, "gain": 0.3}]},
         "thunk": {"seconds": 0.15, "level": 0.5, "layers": [{"sine": 110, "decay": 20, "attack": 0.004, "release": 0.05}, {"noise": "hp 0.3", "decay": 30, "gain": 0.25}]},
         "zap": {"seconds": 0.4, "level": 0.5, "layers": [{"sweep": [900, 200, 5], "decay": 7}, {"noise": "hp 0.4", "decay": 14, "gain": 0.4}]},
         "reset": {"seconds": 0.2, "level": 0.4, "layers": [{"sweep": [500, 200, 6], "decay": 10}]},
         "chord": {"seconds": 3.2, "level": 0.95, "layers": [{"sine": hz, "harmonics": [[2.0, 0.3], [3.0, 0.15], [2.76, 0.12]], "decay": 1.1, "gain": 0.5}
                                                           for _, hz in NOTES] + [{"noise": "lp 0.5", "decay": 6, "gain": 0.35},
                                                                                  {"sine": 65, "decay": 2.2, "gain": 0.8}]}}
    for k, (n, hz) in enumerate(NOTES):
        s[f"note{k}"] = {"seconds": 1.6, "level": 0.6, "layers": [{"sine": hz, "harmonics": [[2.0, 0.35], [3.0, 0.15], [4.2, 0.07]], "decay": 2.2}]}
    return s


def prefabs():
    p = {"robot": {"tag": "player", "shape": {"sprite": "robot"}, "size": [9, 9], "layer": 5,
                   "body": {"type": "dynamic", "gravity": 0, "bounce": 0, "friction": 0, "drag": 0, "max_fall": 900}, "collide": ["wall"]},
         "wall": {"tag": "wall", "shape": {"sprite": "hedge"}, "size": [10, 10], "body": {"type": "static"}},
         "spark": {"tag": "spark", "shape": {"sprite": "spark"}, "size": [8, 8], "layer": 4,
                   "emit": {"rate": 5, "life": [0.3, 0.6], "speed": [4, 14], "angle": [0, 360], "color": "#ff8a3d"}}}
    for k, col in enumerate(GLASS):
        for o, sz, rsz in (("v", [3, 40], [9, 44]), ("h", [40, 3], [44, 9])):
            p[f"s{o}{k}"] = {"tag": ["string", f"s{k}", f"s{o}{k}"], "shape": {"rect": sz, "color": col + "bb"}, "size": sz, "layer": 2}
            p[f"r{o}{k}"] = {"tag": ["ring", f"r{k}"], "shape": {"rect": rsz, "color": col + "55"}, "size": rsz, "layer": 3, "ttl": RING,
                             "emit": {"rate": 22, "life": [0.3, 0.7], "speed": [8, 26], "angle": [0, 360], "color": col}}
    return p


def scene_for(L):
    sc = [{"prefab": "robot", "at": list(cellpos(L.start)), "id": "p"}]
    for s in L.strings:
        sc.append({"prefab": f"s{s[1]}{s[0]}", "at": list(L.center(s))})
    for c in sorted(L.sparks):
        sc.append({"prefab": "spark", "at": list(cellpos(c))})
    rows = ["".join("." if (x, y) in L.open else "#" for x in range(W)) for y in range(H)]
    return {"id": L.name, "scene": sc, "map": {"tile": T, "origin": [0, TOP], "rows": rows, "legend": {"#": "wall"}}, "world": [W * T, TOP + H * T],
            "background": "#14221a"}


def rules(levels):
    r = []
    n = len(levels)
    ready = "flying == 0 && won == 0"
    for d, v in (("left", [-SPEED, 0]), ("right", [SPEED, 0]), ("up", [0, -SPEED]), ("down", [0, SPEED])):
        r.append({"id": f"launch {d}", "when": {"press": d}, "if": ready,
                  "do": [{"velocity": {"target": "id:p", "v": v}}, {"set": ["flying", 1]}, {"add": ["shots", 1]}, {"add": ["total", 1]}, {"play": "launch"}]})
    r.append({"id": "stop at hedge", "when": {"hit": ["player", "wall"]},
              "do": [{"velocity": {"target": "self", "v": [0, 0]}}, {"set": ["flying", 0]}, {"play": "thunk"}, {"shake": 1},
                     {"teleport": {"target": "self", "to": [f"floor(self_x / {T}) * {T} + 5", f"floor((self_y - {TOP}) / {T}) * {T} + {TOP + 5}"]}},
                     {"burst": {"at": "self", "n": 5, "color": "#8fd694", "speed": [10, 40], "life": [0.2, 0.4]}}]})
    for k, (nm, hz) in enumerate(NOTES):
        for o in "vh":
            r.append({"id": f"pluck {nm} {o}", "when": {"touch": ["player", f"s{o}{k}"]}, "if": f"count_r{k} == 0",
                      "do": [{"spawn": {"prefab": f"r{o}{k}", "at": "other"}}, {"play": f"note{k}"}, {"add": ["plucks", 1]},
                             {"burst": {"at": "other", "n": 10, "color": GLASS[k], "speed": [20, 70], "life": [0.3, 0.7]}}]})
    r.append({"id": "level number", "when": {"every": 0.05}, "do": [{"set": ["lvn", "level + 1"]}]})
    r.append({"id": "loudest", "when": {"every": 0.05}, "if": "count_string > 0 && count_ring >= count_string && won == 0",
              "do": [{"set": ["won", 1]}, {"set": ["wt", "time"]}, {"play": "chord"}, {"shake": 7}, {"set": ["flying", 0]},
                     {"burst": {"at": [160, 100], "n": 60, "color": "#fff3a0", "speed": [40, 160], "life": [0.6, 1.4], "size": 2}}]})
    r.append({"id": "next level", "when": {"every": 0.05}, "if": f"won == 1 && time - wt > 1.8 && level < {n - 1}",
              "do": [{"set": ["won", 0]}, {"set": ["shots", 0]}, {"set": ["flying", 0]}, {"level": "level + 1"}]})
    r.append({"id": "the end", "when": {"every": 0.05}, "if": f"won == 1 && time - wt > 2.2 && level == {n - 1}",
              "do": [{"set": ["won", 2]}, {"apply": "record"}, {"end": "win"}]})
    r.append({"id": "reset", "when": {"event": "reset"}, "if": "won == 0",
              "do": [{"destroy": "tag:ring"}, {"velocity": {"target": "id:p", "v": [0, 0]}}, {"set": ["flying", 0]}, {"play": "reset"}]})
    for i, L in enumerate(levels):
        x, y = cellpos(L.start)
        home = [{"teleport": {"target": "id:p", "to": [x, y]}}]
        r.append({"id": f"reset home {L.name}", "when": {"event": "reset"}, "if": f"won == 0 && level == {i}", "do": home})
        if L.sparks:
            r.append({"id": f"spark {L.name}", "when": {"touch": ["player", "spark"]}, "if": f"level == {i} && won == 0",
                      "do": [{"destroy": "tag:ring"}, {"velocity": {"target": "id:p", "v": [0, 0]}}, {"set": ["flying", 0]}, {"add": ["zaps", 1]},
                             {"play": "zap"}, {"shake": 4}, {"burst": {"at": "self", "n": 24, "color": "#ff8a3d", "speed": [30, 110], "life": [0.3, 0.7]}}] + home})
    return r


def ui(levels):
    n = len(levels)
    u = [{"panel": {"at": [0, 0], "size": [320, TOP], "color": "#0b140fee"}},
         {"panel": {"at": [0, 169], "size": [320, 11], "color": "#0b140fee"}},
         {"text": "RING EVERY STRING AT ONCE!", "at": [4, 2], "color": "#ffffff"},
         {"text": "RINGING {count_ring} / {count_string}", "at": [316, 2], "color": "#fff08a", "align": "right"},
         {"text": "LEVEL {lvn} of %d" % n, "at": [4, 172], "color": "#7fe3ff"},
         {"text": "SHOTS {shots}", "at": [84, 172], "color": "#c9d3dc"},
         {"text": "BEST {best}", "at": [130, 172], "color": "#c9d3dc", "show": "best > 0"},
         {"text": "ARROWS: fly    R: start over", "at": [316, 172], "color": "#c9ffd0", "align": "right"},
         {"button": {"id": "reset", "label": "", "at": [0, 0], "size": [1, 1], "key": "KeyR", "do": [{"emit": "reset"}]}}]
    for i, L in enumerate(levels):
        u.append({"text": L.hint, "at": [4, 11], "color": "#d6e4ff", "show": f"level == {i} && won == 0"})
    u.append({"text": "GLASS BRIDGE", "at": [160, 40], "color": "#9dffc4", "scale": 2, "align": "center", "show": "level == 0 && shots == 0"})
    u.append({"text": "LOUDEST SOUND!", "at": [160, 80], "color": "#fff3a0", "scale": 2, "align": "center", "show": "won > 0"})
    return u


def scenarios(levels):
    sc = []

    def steps(L, sol):
        out = []
        for d in sol:
            out += [{"press": d}, {"wait": 0.1}, {"wait_until": {"var": "flying", "eq": 0}, "timeout": 4}]
        return out

    L1 = levels[0]
    sc.append({"name": "launch: one arrow shot rings all three strings", "seed": 1, "max_seconds": 10, "level": "1",
               "script": steps(L1, solve(L1)),
               "expect": [{"var": "plucks", "gte": 3}, {"var": "shots", "eq": 1}, {"var": "won", "eq": 1}, {"sound": "chord", "min": 1}]})
    for i, L in enumerate(levels[1:], 1):
        sol = solve(L)
        sc.append({"name": f"level {L.name}: {len(sol)} shots ring every string at once", "seed": 1, "max_seconds": 30, "level": L.name, "script":
                   [{"wait": 0.1}] + steps(L, sol), "expect": [{"var": "won", "gte": 1}, {"sound": "chord", "min": 1}]})
    # a pluck is not enough: strings that were rung once do not win by themselves
    sc.append({"name": "half the strings is not the loudest sound", "seed": 1, "max_seconds": 10, "level": "2",
               "script": [{"press": "right"}, {"wait_until": {"var": "flying", "eq": 0}, "timeout": 4}, {"wait": 0.3}],
               "expect": [{"var": "plucks", "eq": 3}, {"var": "won", "eq": 0}, {"not_ended": True}]})
    # lose path: sparks
    for i, L in enumerate(levels):
        if L.sparks:
            for name, d in DIRS.items():
                cells, end = slide(L, L.start, d)
                if cells is None:
                    sc.append({"name": "a spark sends the robot home and silences the strings", "seed": 1, "max_seconds": 10, "level": L.name,
                               "script": [{"wait": 0.1}, {"press": name}, {"wait": 1.0}],
                               "expect": [{"var": "zaps", "gte": 1}, {"entity": "p", "near": list(cellpos(L.start)), "tol": 4}, {"count": "ring", "eq": 0}]})
                    break
            break
    # the whole game
    full = [{"wait": 0.1}]
    for i, L in enumerate(levels):
        full += steps(L, solve(L)) + [{"wait": 4.5}]
    sc.append({"name": "all five levels to the final chord", "seed": 1, "max_seconds": 120, "smoke": True, "script": full,
               "expect": [{"ended": "win"}, {"var": "best", "gt": 0}]})
    return sc


def main():
    levels = all_levels()
    g = {"game2d": 1, "id": "glass-bridge", "title": "Glass Bridge",
         "description": "A robot gardener launches itself like an arrow through glass strings in an empty park: ring every string at once for the loudest sound.",
         "capabilities": {"presentation": "2d", "platforms": ["windows", "linux"], "networking": "offline", "input": ["keyboard", "mouse"],
                          "persistence": ["progress"]},
         "view": {"width": 320, "height": 180, "background": "#14221a", "scale": "fit"},
         "icon": "robot",
         "sprites": {"robot": ROBOT, "hedge": HEDGE, "spark": SPARK},
         "sounds": sounds(),
         "vars": {"flying": 0, "shots": 0, "total": 0, "plucks": 0, "zaps": 0, "won": 0, "wt": 0, "lvn": 1, "best": 0},
         "persist": ["best"],
         "prefabs": prefabs(),
         "levels": [scene_for(L) for L in levels],
         "effects": {"record": {"do": [{"set": ["best", "(best == 0 || total < best) * total + (best != 0 && total >= best) * best"]}, {"save": True}]}},
         "ui": ui(levels),
         "rules": rules(levels),
         "checks": {"scenarios": scenarios(levels)}}
    OUT.write_text(json.dumps(g, indent=1) + "\n")
    for L in levels:
        print(L.name, len(L.strings), "strings", len(solve(L)), "shots", len(L.sparks), "sparks")


if __name__ == "__main__":
    main()
