"""Honeybound's world art: tiles per world, scenery, the parallax backdrops, the Bloom Gate at the end of a stage and the HUD icons. The cast (Pip, the
bees, the foes) is `cast.py`; the bosses are `bosses.py`. `all_sprites()` is everything, by name.

The look is a bee's-eye meadow: the ground is moss and roots, blocks are wax and honeycomb, hills are far away and soft, and giant stems and flowers stand
between them and the path. Every tile is a small function of (x, y) so a world's tile set is one palette.
"""
import math

import bosses
import cast
from canvas import Canvas


def spr(rows, palette, fps=None):
    s = {"rows": rows, "palette": palette}
    if fps:
        s["fps"] = fps
    return s


def anim(frames, palette, fps):
    return {"frames": frames, "palette": palette, "fps": fps}


def grid(w, h, f):
    return ["".join(f(x, y) for x in range(w)) for y in range(h)]


def speck(x, y, seed):
    """A stable pseudo-random number in 0..99 for a pixel."""
    v = (x * 73856093) ^ (y * 19349663) ^ (seed * 83492791)
    v = (v ^ (v >> 13)) * 1274126177
    return (v ^ (v >> 16)) % 100


# ---- the worlds' tile palettes ------------------------------------------------------------------------------------------------------------------------
# moss: the living cap; soil: the earth (three tones); wax: the breakable blocks (light, mid, the cell lines); stone: walls and hard ground.
THEMES = {
    "meadow": dict(moss="#7cc24a", moss2="#4e9a36", tip="#c4ec7a", soil="#a8683a", soil2="#7a4628", soil3="#c98c52", wax="#f2c46a", wax2="#d79a3e", waxl="#fff0b8", stone="#b8a48a", stone2="#8a7660"),
    "roots": dict(moss="#5aa08c", moss2="#2e6a5e", tip="#9ad8c0", soil="#4a3a4e", soil2="#2e2232", soil3="#6a5670", wax="#d8a85a", wax2="#a8763a", waxl="#ffe0a0", stone="#5a5a72", stone2="#3a3a50"),
    "dusk": dict(moss="#5aa08a", moss2="#2f6a5c", tip="#a8e0c0", soil="#7a5070", soil2="#4e2e4a", soil3="#a07090", wax="#e8b070", wax2="#b07a48", waxl="#ffe0b8", stone="#8a7a9a", stone2="#5a4a6a"),
    "caves": dict(moss="#6a7ad0", moss2="#3a4890", tip="#b0c0ff", soil="#3a3460", soil2="#221e40", soil3="#5a5490", wax="#d0a0e0", wax2="#9a6ab0", waxl="#f4dcff", stone="#4e4a7a", stone2="#2e2a54"),
    "autumn": dict(moss="#e09a3a", moss2="#b0602a", tip="#ffd080", soil="#8a5034", soil2="#5a3020", soil3="#b07048", wax="#f0b060", wax2="#c07a3a", waxl="#ffe0b0", stone="#a08070", stone2="#705040"),
    "frost": dict(moss="#f4fbff", moss2="#b8d8f0", tip="#ffffff", soil="#7a96b8", soil2="#4e6a8e", soil3="#a0bcd8", wax="#d8eaff", wax2="#9ab8e0", waxl="#ffffff", stone="#9ab0c8", stone2="#6a809a"),
    "dunes": dict(moss="#a8c860", moss2="#6a9a3a", tip="#e0f0a0", soil="#e0b060", soil2="#b0803a", soil3="#f0d08a", wax="#f8d070", wax2="#d09a40", waxl="#fff4c0", stone="#d8b880", stone2="#a88a50"),
    "night": dict(moss="#3f8a6a", moss2="#1e5040", tip="#7ad0a0", soil="#2e3050", soil2="#1a1a34", soil3="#4a4c78", wax="#c8b070", wax2="#8a7440", waxl="#fff0b0", stone="#4a5070", stone2="#2a3048"),
    "blossom": dict(moss="#f4a8c8", moss2="#c8689a", tip="#ffe0ee", soil="#8a5a6a", soil2="#5a3444", soil3="#b07a8a", wax="#ffd0a0", wax2="#e09a70", waxl="#fff0e0", stone="#c0a0b0", stone2="#8a6a7a"),
    "ash": dict(moss="#8a6a5a", moss2="#5a3e34", tip="#c09080", soil="#4a3434", soil2="#2a1a1a", soil3="#6a4a44", wax="#d08a50", wax2="#a0582a", waxl="#ffc890", stone="#6a5450", stone2="#423230"),
    "keep": dict(moss="#5e3a78", moss2="#3e2452", tip="#8a64a8", soil="#3a2a3e", soil2="#2c2032", soil3="#46344c", wax="#a07ab0", wax2="#6a4a7a", waxl="#d8c0e8", stone="#4e3c56", stone2="#2e2034"),
}


def ground_tiles(theme, t):
    pal = {"g": t["moss"], "G": t["moss2"], "t": t["tip"], "d": t["soil"], "D": t["soil2"], "e": t["soil3"], "w": t["wax"], "W": t["wax2"], "l": t["waxl"],
           "s": t["stone"], "S": t["stone2"], "o": "#2b1a14" if theme != "keep" else "#1e0f22"}

    def soil(x, y, seed):
        n = speck(x, y, seed)
        if n < 7:
            return "D"
        if n < 11:
            return "e"
        # a root wandering through the earth
        if (x + 2 * (y // 3) + seed) % 13 == 0 and n < 60:
            return "D"
        return "d"

    def top(x, y):
        # a thick mossy cap with rounded tufts that hang over the soil
        drop = [5, 6, 5, 4, 5, 6, 7, 6, 5, 4, 5, 5, 6, 5, 4, 5][x]
        if y == 0:
            return "t" if speck(x, 0, 3) < 45 else "g"
        if y < drop - 1:
            return "t" if speck(x, y, 5) < 10 else ("G" if speck(x, y, 6) < 18 else "g")
        if y < drop:
            return "G"
        return soil(x, y, 7)

    def wax(x, y):
        # a rounded wax block with honeycomb cells pressed into it
        if (x in (0, 15) and y in (0, 15)):
            return "."
        if x == 0 or y == 0 or x == 15 or y == 15:
            return "o"
        if x == 1 or y == 1:
            return "l"
        if x == 14 or y == 14:
            return "W"
        # honeycomb cells: the edges of the nearest-centre regions of a hex lattice
        centres = [(cx + (3.5 if r % 2 else 0), r * 6.0) for r in range(-1, 5) for cx in range(-7, 23, 7)]
        d = sorted(math.hypot(x - cx, (y - cy) * 1.15) for cx, cy in centres)
        if d[1] - d[0] < 1.1:
            return "W"
        if d[0] < 1.6 and (x + y) % 3 == 0:
            return "l"
        return "w"

    def stone(x, y):
        # fitted pebbles: rounded stones with mortar of soil
        # staggered rounded stones, 8x5, with dark joints
        row = y // 5
        xx = (x + (4 if row % 2 else 0)) % 8
        yy = y % 5
        corner = (xx in (0, 7)) and (yy in (0, 4))
        if yy == 4 or xx == 7 or corner:
            return "D"
        if yy == 0 or xx == 0:
            return "e"
        if yy == 3 or xx == 6:
            return "S"
        return "s"

    out = {
        f"{theme}_top": spr(grid(16, 16, top), pal),
        f"{theme}_soil": spr(grid(16, 16, lambda x, y: soil(x, y, 3)), pal),
        f"{theme}_brick": spr(grid(16, 16, wax), pal),
        f"{theme}_stone": spr(grid(16, 16, stone), pal),
    }
    if theme == "keep":
        # the Baron's keeps: walls of woven bramble
        def bramble(x, y):
            a = (x + y) % 8
            b = (x - y) % 8
            if a in (0, 1) or b in (0, 1):
                return "G" if (a + b) % 3 else "o"
            if speck(x, y, 9) < 6:
                return "t"
            return "d" if (x // 4 + y // 4) % 2 else "e"
        out["keep_stone"] = spr(grid(16, 16, bramble), pal)
        out["keep_top"] = spr(grid(16, 16, lambda x, y: "g" if y < 3 and speck(x, y, 2) < 70 else ("t" if y < 3 else bramble(x, y))), pal)
    return out


def block_tiles():
    out = {}
    comb_pal = {"y": "#ffc531", "Y": "#d98e1a", "o": "#7a3a08", "w": "#fff3b0", "k": "#a85d00", "h": "#ffe680"}

    def comb(shine):
        def f(x, y):
            if (x in (0, 15) and y in (0, 15)):
                return "."
            if x in (0, 15) or y in (0, 15):
                return "o"
            # a honey drop in the middle that catches the light
            dx, dy = x - 7.5, y - 8.5
            if dx * dx / 9 + dy * dy / 12 <= 1.0 or (y in (4, 5) and x in (7, 8)):
                if (x, y) in ((6, 7), (6, 8)) or (shine and (x, y) == (7, 6)):
                    return "w"
                return "k" if dx * dx / 9 + dy * dy / 12 > 0.6 else "h"
            row = (y - 1) // 4
            xx = (x - 1 + (2 if row % 2 else 0)) % 5
            yy = (y - 1) % 4
            if (yy == 0 and xx in (1, 2, 3)) or (yy in (1, 2, 3) and xx == 0):
                return "Y"
            if shine and (x + y) % 9 == shine:
                return "w"
            return "y"
        return grid(16, 16, f)

    out["comb"] = anim([comb(0), comb(3), comb(6), comb(3)], comb_pal, 5)
    used_pal = {"u": "#b08a5a", "U": "#7a5a34", "o": "#4a2a10", "l": "#d0b080"}
    out["used"] = spr(grid(16, 16, lambda x, y: "." if (x in (0, 15) and y in (0, 15)) else ("o" if x in (0, 15) or y in (0, 15) else ("U" if ((y - 1) % 4 == 0 and (x + 2 * ((y - 1) // 4 % 2)) % 5 in (1, 2, 3)) else ("l" if x + y < 6 else "u")))), used_pal)
    # a round river pebble (the stairs and the posts)
    hard_pal = {"h": "#c8b8a0", "H": "#8a7a64", "l": "#f0e6d4", "o": "#4a3a2a"}

    def hard(x, y):
        dx, dy = x - 7.5, y - 7.5
        r = max(abs(dx), abs(dy)) * 0.6 + math.hypot(dx, dy) * 0.4
        if r > 7.6:
            return "o"
        if dx + dy < -6:
            return "l"
        if dx + dy > 5:
            return "H"
        return "h"
    out["hard"] = spr(grid(16, 16, hard), hard_pal)
    # hollow stumps (Pip climbs onto them; one of them leads to a honey room): a top with rings, a bark body with moss
    log_pal = {"a": "#8a5a34", "A": "#5a3a1e", "r": "#e0b878", "R": "#b0864a", "k": "#2b1a14", "m": "#1a100a", "g": "#7cc24a", "G": "#4e9a36"}

    def lip(x, y, left):
        xx = x if left else 15 - x
        if y == 0:
            return "k" if xx > 0 else "."
        if y < 6:
            if xx == 0:
                return "k"
            if y == 1 or xx == 1:
                return "r"
            if y >= 3 and xx >= 3:
                return "m" if y < 5 else "R"
            return "R"
        if xx == 0:
            return "k"
        if y in (6, 7) and speck(x, y, 4) < 50:
            return "g" if y == 6 else "G"
        return "A" if (xx + y) % 5 == 0 else "a"

    def body(x, y, left):
        xx = x if left else 15 - x
        if xx == 0:
            return "k"
        if (xx * 3 + y // 3) % 7 == 0:
            return "A"
        return "a" if xx > 2 else "R"
    out["log_tl"] = spr(grid(16, 16, lambda x, y: lip(x, y, True)), log_pal)
    out["log_tr"] = spr(grid(16, 16, lambda x, y: lip(x, y, False)), log_pal)
    out["log_bl"] = spr(grid(16, 16, lambda x, y: body(x, y, True)), log_pal)
    out["log_br"] = spr(grid(16, 16, lambda x, y: body(x, y, False)), log_pal)
    out["log_side"] = spr(grid(16, 16, lambda x, y: lip(y, x, True) if lip(y, x, True) != "." else "k"), log_pal)
    # leaf platforms (the high stages) on green stems
    leaf_pal = {"g": "#7cd65c", "G": "#3f9a3a", "l": "#c8f59a", "v": "#2f7a2a", "t": "#5aa040", "T": "#3a7a2a", "o": "#1e4a18"}

    def leaf(edge):
        def f(x, y):
            if y > 9:
                return "."
            if (edge == "l" and x + y < 3) or (edge == "r" and (15 - x) + y < 3):
                return "."
            if y == 0 or y == 9:
                return "o"
            if (edge == "l" and x == 0) or (edge == "r" and x == 15):
                return "o"
            if y == 4:
                return "v"
            if (y < 4 and (x + y) % 5 == 0) or (y > 4 and (x - y) % 5 == 0):
                return "G"
            return "l" if y == 1 else ("g" if y < 7 else "G")
        return grid(16, 16, f)
    out["leaf_l"] = spr(leaf("l"), leaf_pal)
    out["leaf_m"] = spr(leaf("m"), leaf_pal)
    out["leaf_r"] = spr(leaf("r"), leaf_pal)
    out["trunk"] = spr(grid(16, 16, lambda x, y: "." if x < 6 or x > 9 else ("o" if x in (6, 9) else ("T" if x == 8 else "t"))), leaf_pal)
    # a twig raft (the moving platforms)
    raft = Canvas(48, 8)
    raft.rect(1, 1, 46, 5, "p")
    for x in range(4, 46, 6):
        raft.line(x, 1, x, 5, "P")
    raft.ell(3, 3, 2.5, 2.5, "g").ell(44, 3, 2.5, 2.5, "g")
    raft.outline("k")
    out["lift"] = spr(raft.rows()[:6], {"p": "#c8925a", "P": "#8a5a2a", "k": "#3a1e0a", "g": "#7cc24a"})
    # hot sap (the keeps' pits): amber, slow and glowing
    sap_pal = {"o": "#ff9a2a", "O": "#c8501a", "y": "#ffd060", "w": "#fff2b0"}

    def sap(t):
        def f(x, y):
            wave = 2 + int(1.5 + 1.5 * math.sin((x + t * 4) * math.pi / 8))
            if y < wave:
                return "."
            if y == wave:
                return "y"
            if speck(x + t * 2, y, 11) < 8:
                return "w" if y < 7 else "y"
            return "O" if y > 9 else "o"
        return grid(16, 16, f)
    out["lava"] = anim([sap(t) for t in range(4)], sap_pal, 4)
    out["lava_deep"] = anim([grid(16, 16, lambda x, y, t=t: "y" if speck(x + t * 5, y + t * 3, 13) < 5 else ("O" if (x + 2 * y + t) % 9 == 0 else "o")) for t in range(4)], sap_pal, 3)
    # the bramble gate that shuts behind Pip in a boss's den
    gate = Canvas(16, 16)
    gate.rect(2, 0, 13, 15, "d")
    for y in range(0, 16, 4):
        gate.line(2, y, 13, y + 3, "G")
        gate.line(13, y, 2, y + 3, "G")
    for (x, y) in ((1, 2), (14, 6), (1, 10), (14, 13)):
        gate.dot(x, y, "t")
    gate.outline("o")
    out["gate"] = spr(gate.rows(), {"d": "#3a2a3e", "G": "#7b3fa0", "t": "#ff4fa3", "o": "#1e0f22"})
    return out


# ---- the Bloom Gate: the end of every open stage ---------------------------------------------------------------------------------------------------
def bloom():
    """A great sunflower bud on a tall stem. Pip touches it and it opens: the way on is clear. 32x64, its base at the bottom edge."""
    pal = {"v": "#3f9a3a", "V": "#2a6a28", "l": "#7cd65c", "p": "#ffd23f", "P": "#e0952a", "c": "#7a4a22", "C": "#4a2a14", "o": "#2b1a14", "w": "#fff6c0", "b": "#a0c860"}

    def make(openness):
        c = Canvas(32, 64)
        c.rect(15, 26, 16, 63, "v")
        c.line(17, 26, 17, 63, "V")
        c.ell(10, 46, 6, 2.4, "l").ell(22, 38, 6, 2.4, "l")
        c.shade("l", "v", lambda x, y: y > 46 or (x > 20 and y > 38))
        cy = 18
        if openness == 0:
            c.ell(16, cy, 7, 9, "b")
            c.shade("b", "v", lambda x, y: (x - 16) * (x - 16) > 10)
            c.ell(16, cy - 6, 2.2, 3, "p")
        else:
            n = 12
            r = 7 + 4 * openness
            for k in range(n):
                a = 2 * math.pi * k / n + 0.2
                c.ell(16 + r * math.cos(a), cy + r * math.sin(a), 3.4, 3.4, "p")
            c.shade("p", "P", lambda x, y: y > cy + 3)
            c.ell(16, cy, 6.4, 6.4, "c")
            c.shade("c", "C", lambda x, y: (x + y) % 3 == 0)
            if openness >= 1:
                c.dot(14, 16, "w").dot(17, 15, "w").dot(19, 19, "w")
        c.outline("o")
        return c.rows()
    return {"bloom": spr(make(0), pal), "bloom_open": anim([make(0.6), make(1)], pal, 6)}


# ---- scenery in the play field (behind every thing, touched by nothing) -------------------------------------------------------------------------
def decor():
    out = {}
    o = "#2b1a14"
    # tufts of tall grass
    for name, (g, G) in {"tuft": ("#7cc24a", "#4e9a36"), "tuft_autumn": ("#e0a040", "#a0602a"), "tuft_frost": ("#e8f6ff", "#9ac0e0"), "tuft_night": ("#3f8a6a", "#1e5040"),
                         "tuft_blossom": ("#f4a8c8", "#c8689a"), "tuft_ash": ("#8a6a5a", "#5a3e34"), "tuft_dunes": ("#a8c860", "#6a9a3a"), "tuft_dusk": ("#5aa08a", "#2f6a5c")}.items():
        c = Canvas(16, 12)
        for (x, h, lean) in ((2, 7, -1), (4, 10, 0), (6, 8, 1), (9, 11, 0), (11, 7, 1), (13, 9, 0)):
            c.line(x, 11, x + lean * 2, 11 - h, "g")
            c.dot(x + 1, 11, "G").dot(x, 10, "G")
        out[name] = spr(c.rows(), {"g": g, "G": G})
    # mushrooms
    c = Canvas(16, 16)
    c.rect(6, 9, 9, 15, "s")
    c.ell(8, 8, 7, 4.2, "r")
    c.shade("r", "R", lambda x, y: y > 9)
    c.rect(1, 9, 15, 9, ".")
    for (x, y) in ((5, 6), (10, 5), (8, 8)):
        c.dot(x, y, "w")
    c.outline("o")
    out["mushroom"] = spr(c.rows(), {"s": "#f0e6d4", "r": "#e8572a", "R": "#a8321a", "w": "#ffffff", "o": o})
    # a little daisy patch
    c = Canvas(16, 12)
    for (x, h, col) in ((3, 6, "w"), (8, 9, "p"), (12, 5, "w")):
        c.line(x, 11, x, 11 - h, "g")
        top = 11 - h
        for (dx, dy) in ((-1, 0), (1, 0), (0, -1), (0, 1)):
            c.dot(x + dx, top + dy, col)
        c.dot(x, top, "y")
    out["flowers"] = spr(c.rows(), {"g": "#4e9a36", "w": "#ffffff", "p": "#ff8ab8", "y": "#ffd23f"})
    # pebbles
    c = Canvas(16, 8)
    c.ell(4, 6, 3.4, 2.4, "s").ell(10, 6.5, 2.4, 1.8, "S").ell(13.5, 6, 1.6, 1.4, "s")
    c.shade("s", "l", lambda x, y: y < 5)
    c.outline("o")
    out["pebbles"] = spr(c.rows(), {"s": "#b8a48a", "S": "#8a7660", "l": "#e0d4c0", "o": o})
    # a glow-worm lantern on a hook (the keeps' torches): a soft warm pool of light
    fr = []
    for k in range(2):
        c = Canvas(16, 16)
        c.line(8, 0, 8, 3, "k")
        c.ell(8, 8, 3.6, 4.4, "L" if k == 0 else "l")
        c.ell(8, 8, 1.8, 2.2, "w" if k == 0 else "L", only="Ll")
        c.rect(5, 3, 11, 4, "k")
        c.rect(5, 12, 11, 13, "k")
        fr.append(c.rows())
    out["torch"] = anim(fr, {"k": "#2a1a2e", "L": "#ffd06a", "l": "#ffb84a", "w": "#fff6d0"}, 3)
    # cave crystals
    c = Canvas(16, 16)
    for (x, h) in ((3, 6), (6, 10), (10, 7), (13, 4)):
        c.line(x, 15, x, 15 - h, "c").line(x + 1, 15, x + 1, 16 - h, "C")
        c.dot(x, 15 - h, "w")
    out["crystal"] = spr(c.rows(), {"c": "#9af0ff", "C": "#3aa0c0", "w": "#ffffff"})
    # twinkling stars (the night skies)
    pts = [(6, 0), (14, 2), (3, 5), (10, 7), (1, 10), (8, 12), (13, 14)]
    out["stars"] = anim([grid(16, 16, lambda x, y, k=k: ("w" if (pts.index((x, y)) + k) % 3 else "y") if (x, y) in pts else ".") for k in range(3)], {"w": "#ffffff", "y": "#fff3a0"}, 2)
    return out


# ---- parallax backdrops: far hills, giant plants, soft clouds -----------------------------------------------------------------------------------------
# Each world gets three bands: far hills (light, close to the sky), the giant meadow (stems, flowers, trees) and drifting clouds. They are silhouettes in
# two tones, so the play field (outlined, full colour) always stands in front of them.
BACKS = {
    # world look: sky top, sky bottom, far (hill, hill shade), near (plant, plant shade), cloud, cloud shade, plant kind
    "meadow": ("#69b8f0", "#d8f0ff", "#a8d8a0", "#90c890", "#6aaa62", "#58965a", "#ffffff", "#d8ecff", "flowers"),
    "roots": ("#14101c", "#2a2236", "#2e2440", "#261e36", "#3a2e4a", "#30263e", None, None, "roots"),
    "dusk": ("#3a2a6a", "#e88a8a", "#7a5a8a", "#6a4a7a", "#3e2e54", "#342648", "#f0b0c8", "#c888a8", "trees"),
    "caves": ("#0c1028", "#1e2448", "#242a54", "#1e2448", "#2e3466", "#262c5a", None, None, "roots"),
    "autumn": ("#f4b070", "#ffe8c0", "#e0a070", "#d09060", "#a85a34", "#94502e", "#fff4e0", "#f0d0b0", "trees"),
    "frost": ("#a8d0f0", "#f0f8ff", "#d8e8f8", "#c8dcf0", "#8aaed0", "#7a9cc0", "#ffffff", "#e0ecf8", "pines"),
    "dunes": ("#f0a060", "#ffe0a0", "#f0c080", "#e0b070", "#c08a50", "#ac7a46", "#fff0d0", "#f8d8a8", "cacti"),
    "night": ("#0a0e30", "#2a3a6a", "#24305a", "#1e2850", "#162040", "#121a36", "#3a4a80", "#2e3c6a", "flowers"),
    "blossom": ("#ffc0d8", "#fff4f8", "#f8c8d8", "#f0b8cc", "#d888a8", "#c87898", "#ffffff", "#ffe0ec", "trees"),
    "ash": ("#2a1414", "#8a3a2a", "#4a2a28", "#3e2220", "#2a1a1a", "#221414", "#6a4a44", "#543a36", "dead"),
    "keep": ("#120814", "#2a1430", "#2a1a30", "#22142a", "#3a2044", "#30183a", None, None, "brambles"),
}


def backdrop_sprites():
    out = {}
    for look, (_top, _bot, far, far2, near, near2, cloud, cloud2, kind) in BACKS.items():
        pal = {"f": far, "F": far2, "n": near, "N": near2, "c": cloud or "#ffffff", "C": cloud2 or "#ffffff"}
        # two far hills: soft rounded silhouettes
        for k, (w, h) in enumerate(((64, 30), (64, 22))):
            c = Canvas(w, h)
            if kind in ("roots", "brambles"):
                for j in range(5):
                    x0 = 4 + j * 13 + (k * 5)
                    c.line(x0, h - 1, x0 + 6 * (1 if j % 2 else -1), 0, "f")
                    c.line(x0 + 1, h - 1, x0 + 7 * (1 if j % 2 else -1), 2, "f")
                    c.line(x0 + 2, h - 1, x0 + 5 * (1 if j % 2 else -1), 4, "F")
            else:
                c.ell(w * 0.35, h + 4, w * 0.4, h + 3, "f")
                c.ell(w * 0.72, h + 6, w * 0.32, h * 0.8 + 4, "f")
                c.shade("f", "F", lambda x, y: (x * 3 + y * 5) % 23 == 0 or y > h - 3)
            out[f"far_{look}_{k}"] = spr(c.rows(), pal)
        # the giant plants of the near band
        for k in range(3):
            c = Canvas(32, 64)
            if kind == "flowers":
                stem_x = 12 + k * 4
                c.rect(stem_x, 18, stem_x + 1, 63, "n")
                c.ell(stem_x - 5, 44 - k * 6, 5, 2, "n")
                if k == 1:  # a dandelion clock
                    c.ell(stem_x + 0.5, 12, 10, 10, "N")
                    c.ell(stem_x + 0.5, 12, 8, 8, "n", only="N")
                else:  # a daisy
                    for j in range(8):
                        a = 2 * math.pi * j / 8
                        c.ell(stem_x + 0.5 + 7 * math.cos(a), 14 + 7 * math.sin(a), 3, 3, "N")
                    c.ell(stem_x + 0.5, 14, 3.5, 3.5, "n")
            elif kind in ("trees", "dead"):
                c.rect(14 + k, 30, 17 + k, 63, "N")
                if kind == "dead":
                    c.line(15 + k, 34, 6, 20, "N").line(17 + k, 30, 27, 14, "N").line(16, 24, 12, 8, "N")
                else:
                    c.ell(16, 22 - k * 2, 14 - k, 13 - k, "n")
                    c.ell(9, 28, 8, 7, "n").ell(24, 27, 8, 7, "n")
                    c.shade("n", "N", lambda x, y: (x + y * 2) % 11 == 0 or y > 31)
            elif kind == "pines":
                for j in range(5):
                    half = 4 + j * 3 - k
                    c.ell(16, 12 + j * 9, half, 6, "n")
                c.rect(15, 50, 17, 63, "N")
                c.shade("n", "N", lambda x, y: x > 17)
            elif kind == "cacti":
                c.rect(13, 14 + k * 4, 18, 63, "n")
                c.rect(6, 30, 8, 44, "n").rect(6, 42, 13, 44, "n")
                c.rect(23, 24 + k * 3, 25, 38, "n").rect(18, 36, 25, 38, "n")
                c.shade("n", "N", lambda x, y: x in (17, 18, 8, 25))
            elif kind == "roots":
                for j in range(3):
                    x = 6 + j * 10 + k
                    for y in range(0, 64):
                        c.dot(int(x + 3 * math.sin(y / (6 + j))), y, "n")
                        c.dot(int(x + 1 + 3 * math.sin(y / (6 + j))), y, "N")
            elif kind == "brambles":
                for j in range(4):
                    x0 = 4 + j * 8
                    for y in range(10 + k * 4, 64):
                        x = int(x0 + 4 * math.sin(y / 5 + j))
                        c.dot(x, y, "n")
                        if y % 7 == 0:
                            c.dot(x + 1, y - 1, "N").dot(x + 2, y - 2, "N")
            out[f"near_{look}_{k}"] = spr(c.rows(), pal)
        if cloud:
            for k in range(2):
                c = Canvas(48, 16)
                c.ell(14, 10, 10, 5, "c").ell(26, 8, 10, 7, "c").ell(37, 11, 9, 4.4, "c")
                c.rect(4, 12, 46, 15, ".")
                c.shade("c", "C", lambda x, y: y > 10)
                if k:
                    c = c.flipped()
                out[f"cloud_{look}_{k}"] = spr(c.rows(), pal)
    return out


def comb_wall():
    """The inside of the hive: a wall of honeycomb cells (32x28, it tiles), for the prologue's backdrop."""
    pal = {"a": "#b8782a", "A": "#a86c22", "w": "#6a3a0a", "h": "#d09a40"}
    rows = []
    for y in range(28):
        row = ""
        for x in range(32):
            centres = [(cx + (8 if r % 2 else 0), r * 14.0) for r in range(-1, 4) for cx in range(-16, 49, 16)]
            d = sorted(math.hypot(x - cx, (y - cy) * 1.15) for cx, cy in centres)
            if d[1] - d[0] < 1.3:
                row += "w"
            elif d[0] < 3:
                row += "h"
            else:
                row += "a" if (x + y) % 7 else "A"
        rows.append(row)
    return {"comb_wall": spr(rows, pal)}


def pit_shade():
    """The dark inside of a pit (16x48: it fades in from the ground line down), so a gap in the ground reads as a hole, not as far-off dirt."""
    shades = "abcdefgh"
    pal = {k: "#140c10" + a for k, a in zip(shades, ("40", "70", "a0", "c8", "e0", "f0", "f8", "ff"))}
    rows = [shades[min(7, y // 3)] * 16 for y in range(48)]
    return {"pit_shade": spr(rows, pal)}


def sky(look):
    top, bottom, *_ = BACKS[look]
    return [top, bottom]


def backdrop(look, world_h, ground_y):
    """The three parallax bands of a world look: far hills, giant plants (both standing on the ground line), clouds high up."""
    _t, _b, far, _f2, near, _n2, cloud, *_r = BACKS[look]
    bands = []
    if cloud:
        bands.append({"sprites": [f"cloud_{look}_0", f"cloud_{look}_1"], "y": 70, "parallax": 0.08, "spacing": [60, 160], "seed": 3})
    bands.append({"sprites": [f"far_{look}_0", f"far_{look}_1"], "y": ground_y - 18, "parallax": 0.18, "spacing": [-20, 30], "fill": far, "seed": 5})
    bands.append({"sprites": [f"near_{look}_0", f"near_{look}_1", f"near_{look}_2"], "y": ground_y + 4, "parallax": 0.42, "spacing": [20, 90], "fill": near, "seed": 7})
    return bands


# ---- the HUD -------------------------------------------------------------------------------------------------------------------------------------
def hud():
    out = {}
    # a shard slot (empty) and a found shard, 10x11
    for name, (a, A) in {"hud_shard": ("#ffd23f", "#e09b00"), "hud_shard_empty": ("#5a4a3a", "#3a2e24")}.items():
        c = Canvas(10, 11)
        for y in range(1, 10):
            d = min(y - 1, 9 - y)
            half = min(4, 1 + d * 2)
            c.rect(5 - half, y, 4 + half, y, "a")
        c.shade("a", "A", lambda x, y: x + y > 10)
        if name == "hud_shard":
            c.dot(3, 3, "w").dot(4, 3, "w")
        c.outline("o")
        out[name] = spr(c.rows(), {"a": a, "A": A, "w": "#fffbe0", "o": "#2b1a14"})
    # pad buttons (Xbox colours) and a keyboard key cap, 11x11 with a letter
    glyph = {
        "A": [".#.", "#.#", "###", "#.#", "#.#"],
        "B": ["##.", "#.#", "##.", "#.#", "##."],
        "X": ["#.#", "#.#", ".#.", "#.#", "#.#"],
        "Y": ["#.#", "#.#", ".#.", ".#.", ".#."],
    }
    for b, col in {"A": "#4caf50", "B": "#e53935", "X": "#1e88e5", "Y": "#fdd835"}.items():
        c = Canvas(11, 11)
        c.ell(5.5, 5.5, 5.5, 5.5, "b")
        c.shade("b", "B", lambda x, y: y > 7)
        c.stamp(glyph[b], 4, 3)
        c.px = {p: ("w" if v == "#" else v) for p, v in c.px.items()}
        out[f"btn_{b.lower()}"] = spr(c.rows(), {"b": col, "B": _shade(col), "w": "#ffffff"})
    # Start (the menu button): a pill with three lines
    c = Canvas(15, 9)
    c.ell(7.5, 4.5, 7.5, 4.5, "b")
    for y in (2, 4, 6):
        c.line(5, y, 9, y, "w")
    out["btn_start"] = spr(c.rows(), {"b": "#5a5a66", "w": "#ffffff"})
    # the d-pad / stick
    c = Canvas(11, 11)
    c.rect(4, 0, 6, 10, "b").rect(0, 4, 10, 6, "b")
    c.dot(5, 5, "B")
    out["btn_dpad"] = spr(c.rows(), {"b": "#5a5a66", "B": "#2a2a33"})
    # the music note beside the song's name
    c = Canvas(9, 10)
    c.line(6, 0, 6, 7, "w").line(6, 0, 8, 1, "w").ell(4, 8, 2.4, 1.6, "w")
    out["note"] = spr(c.rows(), {"w": "#ffd23f"})
    # a panel corner flourish for the menus: a little honeycomb (6x6)
    c = Canvas(7, 7)
    for y in range(7):
        d = min(y, 6 - y)
        half = min(3, 1 + d)
        c.rect(3 - half, y, 3 + half, y, "a")
    c.outline("o")
    out["hex"] = spr(c.rows()[:7], {"a": "#ffc531", "o": "#7a3a08"})
    # the Heartcomb, whole (the ending)
    c = Canvas(40, 40)
    for (cx, cy) in ((20, 20), (11, 15), (29, 15), (11, 25), (29, 25), (20, 10), (20, 30)):
        for y in range(-5, 6):
            half = min(5, 1 + (5 - abs(y)) * 2)
            c.rect(cx - half, cy + y, cx + half - 1, cy + y, "a")
    c.shade("a", "A", lambda x, y: (x + y) % 9 == 0 or x + y > 50)
    c.ell(20, 20, 3, 3, "w", only="aA")
    c.outline("o")
    out["heartcomb"] = spr(c.rows(), {"a": "#ffd23f", "A": "#e09b00", "w": "#fffbe0", "o": "#7a3a08"})
    return out


def _shade(c, k=0.7):
    c = c.lstrip("#")
    r, g, b = (int(c[i:i + 2], 16) for i in (0, 2, 4))
    return "#%02x%02x%02x" % (int(r * k), int(g * k), int(b * k))


def all_sprites():
    out = {}
    tiles = {}
    for theme, t in THEMES.items():
        tiles.update(ground_tiles(theme, t))
    for part in (cast.all_cast(), bosses.all_bosses(), tiles, block_tiles(), bloom(), decor(), backdrop_sprites(), hud(), comb_wall(), pit_shade()):
        for k, v in part.items():
            assert k not in out, k
            out[k] = v
    return out
