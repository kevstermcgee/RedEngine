"""A tiny pixel canvas for Honeybound's art: shapes in palette letters, then an outline, then rows (`describe 2d` sprites: "." is transparent).

Characters are round bugs, so drawing them as filled ellipses with a shading pass and a warm outline gives a consistent, soft storybook look at 16-48 px
without hand-placing every pixel. `sheet()` writes a PNG preview of any sprites (stdlib only: zlib + struct) so the art can be looked at while it is made.
"""
import math
import struct
import zlib


class Canvas:
    def __init__(self, w, h):
        self.w, self.h = w, h
        self.px = {}

    # ---- drawing ------------------------------------------------------------------------------------------------------------------------------
    def dot(self, x, y, ch):
        if 0 <= x < self.w and 0 <= y < self.h:
            if ch == ".":
                self.px.pop((x, y), None)
            else:
                self.px[(x, y)] = ch
        return self

    def get(self, x, y):
        return self.px.get((x, y), ".")

    def rect(self, x0, y0, x1, y1, ch):
        for y in range(y0, y1 + 1):
            for x in range(x0, x1 + 1):
                self.dot(x, y, ch)
        return self

    def ell(self, cx, cy, rx, ry, ch, only=None):
        """A filled ellipse centred on (cx, cy) (half-pixel centres allowed); `only` limits it to pixels already of those letters."""
        for y in range(self.h):
            for x in range(self.w):
                dx, dy = (x + 0.5 - cx) / max(rx, 0.01), (y + 0.5 - cy) / max(ry, 0.01)
                if dx * dx + dy * dy <= 1.0 and (only is None or self.get(x, y) in only):
                    self.dot(x, y, ch)
        return self

    def line(self, x0, y0, x1, y1, ch):
        n = max(abs(x1 - x0), abs(y1 - y0), 1)
        for i in range(n + 1):
            self.dot(round(x0 + (x1 - x0) * i / n), round(y0 + (y1 - y0) * i / n), ch)
        return self

    def shade(self, ch, to, f):
        """Recolour pixels of letter `ch` where f(x, y) holds (a shadow side, a highlight)."""
        for (x, y), c in list(self.px.items()):
            if c == ch and f(x, y):
                self.px[(x, y)] = to
        return self

    def outline(self, ch="k", diagonal=False, skip=""):
        """Every empty pixel next to a drawn one (not of letters in `skip`) becomes `ch`."""
        add = []
        for y in range(self.h):
            for x in range(self.w):
                if (x, y) in self.px:
                    continue
                nb = [(1, 0), (-1, 0), (0, 1), (0, -1)] + ([(1, 1), (-1, -1), (1, -1), (-1, 1)] if diagonal else [])
                if any(self.px.get((x + dx, y + dy), ".") not in "." + skip for dx, dy in nb):
                    add.append((x, y))
        for p in add:
            self.px[p] = ch
        return self

    def stamp(self, rows, x0=0, y0=0):
        for y, row in enumerate(rows):
            for x, ch in enumerate(row):
                if ch != ".":
                    self.dot(x0 + x, y0 + y, ch)
        return self

    def copy(self):
        c = Canvas(self.w, self.h)
        c.px = dict(self.px)
        return c

    def shifted(self, dx, dy):
        c = Canvas(self.w, self.h)
        for (x, y), ch in self.px.items():
            c.dot(x + dx, y + dy, ch)
        return c

    def flipped(self):
        c = Canvas(self.w, self.h)
        for (x, y), ch in self.px.items():
            c.dot(self.w - 1 - x, y, ch)
        return c

    def vflipped(self):
        c = Canvas(self.w, self.h)
        for (x, y), ch in self.px.items():
            c.dot(x, self.h - 1 - y, ch)
        return c

    def rows(self):
        return ["".join(self.px.get((x, y), ".") for x in range(self.w)) for y in range(self.h)]


def ring(cx, cy, r, n, phase=0.0):
    """n points on a circle (for petals, spikes, sparkles)."""
    return [(cx + r * math.cos(phase + 2 * math.pi * k / n), cy + r * math.sin(phase + 2 * math.pi * k / n)) for k in range(n)]


# ---- preview -----------------------------------------------------------------------------------------------------------------------------------------


def _png(w, h, rgba):
    raw = b"".join(b"\x00" + bytes(rgba[y * w * 4:(y + 1) * w * 4]) for y in range(h))

    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


def hexrgba(c):
    c = c.lstrip("#")
    if len(c) == 3:
        c = "".join(ch * 2 for ch in c)
    if len(c) == 6:
        c += "ff"
    return [int(c[i:i + 2], 16) for i in (0, 2, 4, 6)]


def sprite_rgba(s, frame=0, scale=1):
    rows = s["frames"][frame] if "frames" in s else s["rows"]
    h, w = len(rows), len(rows[0])
    out = [0] * (w * scale * h * scale * 4)
    for y, row in enumerate(rows):
        for x, ch in enumerate(row):
            if ch == ".":
                continue
            c = hexrgba(s["palette"][ch])
            for yy in range(scale):
                for xx in range(scale):
                    i = ((y * scale + yy) * w * scale + x * scale + xx) * 4
                    out[i:i + 4] = c
    return w * scale, h * scale, out


def png_of(s, scale=1, frame=0):
    w, h, px = sprite_rgba(s, frame, scale)
    return _png(w, h, px)


def sheet(sprites, path, scale=3, cols=12, bg=(90, 110, 140)):
    """Every sprite (first frame, plus the others to its right) on one PNG, for looking at the art."""
    cells = []
    for name, s in sprites.items():
        for f in range(len(s.get("frames", [None])) if "frames" in s else 1):
            cells.append((name, s, f))
    cw = ch = 64 * scale // 2 + 4
    rows_n = (len(cells) + cols - 1) // cols
    W, H = cols * cw, rows_n * ch
    img = [v for p in range(W * H) for v in (bg[0], bg[1], bg[2], 255)]
    for k, (name, s, f) in enumerate(cells):
        w, h, px = sprite_rgba(s, f, scale if max(len(s.get("rows") or s["frames"][0]), len((s.get("rows") or s["frames"][0])[0])) <= 32 else max(1, scale // 2))
        ox, oy = (k % cols) * cw + 2, (k // cols) * ch + 2
        for y in range(min(h, ch - 2)):
            for x in range(min(w, cw - 2)):
                i = (y * w + x) * 4
                if px[i + 3]:
                    j = ((oy + y) * W + ox + x) * 4
                    img[j:j + 4] = px[i:i + 4]
    with open(path, "wb") as fh:
        fh.write(_png(W, H, img))
