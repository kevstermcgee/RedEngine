"""The eight bosses: Baron Thornwick and the seven lieutenants he gave a shard of the Heartcomb to. Each is a creature of its world, soured by his thorns
(violet thorns, a magenta eye), and each has a readable tell: a pose before the move that hurts, and a pose in which it can be stomped.

Sprite names: `<boss>` (its usual pose, animated), `<boss>_<pose>` for the others, `<boss>_hurt` (blinking: just hit, can't be hit again yet) and
`<boss>_dead` (upside down, falling). Projectiles are their own small sprites.
"""
import math

from canvas import Canvas
from cast import FOE_BASE, THORN_OUT, anim, spr


def blinking(c, pal):
    return {"frames": [c.rows(), ["." * c.w] * c.h], "palette": pal, "fps": 14}


def thorn_crown(c, x0, y0, n=3, step=3):
    for k in range(n):
        x = x0 + k * step
        c.line(x, y0, x, y0 - 3, "t").dot(x, y0 - 4, "g")


def eye(c, x, y, big=False):
    c.rect(x, y, x + 1, y + (1 if big else 0), "e")
    c.dot(x + 1, y, "g")


def legs(c, xs, y, phase, n=2, ch="k"):
    for i, x in enumerate(xs):
        d = 1 if (i + phase) % 2 else -1
        c.line(x, y, x + d, y + n, ch)


# ---- 1. Grubmaw, the Larder Grub (Meadowlands) -------------------------------------------------------------------------------------------------
def grubmaw():
    pal = {**FOE_BASE, "b": "#e8b070", "B": "#a8682a", "l": "#ffe0b0", "m": "#7a1f3a", "w": "#fff6e8"}
    fr = []
    for k in range(2):
        c = Canvas(32, 24)
        for i, x in enumerate((6, 12, 18)):
            c.ell(x, 15 - (k if i == 1 else 0), 6, 7.4 - (0.6 if i == 0 else 0), "b")
        c.ell(25, 13.5, 6.4, 7.6, "b")
        c.shade("b", "B", lambda x, y: y > 18)
        c.shade("b", "l", lambda x, y: y < 11 and x % 6 in (0, 1))
        for x in (9, 15, 21):
            c.line(x, 9, x, 21, "B")
        c.ell(27, 17, 3.2, 2.6 + k * 0.6, "m")  # the maw
        c.dot(26, 16, "w").dot(28, 16, "w")
        eye(c, 26, 9, big=True)
        thorn_crown(c, 20, 6, n=3, step=3)
        legs(c, (5, 9, 13, 17, 21), 21, k)
        c.outline("o")
        fr.append(c)
    return {"grubmaw": anim(fr, pal, 4), "grubmaw_hurt": blinking(fr[0], pal), "grubmaw_dead": spr(fr[0].vflipped(), pal)}


# ---- 2. Madam Duskwing, the moth who hoards the light (Duskwood) ---------------------------------------------------------------------------------
def duskwing():
    pal = {**FOE_BASE, "w": "#8a6ac0", "W": "#4a3a80", "p": "#d8b8f0", "y": "#ffe08a", "b": "#5a4a6a", "f": "#e8e0f0"}
    def body(c, wing):
        if wing == "up":
            c.ell(11, 7, 9, 6.5, "w")
            c.ell(9, 13, 6, 4, "w")
        elif wing == "down":
            c.ell(11, 15, 9.5, 5.5, "w")
            c.ell(9, 10, 6, 3, "w")
        elif wing == "fold":
            c.ell(13, 8, 5, 8, "w")
        else:  # spread low on the ground
            c.ell(10, 18, 10, 4, "w")
            c.ell(22, 18, 10, 4, "w")
        c.shade("w", "W", lambda x, y: (x + y) % 7 == 0)
        if wing in ("up", "down", "low"):
            ex, ey = (11, 7) if wing == "up" else ((11, 15) if wing == "down" else (8, 18))
            c.ell(ex, ey, 2.6, 2.2, "y", only="wW")
            c.ell(ex, ey, 1.2, 1.0, "k", only="y")
        c.ell(19, 13, 6, 4.2, "b")
        c.shade("b", "f", lambda x, y: y < 11)
        c.ell(25.5, 11, 3.6, 3.6, "f")
        eye(c, 26, 10, big=True)
        c.line(25, 7, 22, 2, "p").line(27, 7, 30, 2, "p")
        c.dot(21, 2, "t").dot(31, 2, "t")
        c.outline("o")
        return c
    fly = [body(Canvas(32, 24), "up"), body(Canvas(32, 24), "down")]
    dive = body(Canvas(32, 24), "fold")
    rest = [body(Canvas(32, 24), "low"), body(Canvas(32, 24), "low").shifted(0, 1)]
    dust = []
    for k in range(2):
        d = Canvas(8, 8)
        for (x, y) in ((3, 2), (5, 4), (2, 5), (4, 6), (6, 1)):
            d.dot((x + k) % 8, y, "p")
        d.ell(4, 4, 1.6, 1.6, "w")
        dust.append(d)
    return {
        "duskwing": anim(fly, pal, 6),
        "duskwing_dive": spr(dive, pal),
        "duskwing_rest": anim(rest, pal, 2),
        "duskwing_hurt": blinking(rest[0], pal),
        "duskwing_dead": spr(fly[0].vflipped(), pal),
        "dust": anim(dust, pal, 6),
    }


# ---- 3. Captain Burr, the bramble weevil (Autumn Hollow) ----------------------------------------------------------------------------------------
def burr():
    pal = {**FOE_BASE, "a": "#a0522d", "A": "#5a2a14", "h": "#d88a4a", "s": "#c8a060", "y": "#ffe36b"}
    def walker(k):
        c = Canvas(24, 24)
        c.ell(10, 14, 8, 6.4, "a")
        c.shade("a", "h", lambda x, y: y < 11)
        c.shade("a", "A", lambda x, y: y > 17)
        for (x, y) in ((4, 9), (8, 7), (12, 7), (16, 9)):
            c.line(x, y, x - 1, y - 3, "t")
        c.ell(19, 12, 3.6, 3.6, "a")
        c.line(21, 12, 23, 17, "A")  # the snout
        eye(c, 19, 10)
        c.line(18, 9, 20, 6, "s").dot(21, 5, "s")  # a little captain's feather
        legs(c, (5, 9, 13, 16), 19, k, n=3)
        c.outline("o")
        return c
    def ball(k):
        c = Canvas(24, 24)
        c.ell(12, 13, 8.6, 8.6, "a")
        c.shade("a", "A", lambda x, y: (x + y + k * 3) % 6 < 2)
        for j in range(8):
            a = 2 * math.pi * j / 8 + k * 0.4
            x, y = 12 + 10 * math.cos(a), 13 + 10 * math.sin(a)
            c.line(int(12 + 7.5 * math.cos(a)), int(13 + 7.5 * math.sin(a)), int(x), int(y), "t")
            c.dot(int(x), int(y), "g")
        c.outline("o")
        return c
    def dizzy(k):
        c = walker(0)
        c2 = Canvas(24, 24)
        c2.px = {p: v for p, v in c.px.items()}
        for j in range(3):
            a = 2 * math.pi * j / 3 + k * 1.0
            c2.dot(int(18 + 4 * math.cos(a)), int(3 + 1.6 * math.sin(a)), "y")
        return c2
    w = [walker(0), walker(1)]
    return {
        "burr": anim(w, pal, 5),
        "burr_ball": anim([ball(0), ball(1)], pal, 14),
        "burr_dizzy": anim([dizzy(0), dizzy(1)], pal, 6),
        "burr_hurt": blinking(w[0], pal),
        "burr_dead": spr(w[0].vflipped(), pal),
    }


# ---- 4. Rimeweaver, the frost spider (Frostvale) ------------------------------------------------------------------------------------------------
def rimeweaver():
    pal = {**FOE_BASE, "s": "#b8d8f0", "S": "#6a8ab0", "i": "#ffffff", "b": "#3a4a6a", "c": "#d0f0ff"}
    def spider(k, hanging):
        c = Canvas(32, 24)
        cy = 10 if hanging else 14
        for i, x in enumerate((6, 10, 14, 18)):
            d = 2 if (i + k) % 2 else 0
            c.line(x + 4, cy, x - 3, cy + 7 - d, "S")
            c.line(x + 4, cy, x + 11, cy + 7 - (2 - d), "S")
        c.ell(16, cy - 1, 8, 6, "s")
        c.shade("s", "S", lambda x, y: y > cy + 2)
        c.shade("s", "i", lambda x, y: (x * 3 + y) % 11 == 0)
        for x in (12, 16, 20):
            c.line(x, cy - 6, x, cy - 9, "t")
        for x in (13, 16, 19):
            eye(c, x - 1, cy, big=False)
        if hanging:
            c.line(16, 0, 16, cy - 7, "c")
        c.outline("o")
        return c
    hang = [spider(0, True), spider(1, True)]
    ground = [spider(0, False), spider(1, False)]
    ic = Canvas(6, 12)
    for y in range(12):
        half = max(0, 2 - y // 4)
        ic.rect(3 - half - 1, y, 2 + half, y, "c")
    ic.shade("c", "i", lambda x, y: x == 2 and y < 6)
    ic.outline("o")
    return {
        "rime": anim(hang, pal, 4),
        "rime_ground": anim(ground, pal, 6),
        "rime_hurt": blinking(ground[0], pal),
        "rime_dead": spr(ground[0].vflipped(), pal),
        "icicle": spr(ic, {**pal, "o": "#4a6a9a"}),
    }


# ---- 5. Sandmaw, the antlion (Sunset Dunes) -------------------------------------------------------------------------------------------------------
def sandmaw():
    pal = {**FOE_BASE, "d": "#e8c070", "D": "#b08a40", "a": "#8a5a3a", "A": "#4a2a1a", "m": "#d8b890"}
    mound = []
    for k in range(2):
        c = Canvas(32, 12)
        c.ell(16, 12, 14, 7 + k, "d")
        c.shade("d", "D", lambda x, y: (x * 7 + y * 3 + k) % 9 == 0)
        c.line(10, 6 - k, 9, 3 - k, "t").line(22, 6 - k, 23, 3 - k, "t")
        c.outline("o")
        mound.append(c)
    up = []
    for k in range(2):
        c = Canvas(32, 24)
        c.ell(16, 22, 15, 5, "d")
        c.ell(16, 13, 8, 7, "a")
        c.shade("a", "A", lambda x, y: y > 16)
        c.line(16, 6, 6, 1 + k, "m").line(6, 1 + k, 4, 5, "m")  # mandibles open wide
        c.line(16, 6, 26, 1 + k, "m").line(26, 1 + k, 28, 5, "m")
        eye(c, 12, 10, big=True)
        eye(c, 18, 10, big=True)
        thorn_crown(c, 12, 7, n=3, step=4)
        c.shade("d", "D", lambda x, y: (x + y) % 5 == 0)
        c.outline("o")
        up.append(c)
    clod = Canvas(8, 8)
    clod.ell(4, 4, 3.2, 3.2, "d")
    clod.shade("d", "D", lambda x, y: x + y > 7)
    clod.outline("o")
    return {
        "sand_mound": anim(mound, pal, 6),
        "sandmaw": anim(up, pal, 5),
        "sandmaw_hurt": blinking(up[0], pal),
        "sandmaw_dead": spr(up[0].vflipped(), pal),
        "clod": spr(clod, {**pal, "o": "#6a4a1a"}),
    }


# ---- 6. Gloam, the lantern thief (Starlit Meadow) ---------------------------------------------------------------------------------------------------
def gloam():
    pal = {**FOE_BASE, "b": "#2a2a4a", "B": "#14142a", "L": "#fff27a", "l": "#c8d84a", "d": "#5a5a3a", "w": "#c0d0ff"}
    def fly(glow, k, wings=True):
        c = Canvas(24, 24)
        if wings:
            c.ell(10, 7 - k, 5.6, 3.4, "w")
            c.ell(15, 8 + k, 4.6, 2.6, "w")
        c.ell(8, 15, 7, 5.6, glow)
        c.ell(15, 12, 5.4, 4.6, "b")
        c.shade("b", "B", lambda x, y: y > 14)
        c.ell(19.5, 10, 3.4, 3.4, "b")
        eye(c, 20, 9)
        c.line(19, 6, 17, 2, "t").line(21, 6, 23, 2, "t")
        c.outline("o")
        return c
    lit = [fly("L", 0), fly("l", 1)]
    charge = [fly("L", 0), fly("L", 1)]
    for c in charge:
        for (x, y) in ((1, 9), (2, 21), (14, 22), (0, 15)):
            c.dot(x, y, "L")
    drowsy = fly("d", 0, wings=True)
    spark = []
    for k in range(2):
        s = Canvas(6, 6)
        s.ell(3, 3, 1.8 + k * 0.6, 1.8 + k * 0.6, "L")
        s.dot(3, 3, "e")
        spark.append(s)
    return {
        "gloam": anim(lit, pal, 6),
        "gloam_charge": anim(charge, pal, 12),
        "gloam_drowsy": spr(drowsy, pal),
        "gloam_hurt": blinking(drowsy, pal),
        "gloam_dead": spr(drowsy.vflipped(), pal),
        "spark": anim(spark, pal, 12),
    }


# ---- 7. Queen Vespa of the thorn wasps (Blossom Ridge) ---------------------------------------------------------------------------------------------
def vespa():
    pal = {**FOE_BASE, "y": "#f0b030", "Y": "#a06a10", "p": "#4a1a5a", "w": "#f0e8ff", "W": "#c0b0e8", "c": "#ffe14d", "s": "#2a0a2a"}
    def wasp(k, pose):
        c = Canvas(32, 28)
        if pose != "dive":
            c.ell(13, 7 - k * 2, 7, 4.4, "w")
            c.ell(9, 10 + k, 5, 3, "w")
            c.shade("w", "W", lambda x, y: y > 8)
        if pose == "dive":
            c.ell(16, 12, 5, 9, "y")
            c.shade("y", "p", lambda x, y: y in (9, 10, 14, 15))
            c.line(16, 21, 16, 27, "s")
            c.ell(16, 4, 4, 3.6, "p")
            eye(c, 14, 4)
            eye(c, 17, 4)
        else:
            c.ell(11, 17, 9, 6, "y")
            c.shade("y", "p", lambda x, y: x in (6, 7, 11, 12))
            c.shade("y", "Y", lambda x, y: y > 20)
            stinger_y = 27 if pose == "stuck" else 20
            c.line(3, 19, 1, stinger_y, "s")
            c.ell(23, 13, 5, 5, "p")
            eye(c, 24, 11, big=True)
            c.rect(20, 6, 26, 7, "c")
            for x in (20, 23, 26):
                c.dot(x, 5, "c")
            c.dot(23, 6, "g")
            c.line(22, 8, 20, 4, "t") if pose == "stuck" else None
            legs(c, (14, 18, 21), 22, k, n=3)
        c.outline("o")
        return c
    hover = [wasp(0, "hover"), wasp(1, "hover")]
    stuck = [wasp(0, "stuck"), wasp(1, "stuck").shifted(1, 0)]
    return {
        "vespa": anim(hover, pal, 12),
        "vespa_dive": spr(wasp(0, "dive"), pal),
        "vespa_stuck": anim(stuck, pal, 6),
        "vespa_hurt": blinking(stuck[0], pal),
        "vespa_dead": spr(hover[0].vflipped(), pal),
    }


# ---- 8. Baron Thornwick (The Ashlands) ------------------------------------------------------------------------------------------------------------
def baron():
    pal = {**FOE_BASE, "p": "#3a2a4a", "P": "#1e1428", "q": "#6a5a7a", "h": "#8a3a2a", "H": "#4a1a14", "c": "#ffd23f", "C": "#b07a10", "r": "#a02a5a",
           "y": "#ffe36b"}
    def beetle(k, pose):
        c = Canvas(48, 40)
        low = 3 if pose in ("wind", "charge") else 0
        # the thorn cloak over his shell
        c.ell(20, 24, 17, 11, "r")
        for x in range(5, 36, 5):
            c.line(x, 15 + (x % 3), x - 2, 10 + (x % 3), "t")
            c.dot(x - 2, 9 + (x % 3), "g")
        c.ell(21, 25, 14, 9.4, "p")
        c.shade("p", "q", lambda x, y: y < 21 and x < 24)
        c.shade("p", "P", lambda x, y: y > 30)
        c.line(21, 16, 21, 34, "P")
        # head and the great horns (stag mandibles)
        hx, hy = 37, 22 + low
        c.ell(hx, hy, 6, 5.4, "p")
        c.shade("p", "P", lambda x, y: y > hy + 2 and x > 31)
        c.line(hx + 3, hy - 3, hx + 10, hy - 10 + low * 2, "h").line(hx + 10, hy - 10 + low * 2, hx + 7, hy - 13 + low * 2, "h")
        c.line(hx + 4, hy + 1, hx + 10, hy - 3 + low, "h").line(hx + 10, hy - 3 + low, hx + 9, hy - 6 + low, "H")
        eye(c, hx + 1, hy - 2, big=True)
        # his crown
        cx0 = hx - 5
        c.rect(cx0, hy - 8, cx0 + 6, hy - 6, "c")
        for x in (cx0, cx0 + 3, cx0 + 6):
            c.dot(x, hy - 9, "c")
        c.dot(cx0 + 3, hy - 7, "g")
        legs(c, (10, 16, 22, 28), 33, k, n=4, ch="P")
        if pose == "dazed":
            for j in range(3):
                a = 2 * math.pi * j / 3 + k
                c.dot(int(36 + 6 * math.cos(a)), int(8 + 2 * math.sin(a)), "y")
        c.outline("o")
        return c
    walk = [beetle(0, "walk"), beetle(1, "walk")]
    seed = Canvas(8, 8)
    seed.ell(4, 4, 2.6, 2.6, "r")
    for (x, y) in ((4, 0), (0, 4), (7, 4), (4, 7)):
        seed.dot(x, y, "t")
    seed.dot(4, 4, "g")
    seed.outline("o")
    spike = Canvas(16, 16)
    for (x, h) in ((2, 9), (5, 14), (8, 11), (11, 15), (14, 8)):
        spike.line(x, 15, x, 15 - h, "t")
        spike.dot(x, 15 - h, "g")
        spike.dot(x - 1, 15, "T").dot(x + 1, 15, "T")
    spike.outline("o")
    warn = Canvas(16, 6)
    for x in range(1, 15, 3):
        warn.dot(x, 4, "g").dot(x + 1, 5, "t")
    return {
        "baron": anim(walk, pal, 4),
        "baron_wind": spr(beetle(0, "wind"), pal),
        "baron_charge": anim([beetle(0, "charge"), beetle(1, "charge")], pal, 12),
        "baron_dazed": anim([beetle(0, "dazed"), beetle(1, "dazed")], pal, 4),
        "baron_hurt": blinking(walk[0], pal),
        "baron_dead": spr(walk[0].vflipped(), pal),
        "thornseed": spr(seed, pal),
        "spike": spr(spike, pal),
        "spike_warn": anim([warn, Canvas(16, 6)], pal, 8),
    }


def all_bosses():
    out = {}
    for part in (grubmaw(), duskwing(), burr(), rimeweaver(), sandmaw(), gloam(), vespa(), baron()):
        for k, v in part.items():
            assert k not in out, k
            out[k] = v
    return out
