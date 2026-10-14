#!/usr/bin/env python3
"""Picture parity for a change that must not change what is drawn: the same pictures from two builds, compared.

    python3 scripts/picture_parity.py OLD_BIN_DIR NEW_BIN_DIR [--out DIR] [--runs N] [--old-museum re2|frameless] [--new-museum re2|frameless]

Each BIN_DIR holds `red_engine2` and `re2` built from one commit, and `frameless` when that build plays the museum with it (copy them out of
target/debug before switching branches). Run from the repository root. The set: the offline renderer (`museum` stills of Frameless at seed 7, `frame`
of the endless meadow at four times, `splitshot`), and the live renderer (`--headless --shot-at`: the meadow while walking in `re2`, Frameless while
visiting). Frameless is played by `frameless` (`frameless museum` for its stills) since it became a crate of its own; `--old-museum re2` plays it
the way a build from before that did (`red_engine2 museum`, `re2`). Both read `--museum-scene` (default games/frameless/content/frameless.json). Offline pictures must be byte-identical. Live ones can
depend on timing (the meadow's streaming workers, the museum's flights), so each build runs `--runs` times (default 3): a live picture passes when
every new one equals one an old run drew, or when the nearest new one differs from an old one in no more pixels than the old runs differ from each
other (the report gives both, with the largest channel difference). Used for ADR 2026-10-14-a-game-ships-as-its-own-binary-the-client-is-a-library's moves.
"""
import argparse
import json
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import zlib
from pathlib import Path

MUSEUM = "games/frameless/content/frameless.json"
MEADOW = "examples/endless_meadow.json"
VISIT = {"policy": "idle", "steps": [
    {"press": "start"}, {"wait": 1},
    {"approach": "museum:5"}, {"look_at": "museum:5"}, {"interact": 1},
    {"wait_for": {"at": "/museum/place", "contains": "inside"}},
    {"look_at": "museum:key"}, {"approach": "museum:key", "within": 1.6}, {"look_at": "museum:key"},
    {"hold": ["forward"], "secs": 0.8},
    {"wait_for": {"at": "/museum/keys", "eq": 1}},
    {"approach": "museum:way-back"}, {"look_at": "museum:way-back"}, {"interact": 1},
    {"wait_for": {"at": "/museum/place", "eq": "halls"}},
]}
WALK = {"policy": "idle", "steps": [{"wait": 1}, {"hold": ["forward"], "secs": 2}, {"wait": 2}]}


def offline(b: Path, out: Path, museum_by: str, museum: str) -> None:
    out.mkdir(parents=True, exist_ok=True)
    cli = str(b / "red_engine2")
    tool = [str(b / "red_engine2"), "museum"] if museum_by == "re2" else [str(b / "frameless"), "museum"]
    runs = [
        tool + [museum, "--seed", "7", "--shot", out / "museum-room.png"],
        tool + [museum, "--seed", "7", "--inside", "1", "--shot", out / "museum-inside.png"],
        tool + [museum, "--seed", "7", "--look", "2", "--shot", out / "museum-look.png"],
        tool + [museum, "--seed", "7", "--styles", out / "museum-styles.png"],
        [cli, "splitshot", MEADOW, out / "meadow-split.png"],
    ] + [[cli, "frame", MEADOW, out / f"meadow-t{t}.png", "--t", str(t)] for t in (0, 40, 120, 300)]
    for r in runs:
        subprocess.run([str(a) for a in r], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def live(b: Path, out: Path, scratch: Path, museum_by: str, museum: str) -> None:
    (scratch / "visit.json").write_text(json.dumps(VISIT))
    (scratch / "walk.json").write_text(json.dumps(WALK))
    env = dict(os.environ, RE2_SEED="7", RE2_SAVE_DIR=str(scratch / "save"), RE2_MUSEUM_FRESH="1")
    shutil.rmtree(scratch / "save", ignore_errors=True)
    for client, scene, script, shots, name in ((museum_by, museum, "visit.json", "2,5,8", "live-museum"), ("re2", MEADOW, "walk.json", "0.5,3,4.5", "live-meadow")):
        cmd = [str(b / client), scene, "--headless", "--script", str(scratch / script), "--shot-at", shots, "--shot-dir", str(out / name)]
        subprocess.run(cmd, check=True, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def pixels(path: Path):
    """An 8-bit RGB or RGBA PNG's size, bytes per pixel and unfiltered rows (the standard library has no PNG reader)."""
    data = path.read_bytes()
    pos, idat = 8, b""
    while pos < len(data):
        n, kind = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + n]
        if kind == b"IHDR":
            w, h, depth, ctype = struct.unpack(">IIBB", body[:10])
        elif kind == b"IDAT":
            idat += body
        pos += 12 + n
    if depth != 8 or ctype not in (2, 6):
        raise SystemExit(f"{path}: only 8-bit RGB/RGBA pictures are compared")
    bpp = 3 if ctype == 2 else 4
    raw, stride, rows, prev, i = zlib.decompress(idat), w * bpp, [], bytearray(w * bpp), 0
    for _ in range(h):
        f, line = raw[i], bytearray(raw[i + 1:i + 1 + stride])
        i += 1 + stride
        for x in range(stride):
            a = line[x - bpp] if x >= bpp else 0
            up, c = prev[x], (prev[x - bpp] if x >= bpp else 0)
            if f == 1:
                line[x] = (line[x] + a) & 255
            elif f == 2:
                line[x] = (line[x] + up) & 255
            elif f == 3:
                line[x] = (line[x] + (a + up) // 2) & 255
            elif f == 4:
                p = a + up - c
                pa, pb, pc = abs(p - a), abs(p - up), abs(p - c)
                line[x] = (line[x] + (a if pa <= pb and pa <= pc else up if pb <= pc else c)) & 255
        rows.append(bytes(line))
        prev = line
    return w, h, bpp, rows


def distance(a: Path, b: Path):
    """(pixels that differ, their share, the largest channel difference)."""
    wa, ha, ba, ra = pixels(a)
    wb, hb, bb, rb = pixels(b)
    if (wa, ha) != (wb, hb):
        return (wa * ha, 1.0, 255)
    n = top = 0
    for la, lb in zip(ra, rb):
        if la == lb:
            continue
        for x in range(wa):
            d = max(abs(la[x * ba + k] - lb[x * bb + k]) for k in range(3))
            if d:
                n, top = n + 1, max(top, d)
    return (n, n / (wa * ha), top)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("old", type=Path)
    ap.add_argument("new", type=Path)
    ap.add_argument("--out", type=Path, default=Path("out/picture-parity"))
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--old-museum", choices=["re2", "frameless"], default="frameless", help="what plays the museum in the old build")
    ap.add_argument("--new-museum", choices=["re2", "frameless"], default="frameless", help="what plays the museum in the new build")
    ap.add_argument("--museum-scene", default=MUSEUM)
    a = ap.parse_args()
    shutil.rmtree(a.out, ignore_errors=True)
    with tempfile.TemporaryDirectory() as scratch:
        for side, b, by in (("old", a.old, a.old_museum), ("new", a.new, a.new_museum)):
            offline(b, a.out / side / "offline", by, a.museum_scene)
            for r in range(a.runs):
                live(b, a.out / side / f"live{r}", Path(scratch), by, a.museum_scene)
    ok = True
    old_off, new_off = a.out / "old/offline", a.out / "new/offline"
    for p in sorted(old_off.glob("*.png")):
        same = (new_off / p.name).read_bytes() == p.read_bytes()
        ok &= same
        if same:
            print(f"offline {p.name:28} identical")
        else:
            n, share, top = distance(p, new_off / p.name)
            print(f"offline {p.name:28} DIFFERENT: {n} px ({100 * share:.3f}%), max {top}")
    for p in sorted((a.out / "old/live0").rglob("*.png")):
        rel = p.relative_to(a.out / "old/live0")
        olds = [a.out / "old" / f"live{r}" / rel for r in range(a.runs)]
        news = [a.out / "new" / f"live{r}" / rel for r in range(a.runs)]
        old_bytes = {o.read_bytes() for o in olds}
        matched = all(n.read_bytes() in old_bytes for n in news)
        if matched:
            print(f"live    {str(rel):28} every new picture equals an old run's ({len(old_bytes)} variant(s) among the old runs)")
            continue
        spread = max((distance(olds[0], o) for o in olds[1:]), key=lambda d: d[0], default=(0, 0.0, 0))
        nearest = min((distance(n, o) for n in news for o in olds), key=lambda d: d[0])
        print(f"live    {str(rel):28} old runs differ by up to {spread[0]} px (max {spread[2]}); nearest new is {nearest[0]} px (max {nearest[2]}) from an old")
        ok &= nearest[0] <= spread[0]
    print("parity: OK" if ok else "parity: DIFFERENT")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
