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


# Stages where the beam search needs a waypoint: it is searched up to `goal`, then the scripted `tail` finishes the stage (the scenario replays both).
# 2-3: riding a moving raft is a long wait the search does not value (walking off the raft's end looks "further" until the fall), so it searches up
# to Pip jumping from the raft and a flutter carries Pip over the bridge.
WAYPOINTS = {
    "2-3": {
        "goal": "p_x >= 2500 && p_y < 172",
        "tail": [{"hold": ["right", "secondary"], "seconds": 0.3}, {"hold": ["right", "secondary", "action"], "seconds": 0.6},
                 {"hold_until": ["right", "secondary"], "until": {"var": "p_x", "gte": 2810}, "timeout": 10}, {"hold": ["right", "secondary", "action"], "seconds": 0.5},
                 {"hold_until": ["right"], "until": {"var": "mode", "eq": 4}, "timeout": 10}],
    },
}


def solve(stages):
    import build as b

    sols = json.loads(SOLUTIONS.read_text()) if SOLUTIONS.exists() else {}
    build()
    wanted = [L for L in b.STAGE_LEVELS if not stages or L.id in stages]
    sols = {k: v for k, v in sols.items() if k in {L.id for L in b.STAGE_LEVELS} | {m["key"] for m in b.MECHANICS}}
    for L in wanted:
        avoid = "mode == 3 || p_y > 250"
        if L.id.endswith("-3"):
            # high up (treetops, bridges): between the ground at the start and the ground at the end every footing is above y 204, so a Pip below it
            # is already falling to its loss; saying so keeps those candidates out of the beam (they look "furthest right" while they fall)
            row = L.grid[13]
            first_gap = next(c for c in range(len(row)) if row[c] == ".")
            last_ground = max(c for c in range(len(row)) if row[c] == "=" and all(ch == "=" for ch in row[c:]))
            avoid += f" || (p_y > 204 && p_x > {first_gap * 16 + 12} && p_x < {last_ground * 16 - 8})"
        cmd = [str(CLI), "solve", str(GAME), "--level", L.id, "--avoid", avoid, "--max-seconds", "300"]
        if L.id in WAYPOINTS:
            wp = WAYPOINTS[L.id]
            r = subprocess.run(cmd + ["--goal", wp["goal"]], capture_output=True, text=True)
            ok = r.returncode == 0
            print(f"{L.id} (waypoint): {'SOLVED to the waypoint' if ok else r.stdout.strip()[-200:]}", flush=True)
            if ok:
                sols[L.id] = json.loads(r.stdout.strip().splitlines()[0]) + wp["tail"]
                SOLUTIONS.write_text(json.dumps(sols, indent=1) + "\n")
            continue
        if L.theme == "keep":
            # the den: get there, then hurt the boss (closing in only while it can be hit, keeping a safe distance otherwise), then take its shard
            w = int(L.id.split("-")[0])
            vuln = {1: "(bs == 1 || bs == 2)", 7: "(bs == 4)", 8: "(bs == 4)"}.get(w, "(bs == 3)")
            corridor = "(den == 0) * p_x"
            fight = f"den * 6000 + (bmax - bhp) * 4000 + form * 300 - {vuln} * abs(p_x - boss_x) - (1 - {vuln}) * (den == 1) * abs(abs(p_x - boss_x) - 90) * 0.3"
            cmd += ["--goal", "mode == 6", "--progress", f"{corridor} + {fight} + (den == 2) * (3000 - abs(p_x - den_c_x))"]
        else:
            cmd += ["--goal", "mode == 4"]
        for beam in (48, 128, 320):
            r = subprocess.run(cmd + ["--beam", str(beam)], capture_output=True, text=True)
            last = r.stdout.strip().splitlines()[-1] if r.stdout.strip() else r.stderr.strip()
            print(f"{L.id} (beam {beam}): {last}", flush=True)
            if r.returncode == 0:
                sols[L.id] = json.loads(r.stdout.strip().splitlines()[0])
                SOLUTIONS.write_text(json.dumps(sols, indent=1) + "\n")  # kept as it goes: a long run that stops loses nothing
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
            print(f"@{m['key']} (beam {beam}): {last}", flush=True)
            if r.returncode == 0:
                sols[m["key"]] = json.loads(r.stdout.strip().splitlines()[0])
                SOLUTIONS.write_text(json.dumps(sols, indent=1) + "\n")
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
