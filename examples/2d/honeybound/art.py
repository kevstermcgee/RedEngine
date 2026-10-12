"""Honeybound's pixel art: every sprite as palette rows (`describe 2d`: "." is transparent, at most 64 px a side).

Characters and items are drawn by hand below; tiles are drawn by small functions (a brick is a formula, soil is seeded specks) so a tile set
is consistent and a new world's palette is one dict. Every sprite of one thing (walk, idle, air) has the same size: the engine checks it.
"""

# ---- helpers -------------------------------------------------------------------------------------------------------------------------------


def spr(rows, palette, fps=None):
    s = {"rows": rows, "palette": palette}
    if fps:
        s["fps"] = fps
    return s


def anim(frames, palette, fps):
    return {"frames": frames, "palette": palette, "fps": fps}


def recolor(sprite, changes):
    """The same picture with some palette letters given new colours (ember Tobin, a dusk grubble)."""
    out = dict(sprite)
    out["palette"] = {**sprite["palette"], **changes}
    return out


def flip_v(rows):
    return list(reversed(rows))


def blank(w, h):
    return ["." * w] * h


def grid(w, h, f):
    """Rows from a function of (x, y) giving one palette letter."""
    return ["".join(f(x, y) for x in range(w)) for y in range(h)]


def speck(x, y, seed):
    """A stable pseudo-random number in 0..99 for a pixel."""
    v = (x * 73856093) ^ (y * 19349663) ^ (seed * 83492791)
    v = (v ^ (v >> 13)) * 1274126177
    return (v ^ (v >> 16)) % 100


# ---- Tobin, the hero ---------------------------------------------------------------------------------------------------------------------------
# A young beekeeper: a wide straw-cream sun hat, a striped honey jumper, white gloves, brown boots. Faces right (the engine mirrors it).

TOBIN = {"h": "#f4e7c1", "H": "#c9ad6e", "f": "#f7c393", "k": "#2a1a12", "y": "#ffc531", "Y": "#d98e1a", "w": "#ffffff", "p": "#7a4a22", "b": "#5a3014", "r": "#e8836b"}
EMBER = {"h": "#ffffff", "H": "#d9d2c4", "y": "#ff6a2c", "Y": "#a8321a", "p": "#ffffff", "b": "#7a2414"}

_PIP_HEAD = [
    "................",
    ".....hhhhhh.....",
    "...hhhhhhhhhh...",
    "..HHHhhhhhhHHH..",
    "....ffffffff....",
    "....fffkfffk....",
    "....ffffffff....",
    ".....frffff.....",
]
_PIP_BODY = [
    "....yykkyyyy....",
    "...wyyyyyyyyw...",
    "...wkkkkkkkkw...",
    "....yyyyyyyy....",
]
_PIP_LEGS = {
    "stand": ["....pppppppp....", "....bbb..bbb....", "...bbbb..bbbb...", "................"],
    "stride": ["....pppppppp....", "...bbb....bbb...", "..bbbb....bbbb..", "................"],
    "close": ["....pppppppp....", ".....bbbbbb.....", ".....bbbbbbb....", "................"],
}
_PIP_AIR = [
    "................",
    ".....hhhhhh..w..",
    "...hhhhhhhhhhw..",
    "..HHHhhhhhhHHw..",
    "....ffffffffy...",
    "....fffkfffky...",
    "....ffffffffy...",
    ".....frffff.y...",
    "....yykkyyyyy...",
    "...wyyyyyyyy....",
    "...wkkkkkkkk....",
    "....yyyyyyyy....",
    "....pppppppp....",
    "...bbb...bbbb...",
    "..bbb.....bbb...",
    "................",
]


def tobin_small():
    stand = _PIP_HEAD + _PIP_BODY + _PIP_LEGS["stand"]
    walk = [_PIP_HEAD + _PIP_BODY + _PIP_LEGS["stride"], _PIP_HEAD + _PIP_BODY + _PIP_LEGS["close"]]
    return {"tobin": anim(walk, TOBIN, 10), "tobin_idle": spr(stand, TOBIN), "tobin_air": spr(_PIP_AIR, TOBIN)}


_BIG_HEAD = [
    "................",
    "......hhhh......",
    "....hhhhhhhh....",
    "...hhhhhhhhhh...",
    ".HHHHhhhhhhHHHH.",
    "..HHHHHHHHHHHH..",
    "....ffffffff....",
    "....ffffffff....",
    "....fffkfffk....",
    "....fffkfffk....",
    "....ffffffff....",
    ".....frrffff....",
    ".....ffffff.....",
]
_BIG_BODY = [
    "....yyyyyyyy....",
    "...yykkkkkkyy...",
    "..wyyyyyyyyyyw..",
    "..wyyyyyyyyyyw..",
    "..wykkkkkkkkyw..",
    "...yyyyyyyyyy...",
    "...yyyyyyyyyy...",
    "...ykkkkkkkkY...",
    "....yyyyyyyy....",
]
_BIG_LEGS = {
    "stand": ["....pppppppp....", "....ppp..ppp....", "....ppp..ppp....", "....ppp..ppp....", "....bbb..bbb....", "...bbbb..bbbb...", "...bbbb..bbbb...", "................", "................", "................"],
    "stride": ["....pppppppp....", "...ppp....ppp...", "...ppp....ppp...", "..ppp......ppp..", "..bbb......bbb..", ".bbbb......bbbb.", ".bbbb......bbbb.", "................", "................", "................"],
    "close": ["....pppppppp....", ".....pppppp.....", ".....pppppp.....", ".....pppppp.....", ".....bbbbbb.....", ".....bbbbbbb....", ".....bbbbbbb....", "................", "................", "................"],
}


def _big(legs, arms_up=False):
    body = list(_BIG_BODY)
    if arms_up:
        body = ["....yyyyyyyy.w..", "...yykkkkkkyyw..", "...yyyyyyyyyyw..", "...yyyyyyyyyy...", "...ykkkkkkkky...", "...yyyyyyyyyy...", "...yyyyyyyyyy...", "...ykkkkkkkkY...", "....yyyyyyyy...."]
    # One empty row on top so the feet sit on the bottom of a 28 px tall box centred in the 32 px picture.
    rows = ["................"] + _BIG_HEAD + body + _BIG_LEGS[legs]
    return rows[:32]


def tobin_big():
    return {
        "tobinbig": anim([_big("stride"), _big("close")], TOBIN, 10),
        "tobinbig_idle": spr(_big("stand"), TOBIN),
        "tobinbig_air": spr(_big("stride", arms_up=True), TOBIN),
    }


def tobin_variants():
    """Every Tobin sprite: small and big, normal and ember, plus the star-power and blinking versions (palette frames), the dead pose."""
    out = {}
    for name, s in {**tobin_small(), **tobin_big()}.items():
        out[name] = s
        out[name.replace("tobin", "ember", 1)] = recolor(s, EMBER)
    # Star power: the walk cycle, its colours cycling (three palettes, one per frame).
    star_cols = [{"y": "#ff4fd8", "Y": "#9b2fd0", "h": "#fff27a"}, {"y": "#5cf2ff", "Y": "#2a8fb0", "h": "#ffffff"}, {"y": "#9dff5c", "Y": "#3b9b2a", "h": "#ffd166"}]
    for base, size in (("tobin", 16), ("tobinbig", 32)):
        src = out[base]
        frames = []
        pal = dict(TOBIN)
        letters = "ijl"
        for k, cols in enumerate(star_cols):
            fr = []
            for row in (src["frames"][k % 2]):
                fr.append(row.replace("y", letters[k]).replace("Y", letters[k].upper()).replace("h", "aeo"[k]))
            frames.append(fr)
            pal[letters[k]] = cols["y"]
            pal[letters[k].upper()] = cols["Y"]
            pal["aeo"[k]] = cols["h"]
        out[base + "_star"] = anim(frames, pal, 15)
        # Blinking after a hit: the walk frames with every other one empty.
        out[base + "_blink"] = anim([src["frames"][0], blank(16, size), src["frames"][1], blank(16, size)], TOBIN, 16)
    out["tobin_dead"] = spr(
        [
            "................",
            "...hhhhhhhhhh...",
            "..HHHhhhhhhHHH..",
            "....ffffffff....",
            "...fkkffffkkf...",
            "....ffffffff....",
            ".w...frrrrf...w.",
            ".wy.yykkyyyy.yw.",
            "..yyyyyyyyyyyy..",
            "....kkkkkkkk....",
            "....yyyyyyyy....",
            "....pppppppp....",
            "....bbb..bbb....",
            "...bbbb..bbbb...",
            "................",
            "................",
        ],
        TOBIN,
    )
    return out


# ---- friends ----------------------------------------------------------------------------------------------------------------------------------

BEE = {"y": "#ffd23f", "k": "#1d1610", "w": "#dff6ff", "W": "#9fd8f0", "e": "#ffffff"}


def friends():
    buzz = [
        [
            "....ww....ww....",
            "...wWWw..wWWw...",
            "...wWWw..wWWw...",
            "....ww....ww....",
            ".....kyyyyk.....",
            "....kyyyyyyk....",
            "...yyekyyekyy...",
            "...yyyyyyyyyy...",
            "...kkkkkkkkkk...",
            "...yyyyyyyyyy...",
            "....kkkkkkkk....",
            ".....yyyyyy.....",
            "......kkkk......",
            ".......kk.......",
            "................",
            "................",
        ],
        [
            "................",
            "..ww........ww..",
            ".wWWw......wWWw.",
            "..wwW.....Www...",
            ".....kyyyyk.....",
            "....kyyyyyyk....",
            "...yyekyyekyy...",
            "...yyyyyyyyyy...",
            "...kkkkkkkkkk...",
            "...yyyyyyyyyy...",
            "....kkkkkkkk....",
            ".....yyyyyy.....",
            "......kkkk......",
            ".......kk.......",
            "................",
            "................",
        ],
    ]
    queen_pal = {**BEE, "c": "#ffe066", "C": "#e0a800", "r": "#d63a6a", "g": "#7cf0a0"}
    queen = [
        [
            "......c.c.c.....",
            ".....cCcCcCc....",
            ".....crcgcrc....",
            "......kyyyk.....",
            ".....kyyyyyk....",
            "....yyekyekyy...",
            "....yyyyyyyyy...",
            ".ww..kyyyyyk..ww",
            "wWWw.kkkkkkk.wWW",
            "wWWwyyyyyyyyywWW",
            ".ww.kkkkkkkkk.ww",
            "....yyyyyyyyy...",
            "....kkkkkkkkk...",
            "....yyyyyyyyy...",
            ".....kkkkkkk....",
            ".....yyyyyyy....",
            "......kkkkk.....",
            "......yyyyy.....",
            ".......kkk......",
            "........k.......",
            "................",
            "................",
            "................",
            "................",
        ]
    ]
    queen.append([r.replace("wWWw", "WwwW") for r in queen[0]])
    out = {"friend1": anim(buzz, BEE, 8), "queen": anim(queen, queen_pal, 4)}
    tints = {2: "#ff9a3c", 3: "#b07a3a", 4: "#8ad0ff", 5: "#ff6a8a", 6: "#9aff6a", 7: "#c8c8d8"}
    for k, col in tints.items():
        out[f"friend{k}"] = recolor(out["friend1"], {"y": col})
    return out


# ---- foes -------------------------------------------------------------------------------------------------------------------------------------

GRUB = {"b": "#d0782a", "B": "#6a2a0e", "l": "#f6c27a", "k": "#1e120a", "e": "#ffffff", "f": "#2a1a10"}
GRUB_DUSK = {"b": "#b05a8a", "B": "#5a1e44", "l": "#f0b0d0"}


def grubble():
    """A grubble: a segmented beetle grub with feelers and a row of little legs (faces right; the engine mirrors it)."""
    body = [
        "................",
        "................",
        "...........k.k..",
        "............kk..",
        ".....BBBBBBB....",
        "...BBbbBbbBbbB..",
        "..BbbbBbbbBbbeB.",
        ".BbbbBbbbBbbekB.",
        ".BbbbBbbbBbbbbBB",
        ".BlllBlllBllllB.",
        "..BBBBBBBBBBBB..",
    ]
    legs = [["...f.f.f.f.f.f..", "..f.f.f.f.f.f...", "................"], ["..f.f.f.f.f.f...", "...f.f.f.f.f.f..", "................"]]
    pad = ["................", "................"]
    walk = [pad + body + legs[0], pad + body + legs[1]]
    flat = ["................"] * 10 + [
        "................",
        "...BBBBBBBBBB...",
        ".BbbBbbbBbbbekB.",
        ".BlllBlllBllllB.",
        "..f.f.f.f.f.f...",
        "................",
    ]
    out = {"grubble": anim(walk, GRUB, 6), "grubble_flat": spr(flat, GRUB), "grubble_dead": spr(flip_v(walk[0]), GRUB)}
    out["dgrubble"] = recolor(out["grubble"], GRUB_DUSK)
    out["dgrubble_flat"] = recolor(out["grubble_flat"], GRUB_DUSK)
    out["dgrubble_dead"] = recolor(out["grubble_dead"], GRUB_DUSK)
    return out


SNAIL = {"s": "#3fbf8f", "S": "#1f7a5a", "q": "#b8f5d8", "b": "#f0c890", "B": "#c08a50", "k": "#1d1610", "e": "#ffffff"}


def snail():
    a = [
        "................",
        "..k.k...........",
        "..k.k...........",
        "..e.e...........",
        "..b.b...SSSS....",
        "..bbb..SssssS...",
        ".bbbb.SsqqqssS..",
        ".bebb.SsqSSqsS..",
        ".bkbb.SsqSqqsS..",
        ".bbbb.SssSSssS..",
        ".bbbb..SssssS...",
        ".bbbbb..SSSS....",
        ".bbbbbbbbbbbbbb.",
        "..BBBBBBBBBBBBB.",
        "................",
        "................",
    ]
    b = [r for r in a]
    b[12] = "..bbbbbbbbbbbbbb"
    b[13] = "...BBBBBBBBBBBBB"
    b[1] = "...k.k.........."
    b[2] = "..k.k..........."
    shell = [
        "................",
        "................",
        "................",
        "......SSSS......",
        "....SSssssSS....",
        "...SsssqqsssS...",
        "..SssqqSSqqssS..",
        "..SsqSSssSSqsS..",
        "..SsqSqqqsSqsS..",
        "..SsqSqSSqSqsS..",
        "..SssqSqqSqssS..",
        "..SsssqSSqsssS..",
        "...SsssqqsssS...",
        "....SSSSSSSS....",
        "................",
        "................",
    ]
    spin = [shell, [r[::-1] for r in flip_v(shell)]]
    return {"snail": anim([a, b], SNAIL, 4), "shell": spr(shell, SNAIL), "shell_spin": anim(spin, SNAIL, 12), "snail_dead": spr(flip_v(shell), SNAIL)}


WASP = {"y": "#ffb703", "k": "#1d1610", "w": "#e8f8ff", "W": "#a9dcef", "r": "#e63946", "e": "#ffffff"}


def wasp():
    body = [
        ".....kyyyk......",
        "....kyyyyyk.....",
        "...yyerkyyyy....",
        "...yykkyyyyyk...",
        "....kkkkkkkkkk..",
        "....yyyyyyyyyyk.",
        ".....kkkkkkkkkk.",
        "......yyyyyyy.kk",
        ".......kkkkk....",
        "................",
    ]
    wings_up = ["......wWw.......", ".....wWWWw......", ".....wWWWw..wWw.", "......www..wWWw.", "............ww.."]
    wings_dn = ["................", "................", "................", "......www.......", ".....wWWWw..ww.."]
    f1 = ["................"] + wings_up + body
    f2 = ["................"] + wings_dn + body
    return {"wasp": anim([f1, f2], WASP, 12), "wasp_dead": spr(flip_v(f1), WASP)}


SPIKE = {"r": "#d62828", "R": "#7a1010", "w": "#fff3e0", "k": "#1d1610", "e": "#ffffff", "f": "#2a1a10"}


def spikebug():
    a = [
        "................",
        "...w...w...w....",
        "...ww..ww..ww...",
        "..wRRRRRRRRRw...",
        ".wRrrrrrrrrrRw..",
        ".RrrwrrrwrrrrR..",
        "wRrrrrrrrrrrrRw.",
        ".RrrrrwrrrrwrrRk",
        ".RrrrrrrrrrrrRek",
        "..RRRRRRRRRRRRkk",
        "...ff..ff..ff...",
        "..ff..ff..ff....",
    ]
    pad = ["................"] * 4
    b = list(a)
    b[10] = "....ff..ff..ff.."
    b[11] = "...ff..ff..ff..."
    return {"spikebug": anim([pad + a, pad + b], SPIKE, 6), "spikebug_dead": spr(flip_v(pad + a), SPIKE)}


VINE = {"g": "#4caf50", "G": "#1f6f2a", "m": "#ff5c8a", "M": "#b0203f", "w": "#ffffff", "k": "#1d1610"}


def snapvine():
    head_open = [
        "...GGG....GGG...",
        "..GgggG..GgggG..",
        ".GggwgG..GgwggG.",
        ".GgmmmG..GmmmgG.",
        ".GmmMmmGGmmMmmG.",
        ".GmMMMmmmmMMMmG.",
        ".GmmMmmmmmmMmmG.",
        "..GmmmmmmmmmmG..",
        "...GGmmmmmmGG...",
        ".....GGGGGG.....",
    ]
    head_shut = [
        "................",
        "................",
        "....GGGGGGGG....",
        "...GggwggwggG...",
        "..GggggggggggG..",
        "..GwmwmwmwmwmG..",
        "..GggggggggggG..",
        "...GggggggggG...",
        "....GGGGGGGG....",
        ".....GGGGGG.....",
    ]
    stem = [
        ".......Gg.......",
        "...gg..Gg.......",
        "..gGGg.Gg..gg...",
        "..gGgggGg.gGGg..",
        "...gggGGggggGg..",
        ".......Gggg.g...",
        ".......Gg.......",
        ".......Gg.......",
        ".......Gg.......",
        ".......Gg.......",
        ".......Gg.......",
        ".......Gg.......",
        ".......Gg.......",
        ".......Gg.......",
    ]
    return {"snapvine": anim([head_open + stem, head_shut + stem], VINE, 4)}


FIRE = {"o": "#ff8c1a", "O": "#ff3d00", "y": "#ffe36b", "w": "#ffffff", "k": "#3a0d00"}


def fire_things():
    cinder = [
        ["....oooo....", "..ooOOOOoo..", ".oOOyyyyOOo.", ".oOywyywyOo.", "oOyykyykyyOo", "oOyyyyyyyyOo", "oOyykkkkyyOo", ".oOyyyyyyOo.", ".oOOyyyyOOo.", "..ooOOOOoo..", "....oooo....", "............"],
        ["....o..o....", "..oooOOooo..", ".oOOyyyyOOo.", ".oOywyywyOo.", "oOyykyykyyOo", "oOyyyyyyyyOo", "oOyyykkyyyOo", ".oOyyyyyyOo.", ".oOOyyyyOOo.", "..ooOOOOoo..", "...o.oo.o...", "............"],
    ]
    orb = [["..oooo..", ".oOyyOo.", "oOywyyOo", "oyyyyyyo", "oyyyyyyo", "oOyyyyOo", ".oOyyOo.", "..oooo.."], ["..o..o..", ".oOOOOo.", "oOyyyyOo", "oyywyyyo", "oyyyyyyo", "oOyyyyOo", ".oOOOOo.", "..o..o.."]]
    breath = [
        ["................", "......oooo......", "...ooOOOOOOoo...", ".oOOyyyyyyyOOo..", "oOyyyywwyyyyyOoo", ".oOOyyyyyyyOOo..", "...ooOOOOOOoo...", "......oooo......"],
        ["................", ".....oo.oo......", "..oooOOOOOOooo..", "oOOyyyyyyyyOOOo.", "Oyyyywwwyyyyyyoo", "oOOyyyyyyyyOOOo.", "..oooOOOOOOooo..", ".....oo.oo......"],
    ]
    seed = [["..oo....", ".oyyo...", "oywyyo..", "oyyyyo..", ".oyyo...", "..oo....", "........", "........"], ["........", "..oooo..", ".oyyyyo.", ".oywyyo.", ".oyyyyo.", "..oooo..", "........", "........"]]
    return {"cinder": anim(cinder, FIRE, 8), "orb": anim(orb, FIRE, 10), "breath": anim(breath, FIRE, 10), "seed": anim(seed, FIRE, 12)}


BARON = {"p": "#6a3d9a", "P": "#3b1f5e", "q": "#a678de", "k": "#120a18", "r": "#ff3355", "w": "#ffffff", "c": "#ffd23f", "C": "#c98e00", "t": "#2f7a3a", "T": "#174a20"}


def baron():
    """Baron Thornwick: a crowned stag beetle in a thorn cloak, 32x32."""
    a = [
        "................................",
        "...........c..c..c..............",
        "..........cCccCccCc.............",
        "..........cccccccccc............",
        "kk.........PPPPPPPP.......kk....",
        ".kk......PPppppppppPP....kk.....",
        "..kk....PppqqppppqqppP..kk......",
        "...kk..PpprrwppppwrrppPkk.......",
        "....kkkPpprrrppppprrrppkk.......",
        ".....kkPppppppppppppppPk........",
        "......PPpppkkkkkkkkpppPP........",
        "....TTPpppkwkwkwkwkpppPTT.......",
        "...TtTPPpppkkkkkkkkpppPPTtT.....",
        "..TttTtPPPpppppppppPPPTtttT.....",
        "..TtTTtTPPPPPPPPPPPPPTtTTtT.....",
        ".TttTtttTpqqqqqqqqqqpTttTttT....",
        ".TtTTtTTTpqppppppppqpTTtTTtT....",
        ".TttTtttTpqpppppppqqpTttTttT....",
        "..TtTTtTTpqqppppppqqpTTtTtT.....",
        "..TttTttTppqqqqqqqqppTtTttT.....",
        "...TtTtTTpppppppppppTTtTtT......",
        "...TttTtTTppppppppppTtTttT......",
        "....TTtTtTTTPPPPPPPTTtTTT.......",
        ".....TTTTTTTTTTTTTTTTTTT........",
        "......kk...kk....kk...kk........",
        ".....kk...kk......kk...kk.......",
        "....kk...kk........kk...kk......",
        "...kkk..kkk........kkk..kkk.....",
        "................................",
        "................................",
        "................................",
        "................................",
    ]
    b = list(a)
    b[24] = ".......kk..kk....kk..kk........."
    b[25] = "......kk..kk......kk..kk........"
    b[26] = ".....kk..kk........kk..kk......."
    b[27] = "....kkk.kkk........kkk.kkk......"
    # shift the whole drawing right by 2 so it sits centred
    def centre(rows):
        return [(".." + r)[:32] for r in rows]
    return {"baron": anim([centre(a), centre(b)], BARON, 4), "baron_dead": spr(flip_v(centre(a)), BARON)}


# ---- items ------------------------------------------------------------------------------------------------------------------------------------

HONEY = {"y": "#ffd23f", "Y": "#e09b00", "o": "#a85d00", "w": "#fffbe0"}


def items():
    drop = [
        ["......o.........", ".....oyo........", ".....oyo........", "....oyyyo.......", "...oywyyyo......", "...oywyyyo......", "..oywyyyyYo.....", "..oyyyyyyYo.....", "..oyyyyyYYo.....", "...oyyyYYo......", "....oooo........", "................"],
        ["......o.........", ".....oyo........", ".....oyo........", "....oyyyo.......", "...oyyyyyo......", "...oywyyyo......", "..oywyyyyYo.....", "..oywyyyyYo.....", "..oyyyyyYYo.....", "...oyyyYYo......", "....oooo........", "................"],
        ["......o.........", ".....oyo........", ".....oyo........", "....oyyyo.......", "...oyyyyyo......", "...oyyyyyo......", "..oyyyyyyYo.....", "..oywyyyyYo.....", "..oywyyyYYo.....", "...oyyyYYo......", "....oooo........", "................"],
    ]
    # centre the drop in 12x12
    drop = [[r[1:13] for r in f] for f in drop]
    spin = [
        ["....oooo....", "...oyyyyo...", "..oywyyyYo..", "..oywyyyYo..", "..oyyyyyYo..", "..oyyyyYYo..", "..oyyyyYYo..", "..oyyyYYYo..", "...oyyYYo...", "....oooo....", "............", "............"],
        [".....oo.....", "....oyyo....", "....oyYo....", "....oyYo....", "....oyYo....", "....oyYo....", "....oyYo....", "....oyYo....", "....oyyo....", ".....oo.....", "............", "............"],
    ]
    jelly_pal = {"g": "#d8f3ff", "G": "#8ec9e8", "c": "#e8c07a", "C": "#a6762e", "y": "#ffd23f", "Y": "#f0a500", "w": "#ffffff", "k": "#3a2a10", "r": "#e8536b"}
    jelly = [
        "................",
        "....CCCCCCCC....",
        "....cccccccc....",
        "...rrrrrrrrrr...",
        "...GggggggggG...",
        "..GgwwgggggggG..",
        "..GwyyyyyyyygG..",
        "..GwyYyykyyyYG..",
        "..GgyyykkkyyYG..",
        "..GgyyyykyyyYG..",
        "..GgyyyyyyyyYG..",
        "..GgYyyyyyyYYG..",
        "...GgYYYYYYYG...",
        "....GGGGGGGG....",
        "................",
        "................",
    ]
    bloom_pal = {"o": "#ff7a1a", "O": "#d93a00", "y": "#ffe36b", "g": "#3fae49", "G": "#1f6f2a", "w": "#ffffff", "k": "#3a0d00"}
    bloom = [
        [
            "......oooo......",
            "...ooooOOoooo...",
            "..oOOoyyyyoOOo..",
            ".oOOoyywwyyoOOo.",
            ".oOoyyykkyyyoOo.",
            "..oOoyyyyyyoOo..",
            "...ooooOOoooo...",
            "......oooo......",
            ".......gg.......",
            "..gg...gg...gg..",
            "..gGg..gg..gGg..",
            "...gGg.gg.gGg...",
            "....gGggggGg....",
            ".....ggGGgg.....",
            ".......gg.......",
            "................",
        ]
    ]
    bloom.append([r.replace("o", "q").replace("O", "o").replace("q", "O") for r in bloom[0]])
    star_pal = {"s": "#ffe94a", "S": "#ff9f1c", "w": "#ffffff", "k": "#3a2a00", "t": "#ff6fd8", "T": "#c33fa8"}
    star = [
        [
            ".......ss.......",
            "......sSSs......",
            "......sSSs......",
            ".....sSwSSs.....",
            "sssssSwSSSSsssss",
            ".sSSSSSSSSSSSSs.",
            "..sSSSkSSkSSSs..",
            "...sSSkSSkSSs...",
            "....sSSSSSSs....",
            "....sSSSSSSs....",
            "...sSSSssSSSs...",
            "...sSSs..sSSs...",
            "..sSs......sSs..",
            "..ss........ss..",
            "................",
            "................",
        ]
    ]
    star.append([r.replace("s", "t").replace("S", "T") for r in star[0]])
    clover_pal = {"g": "#5ce05c", "G": "#1f8a2a", "w": "#ffffff", "k": "#123a12", "y": "#ffe36b"}
    clover = [
        "................",
        "....GG....GG....",
        "...GggG..GggG...",
        "..GgwggGGggwgG..",
        "..GggggGGgggggG.",
        "...GgggggggggG..",
        "....GGgkgkgGG...",
        "...GgggggggggG..",
        "..GgggggGgggggG.",
        "..GgwggG.GgwggG.",
        "...GggG...GggG..",
        "....GG..G..GG...",
        "........G.......",
        ".........G......",
        ".........G......",
        "................",
    ]
    return {
        "honey": anim(drop, HONEY, 5),
        "honey_spin": anim(spin, HONEY, 16),
        "jelly": spr(jelly, jelly_pal),
        "firebloom": anim(bloom, bloom_pal, 4),
        "starpetal": anim(star, star_pal, 10),
        "clover": spr(clover, clover_pal),
    }


# ---- tiles ------------------------------------------------------------------------------------------------------------------------------------

THEMES = {
    "meadow": {"grass": "#5cc44a", "grass2": "#2f8a2f", "dirt": "#c47a3c", "dirt2": "#8f5226", "dirt3": "#e6a25c", "brick": "#c4602c", "brick2": "#7a3416", "mortar": "#f0b27a"},
    "roots": {"grass": "#3f8fa0", "grass2": "#21566a", "dirt": "#3d5f78", "dirt2": "#24384a", "dirt3": "#5f86a0", "brick": "#2f8fa3", "brick2": "#164a5a", "mortar": "#8fd8e8"},
    "dusk": {"grass": "#4fa89a", "grass2": "#25685e", "dirt": "#8a5a8a", "dirt2": "#55305a", "dirt3": "#b07ab0", "brick": "#a0506a", "brick2": "#5a2038", "mortar": "#e6a0b8"},
    "caves": {"grass": "#6a5fb5", "grass2": "#3a2f7a", "dirt": "#3e3566", "dirt2": "#221c40", "dirt3": "#6a5f9a", "brick": "#5a4fa8", "brick2": "#2c2366", "mortar": "#b0a8f0"},
    "autumn": {"grass": "#e08a2c", "grass2": "#a0521a", "dirt": "#9a5a3a", "dirt2": "#6a3420", "dirt3": "#c4805a", "brick": "#b04a2a", "brick2": "#6a2414", "mortar": "#f0b08a"},
    "frost": {"grass": "#f4fbff", "grass2": "#a8d0e8", "dirt": "#7aa0c0", "dirt2": "#4a6a8a", "dirt3": "#a8c8e0", "brick": "#6aa0d0", "brick2": "#2a5a8a", "mortar": "#d8f0ff"},
    "dunes": {"grass": "#f0d080", "grass2": "#c0a050", "dirt": "#d8b060", "dirt2": "#a07a30", "dirt3": "#f0d090", "brick": "#c08040", "brick2": "#7a4a1a", "mortar": "#f8e0a0"},
    "blossom": {"grass": "#f7a8c8", "grass2": "#c0588a", "dirt": "#8a5a6a", "dirt2": "#5a3040", "dirt3": "#b07a8a", "brick": "#d06a8a", "brick2": "#7a2a4a", "mortar": "#ffd0e0"},
    "ash": {"grass": "#7a6a6a", "grass2": "#4a3a3a", "dirt": "#5a4a4a", "dirt2": "#2a2020", "dirt3": "#8a7070", "brick": "#8a3a2a", "brick2": "#4a1a10", "mortar": "#c08070"},
    "castle": {"grass": "#9a9aa8", "grass2": "#5a5a6a", "dirt": "#7a7a88", "dirt2": "#4a4a58", "dirt3": "#b0b0bc", "brick": "#8a8a96", "brick2": "#4a4a56", "mortar": "#c8c8d4"},
}


def ground_tiles(theme, t):
    """Grass-topped ground, plain soil and bricks in one world's colours (castle stone shares the shapes)."""
    pal = {"g": t["grass"], "G": t["grass2"], "d": t["dirt"], "D": t["dirt2"], "e": t["dirt3"], "b": t["brick"], "B": t["brick2"], "m": t["mortar"]}

    def soil(x, y, seed):
        n = speck(x, y, seed)
        return "D" if n < 9 else ("e" if n < 14 else "d")

    def top(x, y):
        edge = [2, 3, 2, 3, 3, 2, 4, 3, 2, 3, 3, 2, 3, 4, 3, 2][x]
        if y < edge - 1:
            return "g" if speck(x, y, 5) > 15 else "G"
        if y < edge:
            return "G"
        return soil(x, y, 7)

    def brick(x, y):
        row = y // 4
        if y % 4 == 3:
            return "m"
        off = 4 if row % 2 else 0
        if (x + off) % 8 == 7:
            return "m"
        return "B" if (y % 4 == 2 or (x + off) % 8 == 6) else "b"

    def stone(x, y):
        if x in (0,) or y in (0,):
            return "e"
        if x == 15 or y == 15:
            return "D"
        if (x, y) in ((1, 1), (14, 1), (1, 14), (14, 14)):
            return "D"
        return "d"

    out = {
        f"{theme}_top": spr(grid(16, 16, top), pal),
        f"{theme}_soil": spr(grid(16, 16, lambda x, y: soil(x, y, 3)), pal),
        f"{theme}_brick": spr(grid(16, 16, brick), pal),
        f"{theme}_stone": spr(grid(16, 16, stone), pal),
    }
    return out


def block_tiles():
    comb_pal = {"y": "#ffc531", "Y": "#d98e1a", "o": "#8a4a0a", "w": "#fff3b0", "k": "#7a3a00"}

    def comb(shine):
        drop = [
            "................",
            "................",
            "................",
            ".......kk.......",
            "......kwyk......",
            "......kwyk......",
            ".....kwyyyk.....",
            "....kwyyyyyk....",
            "....kwyyyyYk....",
            "....kyyyyyYk....",
            ".....kyyYYk.....",
            "......kkkk......",
            "................",
            "................",
            "................",
            "................",
        ]

        def cell(x, y):
            # a honeycomb: offset rows of hexagon-ish cells, 5 px wide, 4 px tall
            r = y // 4
            xx = (x + (2 if r % 2 else 0)) % 5
            yy = y % 4
            return yy == 0 and xx in (1, 2, 3) or (yy in (1, 2, 3) and xx == 0)

        def f(x, y):
            if x in (0, 15) or y in (0, 15):
                return "o"
            if drop[y][x] != ".":
                ch = drop[y][x]
                return "w" if (ch == "w" and shine) else ("y" if ch == "w" else ch)
            if cell(x, y):
                return "Y"
            if shine and (x + y) % 11 == shine:
                return "w"
            return "y"

        return grid(16, 16, f)

    used_pal = {"u": "#9a6a3a", "U": "#5e3c1a", "l": "#c89a62"}
    used = grid(16, 16, lambda x, y: "U" if x in (0, 15) or y in (0, 15) else ("l" if (x, y) in ((2, 2), (13, 2), (2, 13), (13, 13)) else "u"))
    hard_pal = {"h": "#c8a87a", "H": "#7a5a34", "l": "#ecd2a6"}

    def hard(x, y):
        if x == 0 or y == 0:
            return "l"
        if x == 15 or y == 15:
            return "H"
        if x + y < 6:
            return "l"
        if x + y > 24:
            return "H"
        return "h"

    log_pal = {"a": "#8a5228", "A": "#5a3214", "r": "#d9a066", "R": "#b07a40", "k": "#3a1e0a", "m": "#1e0f05"}
    # The log is two tiles wide: a top lip (a ring with the dark hole) and a bark body.
    lip_l = ["kkkkkkkkkkkkkkkk", "krrrrrrrrrrrrrrr", "krRRRRRRRRRRRRRR", "krRmmmmmmmmmmmmm", "krRmmmmmmmmmmmmm", "krRmmmmmmmmmmmmm", "krRRRRRRRRRRRRRR", "krrrrrrrrrrrrrrr", "kaaaAaaaaaAaaaaa", "kaaaAaaaaaAaaaaa", "kaAaAaaAaaAaaAaa", "kaAaaaaAaaaaaAaa", "kaaaaAaaaaaAaaaa", "kaaaaAaaaaaAaaaa", "kAaaaaaaAaaaaaAa", "kkkkkkkkkkkkkkkk"]
    lip_r = [r[::-1] for r in lip_l]
    body_l = ["kaaAaaaaAaaaaAaa", "kaaAaaaaAaaaaAaa", "kaaaaAaaaaAaaaaa", "kaaaaAaaaaAaaaaa", "kAaaaaaAaaaaaAaa", "kAaaaaaAaaaaaAaa", "kaaaAaaaaaAaaaaa", "kaaaAaaaaaAaaaaa", "kaaaaaAaaaaaAaaa", "kaAaaaAaaaaaAaaa", "kaAaaaaaaAaaaaaa", "kaaaaaaaaAaaaaAa", "kaaaAaaaaaaaaaAa", "kaaaAaaaaAaaaaaa", "kAaaaaaaaAaaaaaa", "kAaaaaAaaaaaaAaa"]
    body_r = [r[::-1] for r in body_l]
    side_top = [r for r in lip_l]
    return {
        "comb": anim([comb(0), comb(3), comb(7)], comb_pal, 5),
        "used": spr(used, used_pal),
        "hard": spr(grid(16, 16, hard), hard_pal),
        "log_tl": spr(lip_l, log_pal),
        "log_tr": spr(lip_r, log_pal),
        "log_bl": spr(body_l, log_pal),
        "log_br": spr(body_r, log_pal),
        "log_side": spr(side_top, log_pal),
    }


def world_tiles():
    out = {}
    for theme, t in THEMES.items():
        out.update(ground_tiles(theme, t))
    out.update(block_tiles())
    # Canopy: leafy treetop platforms on trunks.
    leaf_pal = {"g": "#6cd65c", "G": "#2f9a3a", "l": "#b8f59a", "t": "#8a5228", "T": "#5a3214"}

    def leaf(edge):
        def f(x, y):
            if y < 2 and ((edge == "l" and x < 3 - y) or (edge == "r" and x > 12 + y)):
                return "."
            if y == 0 or (edge == "l" and x == 0 and y < 14) or (edge == "r" and x == 15 and y < 14):
                return "G"
            if y >= 12:
                return "G" if y == 12 or speck(x, y, 2) < 40 else "."
            return "l" if speck(x, y, 9) < 12 else "g"

        return grid(16, 16, f)

    out["leaf_l"] = spr(leaf("l"), leaf_pal)
    out["leaf_m"] = spr(leaf("m"), leaf_pal)
    out["leaf_r"] = spr(leaf("r"), leaf_pal)
    out["trunk"] = spr(grid(16, 16, lambda x, y: "." if x < 4 or x > 11 else ("T" if x in (4, 11) or speck(x, y, 4) < 12 else "t")), leaf_pal)
    plank_pal = {"p": "#c8925a", "P": "#7a4a22", "k": "#3a1e0a"}
    out["lift"] = spr(["kkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk", "kppppppppppPpppppppppppPpppppppppppPpppppppppppk", "kpPppppppppPppppPppppppPppppppPppppPpppppppppPpk", "kpppppPppppPpppppppPpppPppppppppppPPpppppPppppk".ljust(47, "p") + "k", "kPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPk", "kkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk", "....kk..........................kk..........", "....kk..........................kk.........."][:6], plank_pal)
    # Castle: lava, bridge, lever, chain.
    lava_pal = {"o": "#ff5a1f", "O": "#c42a00", "y": "#ffb03a", "w": "#ffe9a8"}

    def lava(t):
        def f(x, y):
            wave = 2 + int(2 * (((x + t * 4) % 16) / 16.0 < 0.5))
            if y < wave:
                return "."
            if y == wave:
                return "y"
            if speck(x + t * 3, y, 11) < 10:
                return "w" if y < 6 else "y"
            return "O" if y > 10 else "o"

        return grid(16, 16, f)

    out["lava"] = anim([lava(0), lava(1), lava(2), lava(3)], lava_pal, 4)
    out["lava_deep"] = anim([grid(16, 16, lambda x, y, t=t: "y" if speck(x + t * 5, y + t * 3, 13) < 6 else ("O" if (x + y + t) % 7 == 0 else "o")) for t in range(4)], lava_pal, 4)
    bridge_pal = {"b": "#c8925a", "B": "#7a4a22", "k": "#2a1408", "c": "#9a9aa8"}
    out["bridge"] = spr(["kkkkkkkkkkkkkkkk", "bbbbbbbbbbbbbbbb", "bBbbbbbBbbbbbbBb", "bbbbBbbbbbBbbbbb", "BBBBBBBBBBBBBBBB", "kkkkkkkkkkkkkkkk", "..c.......c.....", "...c.....c......", "....c...c.......", ".....c.c........", "......c.........", "................", "................", "................", "................", "................"], bridge_pal)
    lever_pal = {"c": "#ffd23f", "C": "#c98e00", "k": "#2a1408", "w": "#ffffff", "r": "#e63946", "g": "#9a9aa8", "G": "#5a5a6a"}
    out["lever"] = anim(
        [
            ["................", "..........kk....", ".........kcck...", "........kccwck..", ".......kccck....", "......kcck......", ".....kcck.......", "....kcck........", "...kcck.........", "..GGGGGGGGGG....", ".GgggggggggG....", ".GgrrrrrrrgG....", ".GgggggggggG....", ".GGGGGGGGGGG....", "................", "................"],
            ["................", "..........kk....", ".........kcck...", "........kccCck..", ".......kccck....", "......kcck......", ".....kcck.......", "....kcck........", "...kcck.........", "..GGGGGGGGGG....", ".GgggggggggG....", ".GgrrwrrrrgG....", ".GgggggggggG....", ".GGGGGGGGGGG....", "................", "................"],
        ],
        lever_pal,
        3,
    )
    # The beacon pole: a slim pole, a ball on top, a hive banner.
    pole_pal = {"p": "#e8e8e8", "P": "#9a9a9a", "c": "#ffd23f", "C": "#c98e00", "g": "#3fae49", "y": "#ffc531", "k": "#2a1a12"}
    out["pole"] = spr(["......pP........"] * 16, pole_pal)
    out["pole_top"] = spr(["................", "................", "................", "................", "................", "................", "................", "................", "......cc........", ".....cCCc.......", ".....cCCc.......", "......cc........", "......pP........", "......pP........", "......pP........", "......pP........"], pole_pal)
    out["banner"] = anim(
        [
            ["yyyyyyyyyyyyy...", "ykkyyyyyyyyyyy..", "yyyykkkyyyyyyyy.", "ykkyyyyyyykkyyyy", "yyyykkkyyyyyyyy.", "ykkyyyyyyyyyyy..", "yyyyyyyyyyyyy...", "................"],
            ["yyyyyyyyyyyy....", "ykkyyyyyyyyyyyy.", "yyyykkkyyyyyyyyy", "ykkyyyyyyykkyyyy", "yyyykkkyyyyyyyy.", "ykkyyyyyyyyyyyy.", "yyyyyyyyyyyy....", "................"],
        ],
        pole_pal,
        4,
    )
    return out


def hut():
    """The Hive Hut at the end of a level: a straw skep with a round door, 48x48."""
    pal = {"s": "#e6b85c", "S": "#a8782a", "k": "#3a240a", "d": "#2a1408", "y": "#ffd23f", "g": "#3fae49", "G": "#1f6f2a"}
    rows = []
    for y in range(48):
        row = ""
        for x in range(48):
            cx = x - 23.5
            # dome: wider at the bottom, bands of straw every 6 px
            half = 6 + min(18, y * 0.75) if y < 40 else 22
            if y < 4 or abs(cx) > half:
                row += "."
                continue
            if y >= 44:
                row += "G" if (x + y) % 3 else "g"
                continue
            if abs(cx) > half - 1.2:
                row += "k"
                continue
            door = (y > 26 and abs(cx) < 6) and not (y < 30 and abs(cx) > 6 - (30 - y))
            if door:
                row += "d"
                continue
            if y % 6 == 0:
                row += "S"
            elif y % 6 == 3 and int(cx) % 5 == 0:
                row += "S"
            else:
                row += "s"
        rows.append(row)
    # a little honey sign above the door
    rows[20] = rows[20][:21] + "yyyyyy" + rows[20][27:]
    rows[21] = rows[21][:21] + "ykkkky" + rows[21][27:]
    rows[22] = rows[22][:21] + "yyyyyy" + rows[22][27:]
    return {"hut": spr(rows, pal)}


def decor():
    """Scenery: clouds, bushes, hills, flowers, castle torches. Drawn behind everything, touched by nothing."""
    cloud_pal = {"w": "#ffffff", "W": "#cfe8ff", "k": "#1d3a6a"}
    cloud = [
        "................................",
        "...........kkkk.................",
        ".........kkwwwwkk...............",
        "........kwwwwwwwwk..kkkk........",
        "......kkwwwwwwwwwwkkwwwwk.......",
        ".....kwwwwwwwwwwwwwwwwwwwk......",
        "...kkwwwwwwwwwwwwwwwwwwwwwkk....",
        "..kwwwwwwwwwwwwwwwwwwwwwwwwwk...",
        ".kwwwwwwwwwwwwwwwwwwwwwwwwwwwk..",
        ".kwWwwwwwwwwwwwwwwwwwwwwwwwWwk..",
        "..kWWWwwwwwWWwwwwwwwwWWWWWWWk...",
        "...kkkWWWWWkkWWWWWWWWkkkkkkk....",
        "......kkkkk..kkkkkkkk...........",
        "................................",
        "................................",
        "................................",
    ]
    bush_pal = {"g": "#5cd65c", "G": "#1f8a2a", "k": "#0f3a12", "l": "#a8f59a"}
    bush = [r.replace("w", "g").replace("W", "G") for r in cloud]
    bush = [r.replace("g", "l", 1) if i == 4 else r for i, r in enumerate(bush)]
    hill_pal = {"h": "#3fae49", "H": "#1f6f2a", "k": "#0f3a12", "d": "#2a7a32"}
    hill = []
    for y in range(32):
        row = ""
        for x in range(48):
            half = 6 + y * 0.6
            cx = abs(x - 23.5)
            if cx > half:
                row += "."
            elif cx > half - 1:
                row += "k"
            elif (x, y) in ((20, 12), (26, 16), (22, 20), (28, 22), (18, 24)):
                row += "d"
            else:
                row += "h" if y > 1 else "k"
        hill.append(row)
    flower_pal = {"g": "#3fae49", "y": "#ffe36b", "p": "#ff7ab8", "w": "#ffffff"}
    flowers = ["................"] * 10 + ["..p......y......", ".pwp....ywy..p..", "..p......y..pwp.", "..g..y...g...p..", "..g.ywy..g...g..", "..g..g...g...g.."]
    torch_pal = {"o": "#ff8c1a", "y": "#ffe36b", "k": "#3a3a46", "K": "#1a1a22"}
    torch = [
        ["................", "......o.........", ".....oyo........", ".....oyyo.......", "....oyyyo.......", ".....oyo........", "....kkkkk.......", ".....kkk........", "......k.........", "......k.........", "......K.........", "................", "................", "................", "................", "................"],
        ["................", ".......o........", "......oyo.......", ".....oyyo.......", ".....oyyyo......", "......oyo.......", "....kkkkk.......", ".....kkk........", "......k.........", "......k.........", "......K.........", "................", "................", "................", "................", "................"],
    ]
    crystal_pal = {"c": "#7af0ff", "C": "#2a9ab0", "w": "#ffffff"}
    crystal = ["................"] * 6 + ["......c.........", ".....cwc...c....", ".....cCc..cwc...", "..c..cCc..cCc...", ".cwc.cCc..cCc...", ".cCc.cCc..cCc...", ".cCc.cCc.cCCc...", "................", "................", "................"]
    dusk_cloud = recolor(spr(cloud, cloud_pal), {"w": "#d8b8e8", "W": "#a888c8", "k": "#3a2a5a"})
    star_pal = {"w": "#ffffff", "y": "#fff3a0"}
    stars = anim([["......w.........", "................", "..............y.", "................", "................", "...y............", "................", "..........w.....", "................", "................", ".w..............", "................", "........y.......", "................", "................", "................"], ["......y.........", "................", "..............w.", "................", "................", "...w............", "................", "..........y.....", "................", "................", ".y..............", "................", "........w.......", "................", "................", "................"]], star_pal, 2)
    variants = {
        "autumn": ({"w": "#fff0d8", "W": "#f0c890", "k": "#7a3a10"}, {"g": "#e0902c", "G": "#9a4a14", "k": "#4a1a04", "l": "#ffc060"}, {"h": "#c0702a", "H": "#7a3a10", "k": "#3a1404", "d": "#9a5018"}),
        "frost": ({"w": "#ffffff", "W": "#d0e8ff", "k": "#5a7aa0"}, {"g": "#e8f6ff", "G": "#9ac0e0", "k": "#4a6a8a", "l": "#ffffff"}, {"h": "#dff0ff", "H": "#9ac0e0", "k": "#4a6a8a", "d": "#b0d0ea"}),
        "dunes": ({"w": "#fff8e8", "W": "#f8d8a8", "k": "#a06a30"}, {"g": "#7ac04a", "G": "#3a7a20", "k": "#1a3a0a", "l": "#b0e080"}, {"h": "#e8c070", "H": "#a07a30", "k": "#5a3a10", "d": "#c09040"}),
        "night": ({"w": "#5a6aa8", "W": "#3a4a80", "k": "#1a2050"}, {"g": "#2f7a5a", "G": "#14402e", "k": "#06180e", "l": "#4fa07a"}, {"h": "#1f5a3a", "H": "#0f3020", "k": "#041008", "d": "#184a2e"}),
        "blossom": ({"w": "#ffffff", "W": "#ffd8ea", "k": "#c06a8a"}, {"g": "#ff9ac8", "G": "#c0508a", "k": "#5a1a3a", "l": "#ffd0e8"}, {"h": "#7ad07a", "H": "#3a8a3a", "k": "#1a3a1a", "d": "#ff9ac8"}),
        "ash": ({"w": "#8a7a7a", "W": "#5a4a4a", "k": "#2a1a1a"}, {"g": "#6a3a2a", "G": "#3a1a10", "k": "#1a0804", "l": "#a05a3a"}, {"h": "#5a4040", "H": "#3a2424", "k": "#140808", "d": "#ff5a1f"}),
    }
    extra = {}
    for name, (cp, bp, hp) in variants.items():
        extra[f"cloud_{name}"] = recolor(spr(cloud, cloud_pal), cp)
        extra[f"bush_{name}"] = recolor(spr(bush, bush_pal), bp)
        extra[f"hill_{name}"] = recolor(spr(hill, hill_pal), hp)
    return {
        **extra,
        "cloud": spr(cloud, cloud_pal),
        "dcloud": dusk_cloud,
        "bush": spr(bush, bush_pal),
        "hill": spr(hill, hill_pal),
        "flowers": spr(flowers, flower_pal),
        "torch": anim(torch, torch_pal, 6),
        "crystal": spr(crystal, crystal_pal),
        "stars": stars,
    }


def all_sprites():
    out = {}
    for part in (tobin_variants(), friends(), grubble(), snail(), wasp(), spikebug(), snapvine(), fire_things(), baron(), items(), world_tiles(), hut(), decor()):
        for k, v in part.items():
            assert k not in out, k
            out[k] = v
    return out
