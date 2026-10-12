#!/usr/bin/env python3
"""Honeybound's development loop.

  python3 dev.py solve [STAGE ...]   find a run through each stage with `red_engine2 solve` (enemies included), store it in solutions.json,
                                     rebuild the game: its per-stage scenarios replay these runs, so `verify` proves every stage can be finished
  python3 dev.py check               build, validate and verify the game

A run is the engine's own search over the keys a player holds each tenth of a second; it is redone whenever a level changes (an old run no
longer fits the new layout, and verify says so).
"""
import json, subprocess, sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
GAME = HERE.parent / "honeybound.game2d.json"
SOLUTIONS = HERE / "solutions.json"
CLI = ROOT / "target" / "fast" / "red_engine2"


def build():
    subprocess.run([sys.executable, str(HERE / "build.py")], check=True, cwd=HERE)


def solve(stages):
    import build as b

    sols = json.loads(SOLUTIONS.read_text()) if SOLUTIONS.exists() else {}
    build()
    wanted = [L for L in b.STAGE_LEVELS if not stages or L.id in stages]
    sols = {k: v for k, v in sols.items() if k in {L.id for L in b.STAGE_LEVELS} | {m["key"] for m in b.MECHANICS}}
    for L in wanted:
        goal = "mode == 6" if L.theme == "castle" else "mode == 4"
        cmd = [str(CLI), "solve", str(GAME), "--level", L.id, "--goal", goal, "--avoid", "mode == 3", "--max-seconds", "300"]
        for beam in (48, 128, 320):
            r = subprocess.run(cmd + ["--beam", str(beam)], capture_output=True, text=True)
            last = r.stdout.strip().splitlines()[-1] if r.stdout.strip() else r.stderr.strip()
            print(f"{L.id} (beam {beam}): {last}")
            if r.returncode == 0:
                sols[L.id] = json.loads(r.stdout.strip().splitlines()[0])
                break
    for m in b.MECHANICS:
        if stages and m["key"] not in stages:
            continue
        cmd = [str(CLI), "solve", str(GAME), "--level", m["level"], "--goal", m["goal"], "--avoid", m.get("avoid", "mode == 3"), "--max-seconds", "60"]
        if "progress" in m:
            cmd += ["--progress", m["progress"]]
        for beam in (48, 160):
            r = subprocess.run(cmd + ["--beam", str(beam)], capture_output=True, text=True)
            last = r.stdout.strip().splitlines()[-1] if r.stdout.strip() else r.stderr.strip()
            print(f"@{m['key']} (beam {beam}): {last}")
            if r.returncode == 0:
                sols[m["key"]] = json.loads(r.stdout.strip().splitlines()[0])
                break
    SOLUTIONS.write_text(json.dumps(sols, indent=1) + "\n")
    build()


def check():
    build()
    for verb in ("validate", "verify"):
        r = subprocess.run([str(CLI), verb, str(GAME)], capture_output=True, text=True)
        lines = r.stdout.splitlines()
        print("\n".join(l for l in lines if l.startswith("FAIL") or l.startswith("OK") or "passed" in l))


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "solve":
        solve(sys.argv[2:])
    else:
        check()
