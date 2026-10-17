#!/usr/bin/env python3
"""A game's generator, kept beside it, still writes the game it was committed with.

A game whose rules, levels or objects are written by a script keeps that script in its own folder: `examples/2d/<slug>/` beside
`examples/2d/<slug>.game2d.json` (`examples/2d/honeybound/` is the pattern), `examples/3d/<slug>/` beside `<slug>.json`. A generator is
`build.py` there (or `gen.py` when there is no `build.py`); helper modules may sit beside it; it uses only the standard library and paths relative to itself, and running it again rewrites the committed
files byte for byte, so the next change is an edit to the generator, not to its output.

    python3 scripts/game_generators.py check [GAME_DIR ...]   run every generator (or those of the given game folders) in a temporary copy of
                                                              its folder and compare what it wrote with the committed files; exit 1 on a difference
    python3 scripts/game_generators.py list                   the generators and the game file each writes

Hash randomisation is fixed (PYTHONHASHSEED=0) so iterating a set gives the same order every run; anything else random needs its own seed.
"""
import argparse
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
NAMES = ("build.py", "gen.py")   # the entry point; `build.py` wins (Honeybound's `gen.py` is a module its `build.py` imports)
TIMEOUT = 120
SKIP = {"__pycache__"}


def find(root=None):
    """Every (generator, game file) pair under examples/2d and examples/3d, sorted."""
    root = Path(root) if root else ROOT
    out = []
    for kind in ("2d", "3d"):
        base = root / "examples" / kind
        if not base.is_dir():
            continue
        for folder in sorted(p for p in base.iterdir() if p.is_dir()):
            gen = next((folder / n for n in NAMES if (folder / n).is_file()), None)
            if gen is None:
                continue
            game = base / f"{folder.name}.game2d.json" if kind == "2d" else folder / f"{folder.name}.json"
            out.append((gen, game))
    return out


def for_game(root, kind, slug):
    """The (generator, game file) pair of one game, or None when it has no generator."""
    return next(((g, game) for g, game in find(root) if g.parent.name == slug and g.parent.parent.name == kind), None)


def committed(folder, game):
    """The files a generator may write, relative to the examples/<kind> directory: everything in its folder and its game file."""
    base = folder.parent
    files = {p.relative_to(base) for p in folder.rglob("*") if p.is_file() and not SKIP & set(p.parts)}
    files.add(game.relative_to(base))
    return files


def check_one(gen, game, root=None):
    """Runs `gen` in a temporary copy of its folder (without the game file, so a generator that writes nothing fails) and returns
    (ok, seconds, problems); paths in the problems are relative to `root` (the checkout)."""
    root = Path(root or ROOT)
    folder, base = gen.parent, gen.parent.parent
    name = lambda p: p.relative_to(root).as_posix()   # noqa: E731
    if not game.is_file():
        return False, 0.0, [f"{name(gen)}: its game {name(game)} is not committed"]
    with tempfile.TemporaryDirectory(prefix="game-gen-") as tmp:
        tbase = Path(tmp) / base.relative_to(root)
        shutil.copytree(folder, tbase / folder.name, ignore=shutil.ignore_patterns(*SKIP))
        (tbase / game.relative_to(base)).unlink(missing_ok=True)
        env = dict(os.environ, PYTHONHASHSEED="0", PYTHONDONTWRITEBYTECODE="1")
        t0 = time.monotonic()
        try:
            p = subprocess.run([sys.executable, gen.name], cwd=tbase / folder.name, env=env, capture_output=True, text=True, timeout=TIMEOUT)
        except subprocess.TimeoutExpired:
            return False, time.monotonic() - t0, [f"{name(gen)}: did not finish in {TIMEOUT} s"]
        secs = time.monotonic() - t0
        if p.returncode:
            tail = (p.stderr or p.stdout).strip().splitlines()[-3:]
            return False, secs, [f"{name(gen)}: exited {p.returncode}: " + " | ".join(tail)]
        problems = []
        want = committed(folder, game)
        wrote = {q.relative_to(tbase) for q in tbase.rglob("*") if q.is_file() and not SKIP & set(q.parts)}
        for rel in sorted(want):
            mine, theirs = base / rel, tbase / rel
            if not theirs.is_file():
                if rel == game.relative_to(base):
                    problems.append(f"{name(gen)}: did not write {name(game)} (write it from a path relative to the script: Path(__file__).parent)")
                continue
            if mine.read_bytes() != theirs.read_bytes():
                problems.append(f"{name(gen)}: writes a different {name(base / rel)} than the committed one ({first_difference(mine, theirs)}): "
                                "re-run the generator and commit its output, or make it deterministic")
        for rel in sorted(wrote - want):
            problems.append(f"{name(gen)}: writes {name(base / rel)}, which is not committed (commit it, or stop writing it)")
        return not problems, secs, problems


def first_difference(a, b):
    la, lb = a.read_bytes().splitlines(), b.read_bytes().splitlines()
    for i, (x, y) in enumerate(zip(la, lb), 1):
        if x != y:
            return f"first difference at line {i}"
    return f"{len(la)} lines committed, {len(lb)} written"


def check(pairs, root=None):
    """Checks every pair; returns (problems, one `name seconds` entry per generator)."""
    problems, times = [], []
    for g, game in pairs:
        _ok, secs, found = check_one(g, game, root)
        times.append(f"{g.parent.name} {secs:.2f} s")
        problems += found
    return problems, times


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("check", help="run the generators and compare their output with the committed files")
    c.add_argument("dirs", nargs="*", help="game folders (or game files) to check; default every one")
    sub.add_parser("list", help="the generators and their games")
    a = ap.parse_args(argv)
    pairs = find()
    if a.cmd == "list":
        for g, game in pairs:
            print(f"{g.relative_to(ROOT).as_posix()} -> {game.relative_to(ROOT).as_posix()}")
        return 0
    if a.dirs:
        want = {Path(d).resolve() for d in a.dirs}
        pairs = [(g, game) for g, game in pairs if g.parent.resolve() in want or game.resolve() in want]
        if not pairs:
            print(f"game_generators: no generator in {', '.join(a.dirs)} (a generator is build.py or gen.py in the game's folder)")
            return 0
    problems, times = check(pairs)
    for b in problems:
        print("FAIL", b)
    failed = len({b.split(":")[0] for b in problems})
    print(f"game_generators: {len(pairs)} generator(s), {len(pairs) - failed} reproduce their game" + (f" ({', '.join(times)})" if times else ""))
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
