#!/usr/bin/env python3
"""Idea Forge builder: expands vocabulary.json into ../idea-forge.game2d.json (a RedEngine 2D game).

  python3 build.py              write ../idea-forge.game2d.json
  python3 build.py check        vocabulary lint: font characters, line counts, duplicates, walk is a full cycle
  python3 build.py decode CODE  print the idea behind a CODE shown in the game (the 10-digit number on the card)
  python3 build.py walk N [SEED]  print the first N ideas of the walk (default seed 12345)

The game cannot hold strings in variables and has no random expression (see the feedback notes), so the whole design is arithmetic on
numbers: an idea is ONE integer `cur` in a mixed-radix space (title adjective, title noun, YOU, BUT, PUSHBACK, GOAL, STORY); every forge
adds a constant STEP that is coprime to the size of the space, so the walk visits every idea exactly once before any repeats.
Each possible phrase is a text widget shown only while its digit matches (`show` expressions); the lines are wrapped here, not at run time.
"""
import json, math, sys, textwrap
from pathlib import Path

HERE = Path(__file__).resolve().parent
OUT = HERE.parent / "idea-forge.game2d.json"
VOC = json.loads((HERE / "vocabulary.json").read_text())

# (name, vocabulary key, label, colour, max lines)
SLOTS = [("adj", "adjectives"), ("noun", "nouns"), ("you", "you"), ("but", "but"), ("push", "pushback"), ("goal", "goal"), ("story", "story")]
N = [len(VOC[k]) for _, k in SLOTS]
R = [math.prod(N[:i]) for i in range(len(N))]
M = math.prod(N)
WRAP = 67            # characters a line holds beside the label column (6 px each in a 480 px view)
MAX_LINES = 3
FONT = set("ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_.'-+:)(/=#%!^<>*~?[]↓,$\";&@| ")


def pick_step() -> int:
    """A step near M/phi (so successive ideas differ in every part) that is coprime to M and whose digits are all far from 0 and the radix.
    Skips straight past a run of values whose high digit is out of range instead of counting through them (millions of candidates)."""
    s = int(M * 0.6180339887)
    while True:
        moved = False
        for i in reversed(range(len(N))):
            d = (s // R[i]) % N[i]
            if d < 3:
                s += (3 - d) * R[i] - s % R[i]; moved = True; break
            if d > N[i] - 3:
                s += R[i] - s % R[i]; moved = True; break
        if moved:
            continue
        if math.gcd(s, M) == 1:
            return s
        s += 1


STEP = pick_step()
CAP = 1_000_000      # forges counted before the walk wraps: n * STEP must stay below 2^53 to be exact in the engine's f64
assert CAP * STEP < 2**53 and M < 2**53


def digits(x: int):
    return [(x // R[i]) % N[i] for i in range(len(N))]


def walk(seed: int, n: int) -> int:
    return (seed + (n % CAP) * STEP) % M


def lines_of(text: str):
    return textwrap.wrap(text, WRAP)


def decode(code: int) -> str:
    d = digits(code)
    out = [f"{VOC['adjectives'][d[0]].upper()} {VOC['nouns'][d[1]].upper()}   (code {code:010d})",
           f"YOU {VOC['you'][d[2]]},", f"BUT {VOC['but'][d[3]]}.", f"PUSHBACK  {VOC['pushback'][d[4]]}",
           f"GOAL  {VOC['goal'][d[5]]}", f"STORY  {VOC['story'][d[6]]}"]
    return "\n".join(out)


def check() -> int:
    bad = 0
    for (name, key), vals in zip(SLOTS, (VOC[k] for _, k in SLOTS)):
        if len(set(v.lower() for v in vals)) != len(vals):
            print(f"{key}: duplicate phrase"); bad += 1
        for v in vals:
            miss = {c for c in v.upper() if c not in FONT}
            if miss:
                print(f"{key}: `{v}` has characters the 5x7 font lacks: {sorted(miss)}"); bad += 1
            if len(lines_of(v)) > MAX_LINES:
                print(f"{key}: `{v}` wraps to more than {MAX_LINES} lines"); bad += 1
    for key in ("adjectives", "nouns"):
        if max(len(v) for v in VOC[key]) > 12:
            print(f"{key}: a title word longer than 12 letters would not fit at scale 3"); bad += 1
    # The walk is a full cycle iff gcd(STEP, M) == 1; spot-check by looking for a repeat inside a sample.
    seen, x = set(), 17
    for _ in range(200_000):
        if x in seen:
            print("walk repeated inside 200000 steps"); bad += 1; break
        seen.add(x); x = (x + STEP) % M
    print(f"{'FAIL' if bad else 'ok'}: {M:,} ideas, radices {N}, step {STEP} (gcd {math.gcd(STEP, M)}), {bad} problem(s)")
    return 1 if bad else 0


# ---- expressions -----------------------------------------------------------------------------------------------------------------------------------
def dig(var: str, i: int) -> str:
    """The engine expression for digit i of the integer in variable `var`."""
    return f"{var} % {N[i]}" if i == 0 else f"(({var} - {var} % {R[i]}) / {R[i]}) % {N[i]}"


V = [f"v_{n}" for n, _ in SLOTS]              # displayed digit of each part (spins, then settles on the real one)
LOCKS = {2: "la", 3: "lb", 4: "lc", 5: "ld", 6: "le"}     # parts the player may hold fixed while re-forging
SPIN_STOP = [0.30, 0.38, 0.50, 0.66, 0.82, 0.98, 1.14]    # seconds after the strike at which each reel settles
SPIN_STEP = [7, 5, 11, 13, 7, 11, 5]                       # coprime to each radix so the reel visits every phrase


def build() -> dict:
    vars_ = {"n": 0, "seed": 0, "cur": 0, "w": 0, "story": 1, "fi": 0, "nf": 0, "fn": 0, "hn": 0, "spin": 0, "spin_t": 0, "dups": 0}
    vars_.update({k: 0 for k in LOCKS.values()})
    vars_.update({f"f{j}": 0 for j in range(1, 9)})
    vars_.update({f"h{j}": 0 for j in range(1, 9)})
    vars_.update({v: 0 for v in V})
    persist = ["n", "seed", "cur", "story", "nf", "fn", "fi", "hn"] + list(LOCKS.values()) + [f"f{j}" for j in range(1, 9)] + [f"h{j}" for j in range(1, 9)]

    # --- effects (named action groups) ---
    unpack = [{"set": [V[i], dig("cur", i)]} for i in range(len(V))]
    mixed = " + ".join(
        f"{R[i]} * (" + (f"{LOCKS[i]} * ({dig('cur', i)}) + (1 - {LOCKS[i]}) * ({dig('w', i)})" if i in LOCKS else dig("w", i)) + ")"
        for i in range(len(V)))
    push = [{"set": [f"h{j}", f"h{j - 1}"]} for j in range(8, 1, -1)] + [{"set": ["h1", "cur"]}, {"set": ["hn", "hn + (n > 0) * (hn < 8)"]}]
    ROLL = {"teleport": {"target": "id:die", "to": {"x": [1, 4000000], "y": [1, 4000000]}}}   # the engine's only random source; read next tick
    forge = (
        [{"set": ["w", "die_x * 7919 + die_y + tick * 1000003 + mouse_x * 8191 + mouse_y * 131071"]},   # `play2d` defaults to --seed 1, so the die alone repeats every launch: mix in the timing of the first press and the pointer; fractions are floored below
         {"set": ["seed", f"(seed != 0) * seed + (seed == 0) * (((w - w % 1)) % {M})"]},
         {"set": ["w", f"(seed + (n % {CAP}) * {STEP}) % {M}"]}]
        + push
        + [{"set": ["cur", mixed]},
           {"add": ["n", 1]}, {"set": ["fi", 0]}, {"set": ["spin", 1]}, {"set": ["spin_t", "time"]},
           {"set": ["dups", "dups + (n > 1) * (" + " || ".join(f"cur == h{j}" for j in range(1, 9)) + ")"]},
           {"play": "strike"}, {"shake": 2}, {"destroy": "tag:splash"}, ROLL,
           {"burst": {"at": [240, 44], "n": 36, "color": "#ffcf6a", "speed": 90, "life": 0.7, "size": 1.5, "gravity": 120}}])
    effects = {
        "unpack": {"about": "show the idea in `cur` (all seven parts at once, no spin)", "do": unpack},
        "forge": {"about": "walk to the next idea (parts the player locked stay), remember the old one, and start the reels spinning", "do": forge},
    }

    # --- rules ---
    rules = [
        {"id": "forge-key", "when": {"press": "action"}, "do": [{"apply": "forge"}]},
        {"id": "forge-event", "when": {"event": "forge"}, "do": [{"apply": "forge"}]},
        {"id": "roll-at-start", "when": {"start": True}, "do": [ROLL]},
        {"id": "loaded", "when": {"event": "loaded"}, "do": [{"apply": "unpack"}, {"destroy": "tag:splash"}]},
    ]
    for i, T in enumerate(SPIN_STOP):
        lock = f" && {LOCKS[i]} == 0" if i in LOCKS else ""
        rules.append({"id": f"reel-{SLOTS[i][0]}-spin", "when": {"every": 0.04}, "if": f"spin == 1 && time - spin_t < {T}{lock}",
                      "do": [{"set": [V[i], f"({V[i]} + {SPIN_STEP[i]}) % {N[i]}"]}]})
        rules.append({"id": f"reel-{SLOTS[i][0]}-settle", "when": {"every": 0.04}, "if": f"spin == 1 && time - spin_t >= {T} && {V[i]} != {dig('cur', i)}",
                      "do": [{"set": [V[i], dig("cur", i)]}, {"play": "tick"}]})
    rules.append({"id": "reels-done", "when": {"every": 0.04}, "if": f"spin == 1 && time - spin_t >= {SPIN_STOP[-1] + 0.1}",
                  "do": [{"set": ["spin", 0]}, {"play": "ding"}, {"burst": {"at": [240, 44], "n": 14, "color": "#ffffff", "speed": 40, "life": 0.5, "size": 1, "gravity": 0}}]})
    for slot, var in ((2, "la"), (3, "lb"), (4, "lc"), (5, "ld"), (6, "le")):
        rules.append({"id": f"lock-{var}", "when": {"event": f"lock_{var}"}, "do": [{"set": [var, f"1 - {var}"]}, {"play": "click"}]})
    rules.append({"id": "story-toggle", "when": {"event": "story"}, "do": [{"set": ["story", "1 - story"]}, {"play": "click"}]})
    saved = " || ".join(f"cur == f{j}" for j in range(1, 9))
    for j in range(1, 9):
        rules.append({"id": f"fav-save-{j}", "when": {"event": "fav"}, "if": f"n > 0 && !({saved}) && nf % 8 == {j - 1}",
                      "do": [{"set": [f"f{j}", "cur"]}, {"add": ["nf", 1]}, {"set": ["fn", "nf - (nf > 8) * (nf - 8)"]}, {"play": "chime"}]})
    rules.append({"id": "fav-next", "when": {"event": "nextfav"}, "if": "fn > 0", "do": [{"set": ["fi", "fi % fn + 1"]}, {"play": "click"}]})
    for j in range(1, 9):
        rules.append({"id": f"fav-show-{j}", "when": {"event": "nextfav"}, "if": f"fn > 0 && fi == {j}",
                      "do": [{"set": ["cur", f"f{j}"]}, {"apply": "unpack"}, {"set": ["spin", 0]}]})
    rules.append({"id": "back", "when": {"event": "back"}, "if": "hn > 0",
                  "do": [{"set": ["cur", "h1"]}] + [{"set": [f"h{j}", f"h{j + 1}"]} for j in range(1, 8)] + [{"add": ["hn", -1]}, {"set": ["fi", 0]}, {"apply": "unpack"}, {"set": ["spin", 0]}, {"play": "click"}]})

    # --- ui ---
    ui = []
    BODY, DIM = "#d7def0", "#7f8bb0"
    COL = {2: "#7fe3ff", 3: "#ffb347", 4: "#ff7a8c", 5: "#8ff0a4", 6: "#c3a6ff"}
    ui.append({"panel": {"at": [0, 0], "size": [480, 14], "color": "#1a2140"}})
    ui.append({"text": "IDEA FORGE", "at": [6, 4], "color": "#ffb347", "scale": 1})
    ui.append({"text": "FORGED {n}", "at": [150, 4], "color": DIM, "scale": 1, "show": "n > 0"})
    ui.append({"text": "SAVED {fn}", "at": [226, 4], "color": DIM, "scale": 1, "show": "n > 0 && fi == 0"})
    ui.append({"text": "SAVED", "at": [474, 4], "color": "#8ff0a4", "scale": 1, "align": "right", "show": f"n > 0 && ({saved})"})
    ui.append({"text": "CODE {cur:10}", "at": [474, 4], "color": DIM, "scale": 1, "align": "right", "show": f"n > 0 && !({saved})"})
    # splash while nothing has been forged
    ui.append({"text": "IDEA FORGE", "at": [240, 24], "color": "#ffb347", "scale": 4, "align": "center", "show": "n == 0"})
    for k, line in enumerate(["EACH STRIKE HAMMERS OUT ONE GAME IDEA BUILT AROUND A SINGLE STRANGE MECHANIC.",
                              "A STORY COMES WITH IT UNLESS YOU SWITCH IT OFF.",
                              "NO IDEA REPEATS: THE FORGE WALKS EVERY COMBINATION EXACTLY ONCE."]):
        ui.append({"text": line, "at": [240, 140 + 12 * k], "color": BODY, "scale": 1, "align": "center", "show": "n == 0"})
    ui.append({"text": "PRESS SPACE TO STRIKE", "at": [240, 190], "color": "#ffcf6a", "scale": 2, "align": "center", "show": "n == 0"})
    # the title: adjective right-aligned and noun left-aligned around the centre
    for i in range(len(VOC["adjectives"])):
        ui.append({"text": VOC["adjectives"][i], "at": [231, 22], "color": "#ffcf6a", "scale": 3, "align": "right", "show": f"n > 0 && {V[0]} == {i}"})
    for i in range(len(VOC["nouns"])):
        ui.append({"text": VOC["nouns"][i], "at": [249, 22], "color": "#ffffff", "scale": 3, "align": "left", "show": f"n > 0 && {V[1]} == {i}"})
    # the five sections
    top = {2: 58, 3: 92, 4: 126, 5: 160, 6: 194}
    labels = {2: "YOU", 3: "BUT", 4: "PUSHBACK", 5: "GOAL", 6: "STORY"}
    keys = {2: "Digit1", 3: "Digit2", 4: "Digit3", 5: "Digit4", 6: "Digit5"}
    for i in range(2, 7):
        y = top[i]
        gate = "n > 0" + (" && story == 1" if i == 6 else "")
        ui.append({"panel": {"at": [2, y - 3], "size": [476, 33], "color": "#10162a" if i % 2 else "#141b33"}, "show": gate})
        ui.append({"text": labels[i], "at": [6, y], "color": COL[i], "scale": 1, "show": gate})
        for p, phrase in enumerate(VOC[SLOTS[i][1]]):
            text = phrase
            if i == 2:
                text = "you " + phrase + ","
            elif i == 3:
                text = "but " + phrase + "."
            for ln, line in enumerate(lines_of(text)):
                ui.append({"text": line, "at": [70, y + 9 * ln], "color": BODY, "scale": 1, "show": f"{gate} && {V[i]} == {p}"})
    ui.append({"text": "STORY OFF. JUST THE MECHANIC.", "at": [70, 194], "color": DIM, "scale": 1, "show": "n > 0 && story == 0"})
    for i, var in LOCKS.items():
        y = top[i]
        gate = "n > 0" + (" && story == 1" if i == 6 else "")
        ui.append({"button": {"id": f"lock_{var}", "label": f"LOCK {i - 1}", "at": [6, y + 10], "size": [46, 11], "key": keys[i], "color": "#26305a", "do": [{"emit": f"lock_{var}"}]}, "show": gate})
        ui.append({"text": "LOCKED", "at": [6, y + 23], "color": "#ff7a8c", "scale": 1, "show": f"{gate} && {var} == 1"})
    # footer
    ui.append({"button": {"id": "forge", "label": "FORGE", "at": [6, 244], "size": [56, 18], "key": "Enter", "color": "#7a3b12", "do": [{"emit": "forge"}]}})
    ui.append({"button": {"id": "story", "label": "STORY", "at": [68, 244], "size": [44, 18], "key": "KeyS", "color": "#26305a", "do": [{"emit": "story"}]}})
    ui.append({"button": {"id": "fav", "label": "SAVE", "at": [118, 244], "size": [38, 18], "key": "KeyF", "color": "#26305a", "do": [{"emit": "fav"}]}})
    ui.append({"button": {"id": "nextfav", "label": "SAVED", "at": [162, 244], "size": [44, 18], "key": "KeyV", "color": "#26305a", "do": [{"emit": "nextfav"}]}})
    ui.append({"button": {"id": "back", "label": "BACK", "at": [212, 244], "size": [38, 18], "key": "KeyB", "color": "#26305a", "do": [{"emit": "back"}]}})
    ui.append({"button": {"id": "music", "label": "MUSIC", "at": [256, 244], "size": [44, 18], "key": "KeyM", "color": "#26305a", "do": [{"music": "toggle"}]}})
    ui.append({"text": "OFF", "at": [296, 250], "color": "#ff7a8c", "scale": 1, "align": "right", "show": "!music_on"})
    ui.append({"text": "SPACE FORGE   1-5 LOCK   S STORY   F SAVE   V SAVED   B BACK   M MUSIC", "at": [240, 231], "color": DIM, "scale": 1, "align": "center"})
    ui.append({"text": "SAVED IDEA {fi}/{fn}", "at": [226, 4], "color": "#8ff0a4", "scale": 1, "show": "fi > 0"})

    sprites = {
        "anvil": {"palette": {"a": "#8a97b5", "d": "#4a5570", "h": "#c9d4ee"},
                  "rows": ["..hhhhhhhhhhhh..", "hhhhaaaaaaaaaaaa", ".hhhaaaaaaaaaaa.", "..aaaaaaaaaaaa..", ".....aaaaaa.....", ".....aaaaaa.....", "....dddddddd....", "...dddddddddd..."]},
    }
    sounds = {
        "strike": {"seconds": 0.5, "level": 0.6, "layers": [{"sine": 196, "decay": 9, "attack": 0.002}, {"sine": 523, "decay": 7, "attack": 0.002, "gain": 0.5},
                                                           {"sine": 1318, "decay": 14, "attack": 0.002, "gain": 0.3}]},
        "tick": {"seconds": 0.06, "level": 0.35, "layers": [{"sine": 1500, "decay": 60, "attack": 0.001}]},
        "ding": {"seconds": 0.6, "level": 0.45, "layers": [{"sine": 880, "decay": 7, "attack": 0.003}, {"sine": 1320, "decay": 8, "delay": 0.09, "attack": 0.003, "gain": 0.7}]},
        "click": {"seconds": 0.05, "level": 0.3, "layers": [{"sine": 640, "decay": 70, "attack": 0.001}]},
        "chime": {"seconds": 0.5, "level": 0.4, "layers": [{"sine": 660, "decay": 8, "attack": 0.003}, {"sine": 990, "decay": 9, "delay": 0.07, "attack": 0.003}, {"sine": 1320, "decay": 10, "delay": 0.14, "attack": 0.003}]},
    }
    music = {
        "ember": {
            "bpm": 66, "beats": 4, "bars": 8, "key": "A", "scale": "minor_pentatonic", "seed": 7, "lufs": -29,
            "reverb": {"decay": 3.0, "mix": 0.4},
            "instruments": {
                "pad": {"seconds": 6, "level": 0.5, "layers": [{"sine": 1, "attack": 1.5, "release": 2.5}, {"sine": 2, "attack": 2, "release": 2.5, "gain": 0.25}, {"sine": 0.5, "attack": 2, "release": 2.5, "gain": 0.4}]},
                "spark": {"seconds": 2, "level": 0.4, "layers": [{"sine": 1, "decay": 3}, {"sine": 3.01, "decay": 6, "gain": 0.2}]},
            },
            "tracks": [
                {"inst": "pad", "play": "chords", "chords": "1 6 4 5", "every": "2 bars", "octave": 3, "gain": 0.9},
                {"inst": "spark", "play": "walk", "every": "1 beats", "density": 0.25, "range": [1, 8], "octave": 5, "pan": "spread", "gain": 0.5, "vel": [0.4, 0.8]},
            ],
        }
    }
    prefabs = {
        "die": {"tag": "die", "shape": {"rect": [1, 1]}, "size": [1, 1], "hidden": True},
        "anvil": {"tag": "splash", "shape": {"sprite": "anvil", "scale": 5}, "size": [80, 40], "layer": 1},
        "ember": {"tag": "deco", "shape": {"rect": [1, 1]}, "size": [1, 1], "hidden": True,
                  "emit": {"rate": 5, "life": [1.5, 3.0], "speed": [10, 26], "angle": [255, 285], "color": "#ff8a2a", "size": 1.5, "gravity": -4}},
    }
    scene = [{"prefab": "die", "at": [0, 0], "id": "die"}, {"prefab": "anvil", "at": [240, 98]}]
    scene += [{"prefab": "ember", "at": [x, 270]} for x in (60, 180, 300, 420)]

    # --- checks ---
    scenarios = [
        {"name": "the first strike forges an idea", "seed": 1, "max_seconds": 6, "smoke": True,
         "script": [{"wait": 0.1}, {"press": "action"}, {"wait": 1.5}],
         "expect": [{"var": "n", "eq": 1}, {"var": "spin", "eq": 0}, {"var": "seed", "gt": 0}, {"var": "cur", "gt": 0}, {"sound": "strike", "min": 1}, {"sound": "ding", "min": 1}]},
        {"name": "twelve strikes never repeat an idea", "seed": 7, "max_seconds": 30,
         "script": sum(([{"press": "action"}, {"wait": 1.3}] for _ in range(12)), []),
         "expect": [{"var": "n", "eq": 12}, {"var": "dups", "eq": 0}, {"var": "hn", "eq": 8}]},
        {"name": "a locked part holds while the rest changes", "seed": 3, "max_seconds": 12,
         "script": [{"press": "action"}, {"wait": 1.3}, {"button": "lock_la"}, {"press": "action"}, {"wait": 1.3}, {"press": "action"}, {"wait": 1.3}],
         "expect": [{"var": "la", "eq": 1}, {"var": "n", "eq": 3}, {"var": "dups", "eq": 0}]},
        {"name": "story can be switched off and back", "seed": 2, "max_seconds": 6,
         "script": [{"press": "action"}, {"wait": 1.3}, {"button": "story"}, {"wait": 0.2}],
         "expect": [{"var": "story", "eq": 0}]},
        {"name": "an idea can be saved once and browsed", "seed": 5, "max_seconds": 12,
         "script": [{"press": "action"}, {"wait": 1.3}, {"button": "fav"}, {"button": "fav"}, {"press": "action"}, {"wait": 1.3}, {"button": "fav"},
                    {"button": "nextfav"}, {"wait": 0.2}],
         "expect": [{"var": "fn", "eq": 2}, {"var": "fi", "eq": 1}, {"sound": "chime", "min": 2}]},
        {"name": "back returns to the previous idea", "seed": 9, "max_seconds": 12,
         "script": [{"press": "action"}, {"wait": 1.3}, {"press": "action"}, {"wait": 1.3}, {"button": "back"}, {"wait": 0.2}],
         "expect": [{"var": "hn", "eq": 0}, {"var": "n", "eq": 2}, {"var": "cur", "eq": 4973254184}]},   # the first idea of this seed, restored
    ]
    return {
        "game2d": 1, "id": "idea-forge", "title": "Idea Forge",
        "description": "Strike the anvil to forge a video game idea built around one strange, never repeated gameplay mechanic, with an optional story; lock the parts you like and re-forge the rest.",
        "capabilities": {"presentation": "2d", "platforms": ["windows", "linux"], "networking": "offline", "input": ["keyboard", "mouse"], "persistence": ["settings", "progress"]},
        "view": {"width": 480, "height": 270, "background": "#0b1020", "scale": "fit"},
        "sprites": sprites, "sounds": sounds, "music": music, "vars": vars_, "persist": persist,
        "prefabs": prefabs, "scene": scene, "ui": ui, "effects": effects, "rules": rules, "checks": {"scenarios": scenarios},
    }


def main(argv) -> int:
    if len(argv) > 1 and argv[1] == "check":
        return check()
    if len(argv) > 2 and argv[1] == "decode":
        print(decode(int(argv[2]))); return 0
    if len(argv) > 2 and argv[1] == "walk":
        seed = int(argv[3]) if len(argv) > 3 else 12345
        for k in range(int(argv[2])):
            print(decode(walk(seed, k)), end="\n\n")
        return 0
    if check():
        return 1
    OUT.write_text(json.dumps(build(), indent=1, ensure_ascii=False) + "\n")
    print(f"wrote {OUT} ({OUT.stat().st_size:,} bytes)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
