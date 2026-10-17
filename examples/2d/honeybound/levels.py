"""Honeybound's levels, built with a few verbs (ground, gap, log, stairs, blocks, foes) on a 15-row tile grid (16 px tiles; the 480x270 view shows 30
columns, and `build.py` puts the grid 30 px down so the HUD has a band of sky above it).

Row 13 is the top of the ground (Pip stands on it), row 14 the soil under it; row 15 (below the screen) is the abyss that catches falls. Each verb
can leave a `hint` for the scripted run that proves the level can be finished (`build.py` turns hints into a scenario): where to jump, how
long to hold the button, where to wait for a lift.
"""

T = 16
ROWS = 15


def cx(col):
    return col * T + 8


def cy(row):
    return row * T + 8


class Level:
    def __init__(self, lid, name, width, theme, background, music, decor_cloud="cloud"):
        self.id, self.name, self.w, self.theme, self.background, self.music = lid, name, width, theme, background, music
        self.grid = [["."] * width for _ in range(ROWS)]
        self.scene = []  # (prefab, x, y, id or None)
        self.hints = []  # (x px, kind, args)
        self.cloud = decor_cloud
        self.start = (3, 12)
        self.n = 0

    # ---- placing -----------------------------------------------------------------------------------------------------------------------------
    def set(self, col, row, ch):
        if 0 <= col < self.w and 0 <= row < ROWS:
            self.grid[row][col] = ch

    def text(self, col, row, s):
        for i, ch in enumerate(s):
            if ch != " ":
                self.set(col + i, row, ch)

    def add(self, prefab, x, y, sid=None):
        self.scene.append((prefab, x, y, sid))

    def uid(self, base):
        self.n += 1
        return f"{base}{self.n}"

    def hint(self, col, kind="jump", **args):
        self.hints.append((col * T, kind, args))

    # ---- verbs -------------------------------------------------------------------------------------------------------------------------------
    def ground(self, c0, c1, top=13):
        for c in range(c0, c1 + 1):
            self.set(c, top, "=")
            for r in range(top + 1, ROWS):
                self.set(c, r, "#")

    def fill(self, c0, c1, r0, r1, ch):
        for c in range(c0, c1 + 1):
            for r in range(r0, r1 + 1):
                self.set(c, r, ch)

    def gap(self, c0, c1, hold=0.35):
        """A pit: the ground between c0 and c1 removed; the run jumps a little before it."""
        for c in range(c0, c1 + 1):
            for r in range(0, ROWS):
                if self.grid[r][c] in "=#":
                    self.set(c, r, ".")
        self.hint(c0 - 2, hold=hold)

    def log(self, col, h, base=13, vine=False, enter=None, hint=True, hold=None):
        """An upright hollow log `h` tiles tall standing on row `base`. `enter` makes its mouth a way down (a bonus room)."""
        top = base - h
        self.set(col, top, "[")
        self.set(col + 1, top, "]")
        for r in range(top + 1, base):
            self.set(col, r, "{")
            self.set(col + 1, r, "}")
        if vine:
            self.add("snapvine", col * T + 16, top * T, None)
        if enter:
            self.add("logmouth", col * T + 16, top * T - 4, None)
        if hint:
            self.hint(col - (3 if h >= 3 else 2), hold=hold if hold else (0.45 if h >= 3 else 0.3))

    def stairs(self, col, h, up=True, base=13, hint=True):
        """A staircase of hard blocks, h steps (up = rising to the right)."""
        for i in range(h):
            height = i + 1 if up else h - i
            for r in range(base - height, base):
                self.set(col + i, r, "X")
        if hint and up:
            self.hint(col - 1, hold=0.25)

    def column(self, col, h, base=13):
        for r in range(base - h, base):
            self.set(col, r, "X")

    def foe(self, kind, col, row=12):
        self.set(col, row, {"grubble": "g", "snail": "k", "spike": "s", "dgrubble": "d"}[kind])

    def wasp(self, col, row, axis="y"):
        self.add("wasp" if axis == "y" else "waspx", cx(col), cy(row), None)

    def coins(self, col, row, n, step=1):
        for i in range(n):
            self.set(col + i * step, row, "o")

    def pole(self, col, base=13):
        """The Bloom Gate at the end of a stage: a great flower bud on a tall stem; touching it opens it and clears the stage."""
        self.add("bloom", col * T + 8, base * T - 32, "bloom")
        # the ground the bloom stands on runs on past it, so Pip has somewhere to walk off to
        self.ground(col - 2, min(self.w - 1, col + 12), top=base)

    def folk(self, col, prefab):
        """A bug of the road who says a line when Pip bumps into them (`build.py` picks the line by world)."""
        self.add(prefab, cx(col), 13 * T - 8, None)

    def decor_band(self, every=16, hills=True, bushes=True, clouds=True, flowers=False, seed=0):
        """Ground scenery: tufts of grass, mushrooms, pebbles and daisies (the hills and clouds are the parallax backdrop now). Placed by `place_decor`
        once the level is finished, so nothing stands over a pit dug after this call."""
        self.decor_args = dict(every=every, bushes=bushes, flowers=flowers, seed=seed)

    def place_decor(self):
        if not hasattr(self, "decor_args"):
            return
        a = self.decor_args
        every, bushes, flowers, seed = a["every"], a["bushes"], a["flowers"], a["seed"]
        v = getattr(self, "decor_set", None)
        v = {"ending": "frost"}.get(v, v)
        tuft = f"tuft_{v}" if v in ("autumn", "frost", "night", "blossom", "ash", "dunes", "dusk") else "tuft"
        for c in range(2 + seed % 5, self.w - 4, max(5, every // 3)):
            k = ((c * 2654435761 + seed * 40503) >> 9) % 7  # a stable shuffle: tufts, daisies, a mushroom now and then, pebbles, bare ground
            if self.grid[13][c] != "=" or self.grid[12][c] != ".":
                continue
            if k in (0, 2, 5) and bushes:
                self.add(tuft, cx(c), 13 * T - 6, None)
            elif k == 1 and flowers:
                self.add("flowers", cx(c), 13 * T - 6, None)
            elif k == 3 and v not in ("frost", "ash"):
                self.add("mushroom", cx(c), 13 * T - 8, None)
            elif k == 4:
                self.add("pebbles", cx(c), 13 * T - 4, None)

    def midpoint(self, col):
        self.add("midpoint", cx(col), cy(10), None)

    def lift(self, col, row, axis, rng, speed, sid=None):
        """A moving plank (48 px) centred on col+1, going `rng` px each way along `axis` (`build.py` makes one prefab per kind of lift)."""
        sid = sid or self.uid("lift")
        self.add(f"lift_{axis}_{rng}_{speed}", col * T + 24, row * T + 3, sid)
        return sid

    def firebar(self, col, row, n=5, speed=100, phase=0):
        """A used block with a chain of `n` embers turning around it (one orbit prefab per radius, speed and phase)."""
        self.set(col, row, "U")
        for k in range(1, n + 1):
            self.add(f"orb_{k * 8}_{speed}_{phase}", cx(col), cy(row), None)

    def shift(self, n, ch):
        """Moves everything n columns right, filling the columns that open on the left with `ch` (a narrow room centred on the screen)."""
        self.grid = [[ch] * n + row[: self.w - n] for row in self.grid]
        self.scene = [(p, x + n * T, y, sid) for (p, x, y, sid) in self.scene]
        self.start = (self.start[0] + n, self.start[1])

    def rows(self):
        # lava under lava is the deep part (no wavy surface)
        for r in range(ROWS - 1, 0, -1):
            for c in range(self.w):
                if self.grid[r][c] == "~" and self.grid[r - 1][c] in "~%":
                    self.grid[r][c] = "%"
        out = ["".join(r) for r in self.grid]
        out.append("A" * self.w)  # the abyss: one hidden row below the screen
        return out


# ---- the stages ---------------------------------------------------------------------------------------------------------------------------------


def l11():
    L = Level("1-1", "MEADOW PATH", 212, "meadow", "#6bb4ff", "meadow")
    L.decor_band(every=16, flowers=True)
    L.ground(0, 211)
    L.folk(8, "bramwell")
    L.set(16, 9, "?")
    L.text(20, 9, "B?BPB")
    L.set(22, 5, "?")
    L.foe("grubble", 23)
    L.hint(19, hold=0.2)
    L.log(28, 2)
    L.log(38, 3)
    L.foe("grubble", 41)
    L.log(46, 4, enter="1-1b")
    L.foe("grubble", 51)
    L.foe("grubble", 53)
    L.hint(50, hold=0.4)
    L.log(57, 4)
    L.text(63, 9, "BPB")  # a second power comb: a firebloom once Tobin is big
    L.set(66, 5, "L")  # a clover up high
    L.gap(69, 70)
    L.text(77, 9, "B?B")
    L.text(80, 5, "BBBBBBBB")
    L.foe("grubble", 81, 4)
    L.foe("grubble", 83, 4)
    L.gap(86, 88)
    L.text(91, 5, "BBB?")
    L.set(94, 9, "R")
    L.text(100, 9, "BB")
    L.foe("snail", 103)
    L.hint(101, hold=0.3)
    L.midpoint(100)
    L.set(106, 9, "?")
    L.set(109, 9, "?")
    L.set(109, 5, "P")
    L.set(112, 9, "?")
    L.foe("grubble", 113)
    L.foe("grubble", 115)
    L.hint(111, hold=0.45)
    L.set(118, 9, "B")
    L.text(121, 5, "BBB")
    L.foe("grubble", 123)
    L.foe("grubble", 125)
    L.hint(121, hold=0.45)
    L.text(128, 5, "B??B")
    L.text(129, 9, "BB")
    L.foe("grubble", 130)
    L.foe("grubble", 132)
    L.hint(128, hold=0.45)
    L.stairs(136, 4)
    L.stairs(141, 4, up=False, hint=False)
    L.stairs(150, 4)
    L.column(154, 4)
    L.gap(155, 156, hold=0.3)
    L.hints.pop()  # the jump from the top of the stairs
    L.stairs(157, 4, up=False, hint=False)
    L.log(163, 2)
    L.text(168, 9, "BB?B")
    L.foe("grubble", 172)
    L.foe("grubble", 174)
    L.hint(169, hold=0.45)
    L.log(179, 2)
    L.stairs(181, 8)
    L.column(189, 8)
    L.hint(188, hold=0.6)
    L.pole(197)
    L.exit_col = 163  # where the bonus room's log brings Tobin back out
    return L


def b11():
    L = Level("1-1b", "", 30, "roots", "#0b0c1a", "roots")
    L.fill(16, 29, 0, 14, "W")  # the honey room is half the screen: the rest is earth
    L.fill(0, 0, 0, 14, "W")
    L.fill(0, 15, 13, 14, "W")
    L.fill(4, 11, 0, 0, "W")
    L.fill(13, 15, 0, 10, "W")
    for r in (6, 8, 10):
        L.coins(4, r, 8)
    L.coins(5, 4, 6)
    L.fill(3, 12, 11, 12, "W")
    L.set(12, 11, ".")
    L.set(12, 12, ".")
    L.add("logside", 13 * T + 8, 12 * T, None)
    L.start = (2, 2)
    L.shift(7, "W")
    return L


def l12():
    L = Level("1-2", "ROOTWAYS", 200, "roots", "#0b0c1a", "roots")
    L.ground(0, 199)
    L.fill(6, 175, 2, 2, "B")
    L.text(10, 9, "?????")
    L.foe("grubble", 16)
    L.foe("grubble", 18)
    L.hint(13, hold=0.4)
    L.column(28, 1)
    L.column(30, 2)
    L.column(32, 3)
    L.column(34, 3)
    L.hint(27, hold=0.25)
    L.hint(29, hold=0.3)
    L.hint(31, hold=0.35)
    L.text(40, 9, "BBPBBBBBBBB")
    L.coins(40, 8, 11)
    L.foe("snail", 46)
    L.foe("grubble", 50)
    L.hint(43, hold=0.5)
    L.foe("grubble", 56)
    L.hint(53, hold=0.4)
    L.gap(60, 62)
    L.fill(66, 76, 7, 8, "B")
    L.fill(67, 75, 10, 10, "B")
    L.coins(68, 9, 7)
    L.set(71, 7, "?")
    L.foe("grubble", 72, 12)
    L.hint(69, hold=0.3)
    L.log(82, 3, vine=True)
    L.log(90, 4, vine=True)
    L.midpoint(94)
    L.log(98, 2, vine=True)
    L.foe("snail", 104)
    L.hint(102, hold=0.35)
    L.gap(110, 115)
    L.hints.pop()
    lift = L.lift(111, 10, "y", 40, 30)
    L.hint(108, kind="wait_lift", lift=lift, near=[111 * T + 24, 10 * T + 3], tol=6)
    L.hint(108, hold=0.3)
    L.text(122, 9, "B?B?B")
    L.foe("grubble", 126)
    L.foe("grubble", 128)
    L.hint(124, hold=0.45)
    L.gap(134, 139)
    L.hints.pop()
    lift2 = L.lift(135, 11, "x", 24, 30)
    L.hint(132, kind="wait_lift", lift=lift2, near=[135 * T + 24 - 24, 11 * T + 3], tol=6)
    L.hint(132, hold=0.25)
    L.hint(138, hold=0.3)
    L.foe("snail", 148)
    L.hint(145, hold=0.4)
    L.stairs(156, 6)
    L.column(162, 6)
    L.hint(161, hold=0.5)
    L.pole(176)
    L.coins(150, 8, 4)
    for c in range(20, 170, 24):
        L.add("crystal", cx(c), cy(12), None)
    return L


def l13():
    L = Level("1-3", "CANOPY", 184, "meadow", "#8fd3ff", "canopy")
    for c in range(4, 180, 13):
        L.add("cloud", cx(c), cy(2 + c % 4), None)
    L.ground(0, 14)
    L.ground(166, 183)
    L.folk(9, "ladybug")

    def tree(c0, width, top, hold=None, foe=None, coins=0):
        L.set(c0, top, "(")
        for c in range(c0 + 1, c0 + width - 1):
            L.set(c, top, "-")
        L.set(c0 + width - 1, top, ")")
        mid = c0 + width // 2
        for r in range(top + 1, ROWS):
            L.set(mid, r, "|")
        if hold:
            L.hint(c0 - 2, hold=hold)
        if foe:
            L.foe(foe, mid, top - 1)
        if coins:
            L.coins(c0 + 1, top - 2, coins)

    tree(17, 5, 11, hold=0.3)
    tree(24, 4, 9, hold=0.4, coins=2)
    tree(30, 7, 7, hold=0.45, foe="grubble")
    tree(40, 3, 10, hold=0.25)
    L.wasp(44, 8)
    tree(46, 8, 9, hold=0.35, coins=4)
    L.set(49, 5, "P")
    tree(57, 4, 11, hold=0.2)
    lift = L.lift(62, 10, "x", 32, 36)
    L.hint(59, kind="wait_lift", lift=lift, near=[62 * T + 24 - 32, 10 * T + 3], tol=6)
    L.hint(59, hold=0.2)
    L.hint(66, hold=0.35)
    tree(68, 6, 8, foe="snail")
    tree(77, 3, 6, hold=0.45)
    tree(82, 5, 9, hold=0.25, coins=3)
    L.midpoint(84)
    L.wasp(89, 7)
    tree(91, 4, 10, hold=0.25)
    tree(97, 6, 8, hold=0.4, foe="grubble")
    tree(106, 3, 11, hold=0.2)
    lift2 = L.lift(110, 9, "y", 40, 30)
    L.hint(108, kind="wait_lift", lift=lift2, near=[110 * T + 24, 9 * T + 3 + 40], tol=6)
    L.hint(108, hold=0.25)
    L.hint(112, hold=0.45)
    tree(115, 5, 7, coins=3)
    tree(123, 4, 9, hold=0.3)
    L.wasp(128, 7)
    tree(130, 7, 10, hold=0.3, foe="snail")
    L.set(133, 6, "?")
    tree(140, 4, 8, hold=0.4)
    tree(147, 3, 6, hold=0.45)
    tree(153, 6, 9, hold=0.25, coins=4)
    L.hint(160, hold=0.35)
    L.stairs(168, 4)
    L.pole(176)
    return L


def castle(L, boss_col, lever_col, friend, length, hp):
    """The end of a keep: a corridor, then a drop into the boss's den (one screen, 30 columns) whose bramble gate shuts behind Pip.
    `build.py` puts the boss (`L.boss`, by world) and its prisoner in a thorn cocoon; markers `den_l`, `den_c`, `den_r` give the rules the den's x."""
    d0 = length - 30
    L.fill(boss_col - 12, d0 - 1, 9, 14, "W")
    L.set(d0 - 6, 5, "P")  # one last comb before the den: a grown Pip can take a hit
    L.fill(d0, length - 1, 3, 11, ".")
    L.fill(d0, length - 1, 12, 14, "W")
    L.fill(length - 1, length - 1, 3, 11, "W")
    L.den = d0
    L.add("densensor", cx(d0 + 4), cy(7), None)
    L.add("mark", cx(d0), cy(7), "den_l")
    L.add("mark", d0 * T + 15 * T, cy(7), "den_c")
    L.add("mark", cx(length - 2), cy(7), "den_r")
    L.friend = friend
    L.boss_hp = hp
    L.hint(boss_col - 14, hold=0.5)


def l14():
    L = Level("1-4", "THORNKEEP", 160, "keep", "#140a12", "keep")
    L.fill(0, 159, 0, 2, "W")
    L.fill(0, 14, 9, 14, "W")
    L.start = (2, 8)
    for c in range(6, 150, 12):
        L.add("torch", cx(c), cy(4), None)
    L.fill(15, 18, 12, 12, "~")
    L.fill(15, 18, 13, 14, "~")
    L.add("cinder", cx(17), cy(13), None)
    L.fill(19, 40, 9, 14, "W")
    L.hint(13, hold=0.4)
    L.firebar(26, 9 - 4, n=5, speed=110)
    L.fill(26, 26, 3, 4, "W")
    L.fill(41, 46, 12, 14, "~")
    L.add("cinder", cx(43), cy(13), None)
    L.fill(47, 60, 9, 14, "W")
    L.hint(38, hold=0.45)
    L.set(52, 5, "P")
    L.firebar(57, 8, n=5, speed=-100, phase=90)
    L.fill(61, 64, 12, 14, "~")
    L.fill(65, 68, 11, 14, "W")
    L.fill(69, 72, 12, 14, "~")
    L.add("cinder", cx(70), cy(13), None)
    L.fill(73, 92, 9, 14, "W")
    L.hint(59, hold=0.35)
    L.hint(67, hold=0.35)
    L.midpoint(76)
    L.fill(80, 92, 3, 5, "W")
    L.firebar(84, 8, n=4, speed=120)
    L.fill(93, 96, 12, 14, "~")
    L.add("cinder", cx(94), cy(13), None)
    L.fill(97, 118, 9, 14, "W")
    L.hint(91, hold=0.4)
    L.firebar(104, 9 - 4, n=6, speed=-90)
    L.firebar(112, 8, n=5, speed=90, phase=180)
    L.fill(104, 104, 3, 4, "W")
    castle(L, boss_col=130, lever_col=137, friend="friend1", length=160, hp=5)
    return L


def l21():
    L = Level("2-1", "DUSKWOOD", 212, "dusk", "#3b2a5c", "dusk", decor_cloud="dcloud")
    for c in range(3, 210, 9):
        L.add("stars", cx(c), cy(1 + c % 5), None)
    L.decor_band(every=18, clouds=True, flowers=False, seed=3)
    L.ground(0, 211)
    L.folk(10, "cricket")
    L.text(14, 9, "?B?")
    L.foe("dgrubble", 22)
    L.foe("dgrubble", 24)
    L.hint(19, hold=0.45)
    L.log(30, 3, vine=True)
    L.gap(36, 38)
    L.foe("snail", 45)
    L.text(42, 9, "BPB")
    L.hint(42, hold=0.35)
    L.wasp(52, 9)
    L.log(56, 4, vine=True)
    L.foe("spike", 64)
    L.hint(61, hold=0.45)
    L.gap(70, 73)
    L.text(76, 8, "B?BBB")
    L.foe("dgrubble", 79, 7)
    L.foe("dgrubble", 81)
    L.hint(78, hold=0.45)
    L.log(88, 4, enter="2-1b")
    L.foe("spike", 95)
    L.foe("spike", 97)
    L.hint(93, hold=0.5)
    L.midpoint(100)
    L.stairs(104, 3)
    L.gap(107, 109, hold=0.3)
    L.hints.pop()
    L.stairs(110, 3, up=False, hint=False)
    L.hint(106, hold=0.3)
    L.wasp(116, 8)
    L.wasp(120, 9)
    L.text(118, 5, "?R?")
    L.foe("snail", 126)
    L.hint(123, hold=0.4)
    L.log(132, 2, vine=True)
    L.log(140, 3, vine=True)
    L.foe("dgrubble", 146)
    L.foe("dgrubble", 148)
    L.foe("dgrubble", 150)
    L.hint(144, hold=0.5)
    L.gap(156, 159)
    L.log(164, 2)
    L.foe("spike", 170)
    L.hint(167, hold=0.45)
    L.stairs(180, 8)
    L.column(188, 8)
    L.hint(187, hold=0.6)
    L.pole(197)
    L.exit_col = 164
    return L


def b21():
    L = b11()
    L.id = "2-1b"
    L.theme = "caves"
    L.background = "#07131a"
    return L


def l22():
    L = Level("2-2", "GLIMMER CAVES", 200, "caves", "#07131a", "roots")
    L.ground(0, 199)
    L.fill(4, 180, 0, 2, "B")
    for c in range(8, 180, 14):
        L.add("crystal", cx(c), cy(12), None)
    L.text(12, 9, "B?B?B")
    L.foe("spike", 20)
    L.hint(17, hold=0.45)
    L.log(26, 3, vine=True)
    L.log(34, 2, vine=True)
    L.fill(40, 52, 9, 9, "B")
    L.fill(40, 40, 6, 8, "B")
    L.coins(42, 8, 9)
    L.set(46, 9, "P")
    L.foe("dgrubble", 44)
    L.foe("dgrubble", 48)
    L.hint(41, hold=0.5)
    L.gap(56, 59)
    L.foe("snail", 64)
    L.hint(61, hold=0.4)
    L.log(70, 4, vine=True)
    L.gap(76, 81)
    L.hints.pop()
    lift = L.lift(77, 11, "x", 32, 40)
    L.hint(73, kind="wait_lift", lift=lift, near=[77 * T + 24 - 32, 11 * T + 3], tol=6)
    L.hint(73, hold=0.2)
    L.hint(80, hold=0.3)
    L.midpoint(88)
    L.foe("spike", 90)
    L.foe("spike", 93)
    L.hint(87, hold=0.5)
    L.fill(98, 112, 7, 7, "B")
    L.fill(98, 112, 10, 10, "B")
    L.set(105, 10, "?")
    L.foe("dgrubble", 104)
    L.foe("dgrubble", 108)
    L.hint(100, hold=0.25)
    L.log(118, 3, vine=True)
    L.gap(124, 127)
    L.log(130, 4, vine=True)
    L.foe("snail", 138)
    L.hint(136, hold=0.4)
    L.gap(144, 150)
    L.hints.pop()
    lift2 = L.lift(146, 10, "y", 36, 34)
    L.hint(141, kind="wait_lift", lift=lift2, near=[146 * T + 24, 10 * T + 3 + 36], tol=6)
    L.hint(141, hold=0.3)
    L.hint(149, hold=0.45)
    L.stairs(156, 6)
    L.column(162, 6)
    L.hint(161, hold=0.5)
    L.pole(176)
    return L


def l23():
    L = Level("2-3", "HIGHBRIDGE", 192, "dusk", "#3b2a5c", "canopy", decor_cloud="dcloud")
    for c in range(3, 190, 9):
        L.add("stars", cx(c), cy(1 + c % 5), None)
    for c in range(6, 190, 15):
        L.add("dcloud", cx(c), cy(3 + c % 3), None)
    L.ground(0, 12)
    L.ground(176, 191)

    def bridge(c0, c1, row, foe=None, coins=False):
        for c in range(c0, c1 + 1):
            L.set(c, row, "X")
        if foe:
            L.foe(foe, (c0 + c1) // 2, row - 1)
        if coins:
            L.coins(c0 + 1, row - 2, c1 - c0 - 1)

    bridge(15, 24, 10, foe="dgrubble")
    L.hint(12, hold=0.35)
    L.hint(21, hold=0.45)
    L.wasp(27, 8)
    bridge(29, 36, 9, coins=True)
    L.hint(25, hold=0.3)
    bridge(40, 52, 11, foe="snail")
    L.hint(36, hold=0.25)
    L.hint(46, hold=0.45)
    L.add("cinder_hi", cx(55), cy(14), None)
    bridge(57, 64, 9)
    L.hint(52, hold=0.4)
    L.set(60, 5, "P")
    lift = L.lift(67, 9, "x", 40, 40)
    L.hint(63, kind="wait_lift", lift=lift, near=[67 * T + 24 - 40, 9 * T + 3], tol=6)
    L.hint(63, hold=0.2)
    L.hint(71, hold=0.3)
    bridge(75, 84, 10, foe="spike")
    L.hint(78, hold=0.5)
    L.midpoint(80)
    L.wasp(87, 7)
    L.wasp(91, 9)
    bridge(88, 96, 8, coins=True)
    L.hint(84, hold=0.45)
    L.add("cinder_hi", cx(99), cy(14), None)
    bridge(101, 108, 11)
    L.hint(96, hold=0.3)
    lift2 = L.lift(111, 9, "y", 40, 32)
    L.hint(108, kind="wait_lift", lift=lift2, near=[111 * T + 24, 9 * T + 3 + 40], tol=6)
    L.hint(108, hold=0.2)
    L.hint(113, hold=0.45)
    bridge(116, 126, 7, foe="dgrubble")
    L.hint(122, hold=0.45)
    bridge(130, 140, 10, foe="snail", coins=True)
    L.hint(126, hold=0.2)
    L.hint(135, hold=0.45)
    L.add("cinder_hi", cx(143), cy(14), None)
    bridge(145, 152, 9)
    L.hint(140, hold=0.4)
    lift3 = L.lift(155, 9, "x", 40, 44)
    L.hint(152, kind="wait_lift", lift=lift3, near=[155 * T + 24 - 40, 9 * T + 3], tol=6)
    L.hint(152, hold=0.2)
    L.hint(159, hold=0.3)
    bridge(163, 172, 10, foe="dgrubble")
    L.hint(165, hold=0.5)
    L.hint(172, hold=0.3)
    L.stairs(178, 3)
    L.pole(184)
    return L


def l24():
    L = Level("2-4", "ASHEN THORNKEEP", 176, "keep", "#1c0a08", "keep")
    L.fill(0, 175, 0, 2, "W")
    L.fill(0, 12, 9, 14, "W")
    L.start = (2, 8)
    for c in range(5, 170, 10):
        L.add("torch", cx(c), cy(4), None)
    L.fill(13, 17, 12, 14, "~")
    L.add("cinder", cx(15), cy(13), None)
    L.fill(18, 36, 9, 14, "W")
    L.hint(11, hold=0.45)
    L.firebar(23, 8, n=6, speed=120)
    L.firebar(31, 8, n=6, speed=-120, phase=180)
    L.fill(37, 42, 12, 14, "~")
    L.add("cinder", cx(39), cy(13), None)
    L.add("cinder", cx(41), cy(13), None)
    L.fill(43, 47, 10, 14, "W")
    L.fill(48, 52, 12, 14, "~")
    L.fill(53, 74, 9, 14, "W")
    L.hint(35, hold=0.4)
    L.hint(46, hold=0.4)
    L.set(57, 5, "P")
    L.fill(62, 74, 3, 5, "W")
    L.firebar(66, 8, n=5, speed=140)
    L.firebar(71, 8, n=5, speed=140, phase=180)
    L.midpoint(76)
    L.fill(75, 80, 12, 14, "~")
    L.add("cinder", cx(77), cy(13), None)
    L.fill(81, 104, 9, 14, "W")
    L.hint(72, hold=0.5)
    L.firebar(88, 5, n=6, speed=-100)
    L.fill(88, 88, 3, 4, "W")
    L.firebar(96, 8, n=6, speed=100, phase=90)
    L.fill(105, 110, 12, 14, "~")
    L.add("cinder", cx(107), cy(13), None)
    L.fill(111, 128, 9, 14, "W")
    L.hint(102, hold=0.5)
    L.firebar(118, 8, n=7, speed=-110)
    castle(L, boss_col=141, lever_col=148, friend="friend2", length=176, hp=7)
    return L


STAGES = [l11, l12, l13, l14, l21, l22, l23, l24]
BONUS = {"1-1": b11, "2-1": b21}
