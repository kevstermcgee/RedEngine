"""Worlds 3 to 8 of Honeybound, built from seeded segments (worlds 1 and 2 are hand-made in levels.py).

Every world has the classic shape: x-1 out in the open, x-2 underground, x-3 high up (treetops or bridges over nothing), x-4 a keep with the Baron.
A stage is a row of segments (blocks with a foe below, hollow logs, pits, foe groups, stair pyramids, floating platforms, wasps), each picked by a
seeded generator and harder as the worlds go on: wider pits, more and faster foes, snapvines, cinders, longer ember wheels. Nothing here is
trusted to be passable: `dev.py solve` finds a run through every stage with the real simulation, and verify replays it.
"""
import random

from levels import Level, T, ROWS, cx, cy, castle

# world -> (name, overworld theme, sky, underground theme, high stage kind, keep sky, decor variant, grubble kind)
WORLDS = {
    3: ("AUTUMN HOLLOW", "autumn", "#f6b26b", "roots", "trees", "#1a0c08", "autumn", "grubble"),
    4: ("FROSTVALE", "frost", "#bfe3ff", "caves", "bridges", "#0c1020", "frost", "dgrubble"),
    5: ("SUNSET DUNES", "dunes", "#ffc58a", "roots", "trees", "#200c04", "dunes", "grubble"),
    6: ("STARLIT MEADOW", "meadow", "#101848", "caves", "bridges", "#08081a", "night", "dgrubble"),
    7: ("BLOSSOM RIDGE", "blossom", "#ffe0ef", "roots", "trees", "#1a0814", "blossom", "grubble"),
    8: ("THE ASHLANDS", "ash", "#3a1a1a", "caves", "bridges", "#1c0606", "ash", "dgrubble"),
}
STAGE_NAMES = {
    3: ["AUTUMN HOLLOW", "ACORN CELLARS", "MAPLE CROWNS", "THE RUSTED KEEP"],
    4: ["FROSTVALE", "ICICLE GROTTO", "GLACIER SPANS", "THE FROZEN KEEP"],
    5: ["SUNSET DUNES", "SANDROOT TUNNELS", "PALM HEIGHTS", "THE SANDSTONE KEEP"],
    6: ["STARLIT MEADOW", "MOONLIT CAVES", "COMET BRIDGES", "THE MIDNIGHT KEEP"],
    7: ["BLOSSOM RIDGE", "PETAL BURROWS", "CHERRY CANOPY", "THE THORNBLOOM KEEP"],
    8: ["THE ASHLANDS", "CINDER DEPTHS", "SMOKE BRIDGES", "THE BARON'S CITADEL"],
}


def music_for(world, kind):
    return {"over": "meadow" if world % 2 else "dusk", "under": "roots", "high": "canopy", "keep": "keep"}[kind]


class Builder:
    """Places segments left to right; `c` is the next free column."""

    def __init__(self, L, rng, world):
        self.L, self.rng, self.w = L, rng, world
        self.c = 16
        self.mid_done = False

    def pick_foe(self):
        kinds = [WORLDS[self.w][7]] * 3 + ["snail"] * 2 + ["spike"] * max(0, self.w - 3)
        return self.rng.choice(kinds)

    def foes(self, c, row=12, n=None):
        n = n or self.rng.randint(1, 1 + self.w // 3)
        for k in range(n):
            self.L.foe(self.pick_foe(), c + k * 2, row)

    def maybe_mid(self):
        if not self.mid_done and self.c > self.L.w // 2:
            self.L.midpoint(self.c)
            self.mid_done = True

    # ---- open-air segments (ground on row 13)
    def blocks(self):
        L, c, r = self.L, self.c, self.rng
        n = r.randint(3, 5)
        row = "".join(r.choice("B?B") for _ in range(n))
        if r.random() < 0.45:
            k = r.randrange(n)
            row = row[:k] + "P" + row[k + 1:]
        L.text(c, 9, row)
        if r.random() < 0.4:
            L.text(c + 1, 5, "B" * (n - 2) + "?")
            L.coins(c + 1, 4, n - 2)
        self.foes(c + n // 2)
        self.c += n + 3

    def log(self):
        h = self.rng.randint(2, 4)
        vine = self.rng.random() < 0.12 * (self.w - 2)
        self.L.log(self.c, h, vine=vine, hint=False)
        if self.rng.random() < 0.5:
            self.foes(self.c + 4, n=1)
        self.c += 7

    def pit(self, under=False):
        width = self.rng.randint(2, min(5, 2 + self.w // 2))
        self.L.gap(self.c, self.c + width - 1)
        self.L.hints.pop()
        if self.rng.random() < 0.3 + 0.05 * self.w and not under:
            self.L.wasp(self.c + width // 2, 9)
        self.c += width + 3

    def group(self):
        self.foes(self.c, n=self.rng.randint(2, 2 + self.w // 3))
        self.L.coins(self.c, 8, 4)
        self.c += 9

    def pyramid(self):
        h = self.rng.randint(3, 4)
        gap = self.rng.choice([0, 0, 2]) if self.w < 6 else self.rng.choice([0, 2, 3])
        self.L.stairs(self.c, h, hint=False)
        self.L.column(self.c + h, h)
        if gap:
            self.L.gap(self.c + h + 1, self.c + h + gap)
            self.L.hints.pop()
        self.L.stairs(self.c + h + 1 + gap, h, up=False, hint=False)
        self.c += 2 * h + gap + 4

    def platforms(self):
        L, c, r = self.L, self.c, self.rng
        n = r.randint(4, 7)
        L.text(c, 9, "B" * n)
        L.text(c + 2, 5, "B" * (n - 1) + "?")
        L.coins(c + 2, 4, n - 2)
        L.foe(WORLDS[self.w][7], c + 3, 4)
        self.c += n + 5

    def run(self, segs, end):
        while self.c < end:
            r = self.rng.random()
            acc = 0
            for weight, f in segs:
                acc += weight
                if r < acc:
                    f()
                    break
            self.c += self.rng.randint(1, 3)
            self.maybe_mid()


def overworld(world, n):
    name, theme, sky, *_ , decor, _g = WORLDS[world]
    W = 210
    L = Level(f"{world}-{n}", STAGE_NAMES[world][n - 1], W, theme, sky, music_for(world, "over"), decor_cloud=f"cloud_{decor}")
    L.decor_set = decor
    rng = random.Random(world * 100 + n)
    if decor in ("night", "ash"):
        for c in range(3, W - 2, 9):
            L.add("stars", cx(c), cy(1 + c % 5), None)
    L.decor_band(every=16, flowers=decor not in ("frost", "ash"), seed=world)
    L.ground(0, W - 1)
    b = Builder(L, rng, world)
    b.run([(0.24, b.blocks), (0.18, b.log), (0.2, b.pit), (0.14, b.group), (0.12, b.pyramid), (0.12, b.platforms)], W - 42)
    end = b.c + 2
    L.stairs(end, 8, hint=False)
    L.column(end + 8, 8)
    L.pole(end + 17)
    return L


def underground(world, n):
    name, theme, sky, under, *_ = WORLDS[world]
    W = 200
    L = Level(f"{world}-{n}", STAGE_NAMES[world][n - 1], W, under, "#07131a" if under == "caves" else "#0b0c1a", music_for(world, "under"))
    rng = random.Random(world * 100 + n)
    L.ground(0, W - 1)
    L.fill(6, W - 34, 2, 2, "B")
    for c in range(10, W - 30, 15):
        L.add("crystal", cx(c), cy(12), None)
    b = Builder(L, rng, world)

    def wall():
        h = rng.randint(2, 4)
        L.fill(b.c, b.c + 1, 13 - h, 12, "B")
        L.coins(b.c, 12 - h, 2)
        b.c += 5

    def tunnel():
        # a low ceiling: bricks hang down to row 8, leaving a 4-tile passage with honey and a foe
        n_ = rng.randint(6, 10)
        L.fill(b.c, b.c + n_, 3, 8, "B")
        L.coins(b.c + 1, 11, n_ - 1)
        b.foes(b.c + n_ // 2, n=1)
        b.c += n_ + 3

    def lift_pit():
        width = rng.randint(5, 6)
        L.gap(b.c, b.c + width - 1)
        L.hints.pop()
        L.lift(b.c + 1, 10, rng.choice("xy"), 24 if width == 5 else 32, 30 + 2 * world)
        b.c += width + 3

    def vine_log():
        L.log(b.c, rng.randint(2, 3), vine=True, hint=False)
        b.c += 6

    b.run([(0.2, b.blocks), (0.15, vine_log), (0.12, b.pit), (0.15, b.group), (0.12, wall), (0.14, tunnel), (0.12, lift_pit)], W - 44)
    end = b.c + 2
    L.stairs(end, 6, hint=False)
    L.column(end + 6, 6)
    L.pole(end + 15)
    return L


def high(world, n):
    name, theme, sky, under, kind, *_ = WORLDS[world]
    W = 196
    decor = WORLDS[world][6]
    L = Level(f"{world}-{n}", STAGE_NAMES[world][n - 1], W, theme, sky, music_for(world, "high"), decor_cloud=f"cloud_{decor}")
    rng = random.Random(world * 100 + n)
    for c in range(4, W, 13):
        L.add(f"cloud_{decor}", cx(c), cy(2 + c % 4), None)
    if decor in ("night", "ash"):
        for c in range(3, W - 2, 9):
            L.add("stars", cx(c), cy(1 + c % 5), None)
    L.ground(0, 13)
    c = 15
    top = 11
    while c < W - 36:
        roll = rng.random()
        gap = rng.randint(2, min(4, 2 + world // 3))
        if roll < 0.18:
            # a lift over a wide gap
            span = 6
            L.lift(c + 1, max(top - 1, 6), rng.choice("xy"), 32, 30 + 2 * world)
            c += span + 1
            continue
        width = rng.randint(3, 8)
        top = max(5, min(11, top + rng.choice([-2, -1, 0, 1, 2])))
        if kind == "trees":
            L.set(c, top, "(")
            for k in range(c + 1, c + width - 1):
                L.set(k, top, "-")
            L.set(c + width - 1, top, ")")
            for r in range(top + 1, ROWS):
                L.set(c + width // 2, r, "|")
        else:
            for k in range(c, c + width):
                L.set(k, top, "X")
        if width >= 5 and rng.random() < 0.6:
            L.foe(rng.choice([WORLDS[world][7], "snail"] + (["spike"] if world > 4 else [])), c + width // 2, top - 1)
        if rng.random() < 0.5:
            L.coins(c + 1, top - 2, width - 2)
        if rng.random() < 0.25 + 0.05 * world:
            L.wasp(c + width + gap // 2, top - 2)
        if world >= 5 and rng.random() < 0.25:
            L.add("cinder_hi", cx(c + width + gap // 2), cy(14), None)
        c += width + gap
        if c > W // 2 and not any(p == "midpoint" for p, *_ in L.scene):
            L.midpoint(c - gap - 2)
    L.ground(c, W - 1)
    L.stairs(c + 3, 3, hint=False)
    L.pole(c + 10)
    return L


def keep(world, n, friend, hp):
    W = 150 + 4 * world
    L = Level(f"{world}-{n}", STAGE_NAMES[world][n - 1], W, "castle", WORLDS[world][5], "keep")
    rng = random.Random(world * 100 + n)
    L.fill(0, W - 1, 0, 2, "W")
    L.fill(0, 12, 9, 14, "W")
    L.start = (2, 8)
    for c in range(5, W - 20, 10):
        L.add("torch", cx(c), cy(4), None)
    c = 13
    boss = W - 30
    while c < boss - 22:
        pit = rng.randint(3, min(6, 3 + world // 3))
        L.fill(c, c + pit - 1, 12, 14, "~")
        for k in range(rng.randint(1, 1 + world // 4)):
            L.add("cinder", cx(c + 1 + (k * 2) % pit), cy(13), None)
        c += pit
        length = rng.randint(12, 20)
        L.fill(c, c + length - 1, 9, 14, "W")
        # Ember wheels: at most two per stretch, 8+ tiles apart, never under a low ceiling (that closes the corridor), chains of 4-5 embers.
        slab = rng.random() < 0.35
        if slab:
            L.fill(c + 5, c + length - 4, 3, 5, "W")
        else:
            bars = 1 if length < 18 or world < 5 else 2
            for k in range(bars):
                bc = c + 4 + k * 9
                hanging = rng.random() < 0.4
                row = 5 if hanging else 8
                if hanging:
                    L.fill(bc, bc, 3, 4, "W")
                L.firebar(bc, row, n=min(5, 3 + world // 3), speed=rng.choice([-1, 1]) * (80 + 4 * world), phase=rng.choice([0, 90, 180, 270]))
        if rng.random() < 0.3:
            L.set(c + length // 2, 5, "P")
        c += length
        if c > W // 2 and not any(p == "midpoint" for p, *_ in L.scene):
            L.midpoint(c - 3)
    L.fill(c, boss - 10, 9, 14, "W")
    castle(L, boss_col=boss, lever_col=boss + 7, friend=friend, length=W, hp=hp)
    return L


def world_stages(world):
    friend = "queen" if world == 8 else f"friend{world}"
    return [overworld(world, 1), underground(world, 2), high(world, 3), keep(world, 4, friend, hp=5 + world)]
