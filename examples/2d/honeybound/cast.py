"""Honeybound's cast: Pip, the bees and bugs Pip meets, the Baron's thorn-touched creatures and the eight bosses, all drawn with `canvas.Canvas`.

Every character faces right (the engine mirrors it). The look is one family: round bodies, a warm dark outline (never black), a light from the upper
left, big eyes. Everything the Baron has corrupted carries his mark: violet thorns and a magenta glint in the eye, so a foe reads as "his" at a glance.
"""
from canvas import Canvas

OUT = "#2b1a14"  # the warm outline every friendly thing has
THORN_OUT = "#1e0f22"  # the Baron's creatures: a cold, violet-black outline
THORN = {"t": "#7b3fa0", "T": "#4a1f66", "g": "#ff4fa3"}  # violet thorns, magenta eye glint


def spr(c, pal):
    return {"rows": c.rows(), "palette": pal}


def anim(cs, pal, fps):
    return {"frames": [c.rows() for c in cs], "palette": pal, "fps": fps}


def recolor(s, changes):
    out = dict(s)
    out["palette"] = {**s["palette"], **changes}
    return out


# ---- Pip ------------------------------------------------------------------------------------------------------------------------------------------
# The smallest bee of the Great Hive: round and fuzzy, honey-gold with cocoa stripes, a big curious eye, two antennae and wings too small to fly with.

PIP = {"o": OUT, "y": "#ffc83d", "Y": "#e0952a", "b": "#5a3420", "h": "#ffe08a", "e": "#ffffff", "k": "#1c120e", "r": "#ff8a7a", "w": "#e8fbff", "W": "#a8dcf0", "l": "#3a2418"}
POLLEN = {"y": "#ffe066", "Y": "#f0a030", "b": "#d8432a", "l": "#8a2a14", "h": "#fff6c8", "w": "#fff2c0", "W": "#ffcf6a"}  # pollen Pip: bright gold, ember stripes, golden wings
BIG = 1


def _pip(w_up=True, legs=0, blink=False, big=False, squash=0):
    """Pip, small (16x16) or grown (16x24: a bigger body, longer wings). legs: 0 stand, 1 and 2 the walk, 3 tucked (in the air)."""
    H = 24 if big else 16
    c = Canvas(16, H)
    by = H - 6 + squash * 0.5  # abdomen centre
    ab_rx, ab_ry = (5.6, 5.4) if big else (4.6, 3.9)
    hx, hy, hr = (11.3, by - (7.5 if big else 4.6), 3.7 if big else 3.4)
    ab_x = 6.0
    # wings behind the body
    if w_up:
        c.ell(6.2, by - ab_ry - (1.6 if big else 1.0), 2.4 if not big else 3.2, 2.1 if not big else 3.0, "w")
        c.ell(4.3, by - ab_ry - (0.6 if big else 0.2), 1.8 if not big else 2.4, 1.6 if not big else 2.2, "w")
    else:
        c.ell(4.8, by - ab_ry + 0.4, 3.0 if not big else 3.8, 1.4 if not big else 1.9, "w")
    c.shade("w", "W", lambda x, y: y > by - ab_ry - (1.0 if big else 0.6))
    # abdomen with stripes
    c.ell(ab_x, by, ab_rx, ab_ry - squash * 0.3, "y")
    for sx in ((3, 6) if not big else (2, 5, 8)):
        c.shade("y", "b", lambda x, y, sx=sx: x == sx or x == sx + 1)
    c.shade("y", "Y", lambda x, y: y > by + ab_ry * 0.45)
    c.shade("b", "l", lambda x, y: y > by + ab_ry * 0.45)
    # head
    c.ell(hx, hy, hr, hr, "y")
    c.shade("y", "h", lambda x, y: (x - hx) + (y - hy) < -hr * 0.9)
    c.shade("y", "Y", lambda x, y: y > hy + hr * 0.55 and abs(x - hx) < hr)
    # eye, cheek
    ex, ey = int(hx + 0.6), int(hy - 1)
    if blink:
        c.dot(ex, ey + 1, "k").dot(ex + 1, ey + 1, "k")
    else:
        c.rect(ex, ey, ex + 1, ey + 1, "e")
        c.dot(ex + 1, ey + 1, "k").dot(ex + 1, ey, "k")
        c.dot(ex, ey, "e")
    c.dot(ex + 1, ey + 2 + (1 if big else 0), "r")
    # antennae
    ax = int(hx) - 1
    top = int(hy - hr)
    c.line(ax, top, ax + 1, top - 2, "l").dot(ax + 2, top - 3, "l")
    c.line(ax + 2, top, ax + 3, top - 2, "l").dot(ax + 4, top - 2, "l")
    # little legs
    base = H - 2
    feet = {0: [(5, base), (8, base)], 1: [(4, base), (9, base)], 2: [(6, base), (7, base)], 3: [(6, base - 1), (8, base - 1)]}[legs]
    for fx, fy in feet:
        c.dot(fx, fy, "l")
        c.dot(fx, fy - 1, "l")
    c.outline("o", skip="")
    return c


def pip_sprites():
    out = {}
    for big, name in ((False, "pip"), (True, "pipbig")):
        walk = [_pip(w_up=True, legs=1, big=big), _pip(w_up=False, legs=2, big=big), _pip(w_up=True, legs=2, big=big), _pip(w_up=False, legs=1, big=big)]
        idle = [_pip(w_up=True, legs=0, big=big), _pip(w_up=True, legs=0, big=big), _pip(w_up=True, legs=0, big=big, blink=True), _pip(w_up=False, legs=0, big=big, squash=1)]
        air = [_pip(w_up=True, legs=3, big=big), _pip(w_up=False, legs=3, big=big)]
        out[name] = anim(walk, PIP, 10)
        out[name + "_idle"] = anim(idle, PIP, 3)
        out[name + "_air"] = anim(air, PIP, 18)
    # pollen Pip (throws pollen puffs): the grown sprites dusted white with ember stripes
    for k in ("pipbig", "pipbig_idle", "pipbig_air"):
        out[k.replace("pipbig", "pollen")] = recolor(out[k], POLLEN)
    # sunpetal power: the walk cycle with its colours cycling
    cycles = [{"y": "#fff27a", "Y": "#ffb02e", "b": "#ff4fa3"}, {"y": "#9ff7ff", "Y": "#3ab0d8", "b": "#7b3fa0"}, {"y": "#c8ff8a", "Y": "#5cc44a", "b": "#ff8a3d"}]
    for base in ("pip", "pipbig"):
        frames, pal = [], dict(PIP)
        for k, cols in enumerate(cycles):
            letters = "ijm"[k], "IJM"[k], "npq"[k]
            rows = [r.replace("y", letters[0]).replace("Y", letters[1]).replace("b", letters[2]) for r in out[base]["frames"][k % 4]]
            frames.append(rows)
            pal[letters[0]], pal[letters[1]], pal[letters[2]] = cols["y"], cols["Y"], cols["b"]
        out[base + "_star"] = {"frames": frames, "palette": pal, "fps": 15}
        H = 24 if base == "pipbig" else 16
        f = out[base]["frames"]
        out[base + "_blink"] = {"frames": [f[0], ["." * 16] * H, f[1], ["." * 16] * H], "palette": PIP, "fps": 16}
    # dazed (a life lost): eyes shut tight, wings drooping, upside down
    d = _pip(w_up=False, legs=3, blink=True)
    out["pip_dazed"] = spr(d.vflipped(), PIP)
    return out


def pip_icons():
    """Pip's face for the HUD (12x12) and the game's icon (32x32: Pip waving, a honey drop): the window, the taskbar and the desktop shortcut."""
    h = Canvas(12, 12)
    h.ell(6, 6.5, 4.6, 4.6, "y")
    h.shade("y", "h", lambda x, y: x + y < 8)
    h.shade("y", "Y", lambda x, y: y > 8)
    h.rect(6, 5, 7, 6, "e").dot(7, 5, "k").dot(7, 6, "k")
    h.dot(9, 7, "r").dot(8, 8, "r")
    h.line(4, 2, 3, 0, "l").line(7, 2, 8, 0, "l")
    h.outline("o")
    icon = Canvas(32, 32)
    icon.ell(16, 16, 15.5, 15.5, "s")  # a honey-gold disc behind
    icon.shade("s", "S", lambda x, y: (x - 16) ** 2 + (y - 16) ** 2 > 13.2 ** 2)
    for (cx, cy) in ((9, 9), (23, 9), (16, 5), (5, 17), (27, 17), (9, 25), (23, 25)):
        icon.ell(cx, cy, 2.6, 2.6, "H", only="s")
    icon.ell(10.5, 9.5, 5.5, 4.5, "w")  # wings
    icon.ell(21.5, 9.5, 5.5, 4.5, "w")
    icon.shade("w", "W", lambda x, y: y > 10)
    icon.ell(16, 19, 10, 9, "y")  # Pip's round face
    icon.shade("y", "h", lambda x, y: (x - 16) + (y - 19) < -9)
    icon.shade("y", "Y", lambda x, y: y > 24)
    for ex in (11, 19):
        icon.rect(ex, 16, ex + 2, 19, "e")
        icon.rect(ex + 1, 17, ex + 2, 19, "k")
        icon.dot(ex + 1, 17, "e")
    icon.line(14, 23, 15, 24, "l").line(16, 24, 18, 23, "l")  # a smile
    icon.ell(9.5, 22, 1.6, 1.0, "r").ell(22.5, 22, 1.6, 1.0, "r")
    icon.line(12, 11, 9, 4, "l").dot(8, 3, "l").dot(9, 3, "l").dot(8, 4, "l")
    icon.line(20, 11, 23, 4, "l").dot(24, 3, "l").dot(23, 3, "l").dot(24, 4, "l")
    icon.outline("o")
    pal = {**PIP, "s": "#ffb52e", "S": "#d9771c", "H": "#ffd76a"}
    return {"hud_pip": spr(h, PIP), "icon": spr(icon, pal)}


# ---- the hive and the road ---------------------------------------------------------------------------------------------------------------------

def _bee(body="y", crown=False, big=False, cane=False, wing_up=True, brows=False, scarf=None):
    """A grown bee of the hive (16x18, or 20x24 for the Queen): the same family as Pip, rounder and taller."""
    W, H = (20, 24) if big else (16, 18)
    c = Canvas(W, H)
    by = H - 6
    rx, ry = (6.6, 5.6) if big else (5.2, 4.6)
    if wing_up:
        c.ell(W * 0.36, by - ry - 1.6, 3.2 if big else 2.6, 3.0 if big else 2.4, "w")
        c.ell(W * 0.24, by - ry - 0.4, 2.4, 2.0, "w")
    else:
        c.ell(W * 0.3, by - ry + 0.6, 3.8, 1.8, "w")
    c.shade("w", "W", lambda x, y: y > by - ry - 1)
    c.ell(W * 0.38, by, rx, ry, body)
    for sx in ((3, 6, 9) if big else (3, 6)):
        c.shade(body, "b", lambda x, y, sx=sx: x in (sx, sx + 1))
    c.shade(body, body.upper(), lambda x, y: y > by + ry * 0.4)
    hx, hy, hr = W * 0.68, by - ry - (2.2 if big else 1.2), 3.8 if big else 3.3
    c.ell(hx, hy, hr, hr, body)
    c.shade(body, "h", lambda x, y: (x - hx) + (y - hy) < -hr * 0.9)
    ex, ey = int(hx + 0.5), int(hy - 1)
    c.rect(ex, ey, ex + 1, ey + 1, "e").dot(ex + 1, ey, "k").dot(ex + 1, ey + 1, "k")
    c.dot(ex + 1, ey + 2, "r")
    if brows:
        c.line(ex - 1, ey - 1, ex + 2, ey - 2, "e")
    top = int(hy - hr)
    c.line(int(hx) - 1, top, int(hx), top - 2, "l").dot(int(hx) + 1, top - 3, "l")
    c.line(int(hx) + 1, top, int(hx) + 2, top - 2, "l").dot(int(hx) + 3, top - 2, "l")
    if crown:
        cy0 = top - 1
        c.rect(int(hx) - 3, cy0, int(hx) + 2, cy0 + 1, "c")
        for k in (-3, -1, 1):
            c.dot(int(hx) + k, cy0 - 1, "c")
        c.dot(int(hx) - 1, cy0, "m")
    if scarf:
        c.rect(int(hx) - 3, int(hy + hr) - 1, int(hx) + 1, int(hy + hr), scarf)
    if cane:
        c.line(W - 2, by - 3, W - 2, H - 2, "n").dot(W - 3, by - 4, "n").dot(W - 4, by - 4, "n")
    for fx in (int(W * 0.3), int(W * 0.5)):
        c.dot(fx, H - 2, "l")
    c.outline("o")
    return c


BEE = {"o": OUT, "y": "#ffc83d", "Y": "#e0952a", "b": "#5a3420", "h": "#ffe08a", "e": "#ffffff", "k": "#1c120e", "r": "#ff8a7a", "w": "#e8fbff",
       "W": "#a8dcf0", "l": "#3a2418", "c": "#ffe14d", "m": "#e0457a", "n": "#8a5a30", "s": "#4fa0e0"}


def hive():
    out = {}
    q = [_bee(crown=True, big=True), _bee(crown=True, big=True, wing_up=False)]
    out["queen"] = anim(q, {**BEE, "y": "#ffb84a", "Y": "#d9771c", "b": "#6a2a5a"}, 3)
    bw = [_bee(cane=True, brows=True), _bee(cane=True, brows=True, wing_up=False)]
    out["bramwell"] = anim(bw, {**BEE, "y": "#f0a040", "Y": "#b8662a", "b": "#2a1a14", "h": "#ffd9a0"}, 2)
    # the seven bees the Baron's lieutenants hold, each in their own colours (and a scarf, so they read as people, not workers)
    tints = [("#ffd23f", "#4fa0e0"), ("#ff9a3c", "#3fae49"), ("#e8b860", "#e0457a"), ("#9fd0ff", "#ffffff"), ("#ff8ab0", "#7b3fa0"), ("#b8e070", "#ff8a3d"), ("#d8d0c0", "#e63946")]
    for k, (body, scarf) in enumerate(tints, start=1):
        f = [_bee(scarf="s"), _bee(scarf="s", wing_up=False)]
        out[f"friend{k}"] = anim(f, {**BEE, "y": body, "Y": _darker(body), "s": scarf}, 4)
    # a thorn cocoon: the lieutenants keep their prisoner wrapped in brambles
    cc = Canvas(16, 20)
    cc.ell(8, 11, 6.5, 8.5, "t")
    for (x, y) in ((2, 6), (13, 8), (3, 15), (14, 14), (8, 2), (6, 19)):
        cc.dot(x, y, "T")
    cc.shade("t", "T", lambda x, y: (x + y) % 5 == 0)
    cc.rect(6, 9, 9, 12, "y")
    cc.rect(7, 10, 8, 10, "e")
    cc.dot(8, 10, "k")
    cc.outline("o")
    out["cocoon"] = spr(cc, {**BEE, **THORN, "o": THORN_OUT})
    return out


def _darker(c, k=0.72):
    c = c.lstrip("#")
    r, g, b = (int(c[i:i + 2], 16) for i in (0, 2, 4))
    return "#%02x%02x%02x" % (int(r * k), int(g * k), int(b * k))


def roadfolk():
    """The bugs Pip meets along the road (one per world): a ladybug, a cricket, a caterpillar, a firefly. Bump into one and they say a line."""
    out = {}
    pal = {"o": OUT, "e": "#ffffff", "k": "#1c120e", "r": "#ff8a7a"}
    # Dot the ladybug
    fr = []
    for k in range(2):
        c = Canvas(16, 16)
        c.ell(7, 10, 6, 4.6, "R")
        c.shade("R", "D", lambda x, y: y > 12)
        for (x, y) in ((4, 8), (7, 7), (9, 10), (5, 11)):
            c.dot(x, y, "k")
        c.line(7, 6, 7, 14, "k")
        c.ell(12.5, 9.5, 2.8, 2.8, "K")
        c.dot(13, 9, "e").dot(14, 9, "e")
        c.line(12, 7, 11 + k, 5, "K").line(14, 7, 15, 5 + k, "K")
        for fx in (4, 7, 10):
            c.dot(fx + k % 2, 15, "K")
        c.outline("o")
        fr.append(c)
    out["ladybug"] = anim(fr, {**pal, "R": "#e8432a", "D": "#a82a1a", "K": "#2a1a1a"}, 3)
    # Fennick the cricket (a fiddler)
    fr = []
    for k in range(2):
        c = Canvas(16, 16)
        c.ell(7, 10, 5.5, 3.2, "g")
        c.shade("g", "G", lambda x, y: y > 11)
        c.ell(12, 8, 2.6, 2.6, "g")
        c.dot(13, 7, "e").dot(13, 8, "k")
        c.line(11, 6, 8, 1 + k, "G").line(12, 6, 15, 2, "G")
        c.line(3, 11, 1, 15, "G").line(6, 12, 9, 15, "G")
        c.line(2, 8, 0, 6 - k, "n")  # the fiddle bow
        c.outline("o")
        fr.append(c)
    out["cricket"] = anim(fr, {**pal, "g": "#7cc444", "G": "#3f8a2a", "n": "#c8925a"}, 3)
    # Inchley the caterpillar
    fr = []
    for k in range(2):
        c = Canvas(16, 16)
        for i, x in enumerate((2, 5, 8, 11)):
            c.ell(x + 1.5, 12 - (1 if (i + k) % 2 else 0), 2.2, 2.4, "p" if i % 2 else "P")
        c.ell(13.5, 8.5, 2.6, 2.8, "p")
        c.dot(14, 8, "e").dot(15, 8, "k")
        c.dot(14, 5, "k").dot(12, 5, "k")
        c.outline("o")
        fr.append(c)
    out["caterpillar"] = anim(fr, {**pal, "p": "#9ad86a", "P": "#5cb04a"}, 2)
    # Lumi the firefly
    fr = []
    for k in range(2):
        c = Canvas(16, 16)
        c.ell(6, 10, 4.4, 3.2, "b")
        c.ell(3, 11, 2.6, 2.6, "L" if k == 0 else "l")
        c.ell(11, 8, 2.6, 2.6, "b")
        c.dot(12, 7, "e").dot(12, 8, "k")
        c.ell(7, 6, 3, 1.6, "w")
        c.line(11, 6, 12, 3, "b")
        c.outline("o")
        fr.append(c)
    out["firefly"] = anim(fr, {**pal, "b": "#4a3a6a", "L": "#fff27a", "l": "#d8f070", "w": "#d0e8ff"}, 3)
    return out


# ---- the Baron's creatures ------------------------------------------------------------------------------------------------------------------------
# Ordinary bugs of the meadow, gone sour where his thorns grew: each wears a little violet thorn and has a magenta glint in its eye.

FOE_BASE = {"o": THORN_OUT, "e": "#ffffff", "k": "#120a14", **THORN}


def _thorns(c, pts, ch="t"):
    for (x, y) in pts:
        c.dot(x, y, ch)
        c.dot(x, y + 1, "T")


def grubble():
    out = {}
    frames = []
    for k in range(2):
        c = Canvas(16, 16)
        for i, x in enumerate((3, 6.5, 10)):
            c.ell(x + 0.5, 11 - (k if i == 1 else 0), 3.2, 3.6 - (0.3 if i == 0 else 0), "b")
        c.ell(12.5, 10.5, 3.2, 3.6, "b")
        c.shade("b", "B", lambda x, y: y > 12)
        c.shade("b", "l", lambda x, y: y < 9 and x % 3 == 0)
        c.dot(13, 9, "e").dot(14, 9, "g")
        c.dot(14, 11, "k")
        _thorns(c, [(4, 6), (8, 5 + k), (11, 6)])
        for fx in (3, 6, 9, 12):
            c.dot(fx + (k if fx % 2 else 0), 15, "f")
        c.outline("o")
        frames.append(c)
    pal = {**FOE_BASE, "b": "#e0a060", "B": "#a0602a", "l": "#ffd8a0", "f": "#3a2018"}
    out["grubble"] = anim(frames, pal, 5)
    flat = Canvas(16, 16)
    flat.ell(8, 14, 7, 2, "b")
    flat.shade("b", "B", lambda x, y: y > 14)
    flat.dot(12, 13, "e")
    _thorns(flat, [(5, 11), (9, 11)])
    flat.outline("o")
    out["grubble_flat"] = spr(flat, pal)
    out["grubble_dead"] = spr(frames[0].vflipped(), pal)
    dusk = {"b": "#c08ad0", "B": "#7a4a98", "l": "#f0d0ff"}
    for k in ("grubble", "grubble_flat", "grubble_dead"):
        out["d" + k] = recolor(out[k], dusk)
    return out


def snail():
    out = {}
    pal = {**FOE_BASE, "s": "#a07ac8", "S": "#5a3a88", "q": "#e0c8ff", "b": "#a8c890", "B": "#6a8a50"}
    fr = []
    for k in range(2):
        c = Canvas(16, 16)
        c.rect(1, 12, 14, 14, "b")
        c.shade("b", "B", lambda x, y: y == 14)
        c.ell(12.5, 10, 2.4, 3.2, "b")
        c.line(12, 7, 11 - k, 4, "B").line(14, 7, 15, 4 + k, "B")
        c.dot(14, 9, "e").dot(14, 10, "g")
        c.ell(6.5, 8.5, 5.4, 5.4, "s")
        c.ell(6.5, 8.5, 3.2, 3.2, "q", only="s")
        c.ell(6.5, 8.5, 1.6, 1.6, "s", only="q")
        c.shade("s", "S", lambda x, y: y > 11)
        _thorns(c, [(3, 2), (7, 2), (10, 4)])
        c.outline("o")
        fr.append(c)
    out["snail"] = anim(fr, pal, 3)
    sh = Canvas(16, 16)
    sh.ell(8, 9, 6, 5.5, "s")
    sh.ell(8, 9, 3.6, 3.4, "q", only="s")
    sh.ell(8, 9, 1.8, 1.8, "s", only="q")
    sh.shade("s", "S", lambda x, y: y > 12)
    _thorns(sh, [(4, 3), (8, 2), (12, 3)])
    sh.outline("o")
    out["shell"] = spr(sh, pal)
    out["shell_spin"] = anim([sh, sh.flipped().vflipped()], pal, 14)
    out["snail_dead"] = spr(sh.vflipped(), pal)
    return out


def wasp():
    pal = {**FOE_BASE, "y": "#f0c040", "Y": "#b07a20", "p": "#5a2a7a", "w": "#f0e8ff", "W": "#b8a8e0"}
    fr = []
    for k in range(2):
        c = Canvas(16, 16)
        if k == 0:
            c.ell(7, 4.5, 3, 2.4, "w")
        else:
            c.ell(6, 7.5, 3.6, 1.4, "w")
        c.shade("w", "W", lambda x, y: y > 6)
        c.ell(5.5, 10.5, 4.6, 3, "y")
        c.shade("y", "p", lambda x, y: x in (3, 6))
        c.shade("y", "Y", lambda x, y: y > 12)
        c.dot(0, 11, "k").dot(1, 11, "p")
        c.ell(11.5, 9, 3, 3, "p")
        c.dot(12, 8, "g").dot(13, 8, "e")
        c.line(11, 6, 10, 3, "T").line(13, 6, 14, 3, "T")
        c.dot(12, 12, "k").dot(9, 13, "k")
        c.outline("o")
        fr.append(c)
    return {"wasp": anim(fr, pal, 14), "wasp_dead": spr(fr[0].vflipped(), pal)}


def spikebug():
    pal = {**FOE_BASE, "r": "#6a3a8a", "R": "#3a1a58", "q": "#9a6ac8"}
    fr = []
    for k in range(2):
        c = Canvas(16, 16)
        c.ell(7.5, 11, 6.4, 4.2, "r")
        c.shade("r", "q", lambda x, y: y < 9)
        c.shade("r", "R", lambda x, y: y > 13)
        for (x, top) in ((3, 4), (6, 3), (9, 3), (12, 4)):
            c.line(x, top, x, 7, "t")
            c.dot(x, top, "g")
        c.ell(13.5, 12, 2.2, 2.2, "R")
        c.dot(14, 11, "g")
        for fx in (3, 6, 9, 12):
            c.dot(fx + k, 15, "k")
        c.outline("o")
        fr.append(c)
    return {"spikebug": anim(fr, pal, 6), "spikebug_dead": spr(fr[0].vflipped(), pal)}


def snapvine():
    pal = {**FOE_BASE, "v": "#4a8a3a", "V": "#2a5a22", "m": "#b04ad0", "M": "#6a1f88", "w": "#ffe0f0"}
    fr = []
    for k in range(2):
        c = Canvas(12, 22)
        c.line(6, 10, 6, 21, "v").line(5, 12, 5, 21, "V")
        c.ell(3.5, 15, 2.6, 1.4, "v")
        c.ell(8.5, 18, 2.6, 1.4, "v")
        if k == 0:
            c.ell(6, 5.5, 5, 5, "m")
            c.shade("m", "M", lambda x, y: y > 7)
            c.line(2, 5, 10, 5, "k")
            for x in (3, 5, 7, 9):
                c.dot(x, 4, "w").dot(x + 1, 6, "w")
        else:
            c.ell(6, 6, 4.6, 5.4, "m")
            c.shade("m", "M", lambda x, y: x > 6)
        for (x, y) in ((1, 3), (11, 3), (6, 0)):
            c.dot(x, y, "t")
        c.outline("o")
        fr.append(c)
    return {"snapvine": anim(fr, pal, 3)}


def hazards():
    out = {}
    pal = {**FOE_BASE, "y": "#ffe36b", "o2": "#ff8c1a", "O": "#e0401a"}
    c = Canvas(10, 10)
    c.ell(5, 5, 4.4, 4.4, "O")
    c.ell(5, 4.5, 3, 3, "a", only="O")
    c.ell(4.5, 4, 1.4, 1.4, "y", only="a")
    c.outline("o")
    ember = {"o": "#3a0d00", "O": "#e0401a", "a": "#ff8c1a", "y": "#ffe36b"}
    out["cinder"] = anim([c, c.flipped()], ember, 8)
    # the thorn whirl's burr: a spiky violet seed with a hot core
    b = Canvas(8, 8)
    b.ell(4, 4, 2.6, 2.6, "t")
    b.dot(4, 4, "g").dot(3, 4, "g")
    for (x, y) in ((4, 0), (0, 4), (7, 4), (4, 7), (1, 1), (6, 6)):
        b.dot(x, y, "T")
    out["orb"] = anim([b, b.flipped()], FOE_BASE, 10)
    return out


# ---- what Pip finds ------------------------------------------------------------------------------------------------------------------------------

def items():
    out = {}
    hp = {"o": "#7a3a08", "y": "#ffc531", "Y": "#e08a10", "w": "#fff6c0"}
    fr = []
    for k in range(3):
        c = Canvas(10, 12)
        c.ell(5, 7.5, 3.8, 3.8, "y")
        c.line(5, 1, 5, 4, "y").line(4, 3, 6, 3, "y")
        c.shade("y", "Y", lambda x, y: x + y > 12)
        c.dot(3 + (k == 1), 6, "w").dot(3, 7 - (k == 2), "w")
        c.outline("o")
        fr.append(c)
    out["honey"] = anim(fr + [fr[0]], hp, 5)
    thin = Canvas(10, 12)
    thin.ell(5, 7.5, 1.6, 3.8, "y").line(5, 1, 5, 4, "y")
    thin.outline("o")
    out["honey_spin"] = anim([fr[0], thin, fr[0].flipped(), thin], hp, 16)
    # royal jelly: a little clay pot brimming with pearly jelly
    c = Canvas(14, 14)
    c.ell(7, 10, 5, 3.6, "p")
    c.rect(3, 6, 10, 9, "p")
    c.shade("p", "P", lambda x, y: x > 8)
    c.ell(7, 5, 4.6, 2.6, "j")
    c.ell(6, 3.5, 2.6, 1.6, "j")
    c.dot(5, 3, "w").dot(4, 4, "w")
    c.outline("o")
    out["jelly"] = anim([c, c.shifted(0, -1)], {"o": OUT, "p": "#c8703a", "P": "#8a4a22", "j": "#fff0f8", "w": "#ffffff"}, 3)
    # firebloom: an ember-red poppy heavy with pollen
    fr = []
    for k in range(2):
        c = Canvas(14, 14)
        c.line(7, 8, 7, 13, "v").ell(4, 11, 2, 1, "v")
        for (px, py) in ((7, 3), (3.5, 6), (10.5, 6), (5, 9.5), (9, 9.5)):
            c.ell(px, py, 2.8, 2.4, "f")
        c.shade("f", "F", lambda x, y: y > 7)
        c.ell(7, 6.5, 2, 2, "y" if k == 0 else "Y")
        c.dot(6, 6, "w")
        c.outline("o")
        fr.append(c)
    out["firebloom"] = anim(fr, {"o": OUT, "v": "#3f8a2a", "f": "#ff5a2a", "F": "#c8321a", "y": "#ffe36b", "Y": "#ffb02e", "w": "#ffffff"}, 4)
    # sunpetal: a golden five-petalled bloom that turns
    fr = []
    import math
    for k in range(3):
        c = Canvas(14, 14)
        for j in range(5):
            a = -math.pi / 2 + 2 * math.pi * j / 5 + k * 0.42
            c.ell(7 + 4 * math.cos(a), 7 + 4 * math.sin(a), 2.4, 2.4, "s")
        c.ell(7, 7, 2.6, 2.6, "c")
        c.dot(6, 7, "k").dot(8, 7, "k")
        c.outline("o")
        fr.append(c)
    out["starpetal"] = anim(fr, {"o": "#8a4a00", "s": "#fff27a", "c": "#ffb02e", "k": "#5a2a00"}, 10)
    # four-leaf clover: a life
    c = Canvas(14, 14)
    for (px, py) in ((7, 3.2), (3.2, 7), (10.8, 7), (7, 10.8)):
        c.ell(px, py, 2.7, 2.7, "g")
    for (x, y) in ((7, 5), (7, 9), (5, 7), (9, 7)):
        c.dot(x, y, "G")
    c.shade("g", "G", lambda x, y: x + y > 16)
    c.line(9, 9, 12, 13, "G")
    c.dot(6, 2, "l").dot(2, 6, "l")
    c.outline("o")
    out["clover"] = anim([c, c.shifted(0, -1)], {"o": "#14401a", "g": "#5cd65c", "G": "#2f9a3a", "l": "#b8f59a"}, 3)
    # pollen puff (what pollen Pip throws)
    fr = []
    for k in range(2):
        c = Canvas(8, 8)
        c.ell(4, 4, 3.2 - k * 0.4, 3.2 - k * 0.4, "y")
        c.ell(3.5, 3.5, 1.4, 1.4, "w", only="y")
        c.outline("o")
        fr.append(c)
    out["seed"] = anim(fr, {"o": "#a8321a", "y": "#ffb02e", "w": "#fff6c0"}, 12)
    # a shard of the Heartcomb: a glowing golden hexagon
    fr = []
    for k in range(4):
        c = Canvas(16, 16)
        for y in range(1, 15):
            d = min(y - 1, 14 - y)
            half = min(6, 1 + d * 2)
            c.rect(8 - half, y, 7 + half, y, "a")
        c.shade("a", "A", lambda x, y: x + y > 17)
        c.shade("a", "w", lambda x, y, k=k: (x + y) % 9 == (k * 2) % 9 and y < 10)
        c.rect(6, 6, 9, 9, "c")
        c.outline("o")
        if k % 2:
            for (x, y) in ((1, 1), (14, 2), (13, 14)):
                c.dot(x, y, "w")
        fr.append(c)
    out["shard"] = anim(fr, {"o": "#8a4a00", "a": "#ffd23f", "A": "#e09b00", "w": "#fffbe0", "c": "#fff27a"}, 6)
    return out


def all_cast():
    out = {}
    for part in (pip_sprites(), pip_icons(), hive(), roadfolk(), grubble(), snail(), wasp(), spikebug(), snapvine(), hazards(), items()):
        for k, v in part.items():
            assert k not in out, k
            out[k] = v
    return out
