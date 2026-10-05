#!/usr/bin/env python3
"""What the live renderer draws and how long it takes, for a fixed set of scenes, kept as a trend.

Triangles, draw calls and shadow triangles are exact: the same scene gives the same numbers on every machine, so a change in them is a change in the
engine or the content. Milliseconds are not: they depend on the GPU, and where there is none (this dev box, a CI runner) on a software rasteriser
(llvmpipe), which says little about a real GPU. So every record names its adapter, and `compare` only calls a millisecond change real between runs on the
same adapter. Nothing here gates a build; it makes a number visible.

    python3 benches/render_trend.py run [--label "what changed"] [--repeat 5] [--no-record]   # draw the scenes in benches/render_scenes.json
    python3 benches/render_trend.py compare                                                     # the last two recorded runs, scene by scene
    python3 benches/render_trend.py list

Needs the CLI built (`cargo build --bin red_engine2`; `--profile fast|release` selects target/fast|release). Records go to benches/history/render.json.
"""
import argparse, datetime, json, os, pathlib, platform, subprocess, sys, tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
SCENES = ROOT / "benches" / "render_scenes.json"
HISTORY = ROOT / "benches" / "history" / "render.json"
SCHEMA = "red-render/1"


def git(*args):
    r = subprocess.run(["git", "-C", str(ROOT), *args], capture_output=True, text=True)
    return r.stdout.strip() if r.returncode == 0 else ""


def cli(profile):
    return ROOT / "target" / {"debug": "debug", "dev": "debug"}.get(profile, profile) / ("red_engine2.exe" if os.name == "nt" else "red_engine2")


def measure(profile, repeat):
    red = cli(profile)
    if not red.exists():
        sys.exit(f"{red} is not built: cargo build {'--profile ' + profile + ' ' if profile in ('fast', 'release') else ''}--bin red_engine2")
    out, results = pathlib.Path(tempfile.mkdtemp(prefix="render_trend_")), []
    for s in json.loads(SCENES.read_text())["scenes"]:
        stats = out / f"{s['id']}.json"
        cmd = [str(red), "splitshot", str(ROOT / s["scene"]), str(out / f"{s['id']}.png"), "--players", str(s["players"]), "--size", s["size"],
               "--repeat", str(repeat), "--stats", str(stats)]
        if "hour" in s:
            cmd += ["--hour", str(s["hour"])]
        r = subprocess.run(cmd, capture_output=True, text=True, cwd=ROOT)
        if r.returncode != 0:
            sys.exit(f"{s['id']}: {' '.join(cmd)}\n{r.stdout}{r.stderr}")
        d = json.loads(stats.read_text())
        results.append({"id": s["id"], **{k: d[k] for k in ("players", "view", "ms_best", "ms_median", "draws", "tris", "shadow_tris", "resident")}, "adapter": d["adapter"]})
        print(f"  {s['id']:<18} {d['ms_median']:8.0f} ms   " + (f"{d['tris']:>9,} tris  {d['shadow_tris']:>9,} shadow  {d['draws']:>4} draws" if d["streamed"] else "(no streamed world: no counts)"))
    return results


def record(results, label, repeat, profile):
    return {"label": label, "date": datetime.datetime.now().astimezone().isoformat(timespec="seconds"), "engine_commit": git("rev-parse", "--short", "HEAD"),
            "dirty": bool(git("status", "--porcelain", "--untracked-files=no")), "profile": profile, "repeat": repeat,
            "adapter": results[0]["adapter"], "machine": {"cpu": platform.processor() or platform.machine(), "cores": os.cpu_count(), "os": platform.platform()},
            "scenes": results}


def load():
    return json.loads(HISTORY.read_text())["runs"] if HISTORY.exists() else []


def pct(a, b):
    return "n/a" if not a else f"{(b - a) / a * 100:+.1f}%"


def compare(runs):
    if len(runs) < 2:
        sys.exit("need two recorded runs: run `render_trend.py run` before and after a change")
    a, b = runs[-2], runs[-1]
    same = a["adapter"] == b["adapter"]
    print(f"{a['engine_commit']} ({a['label']})  ->  {b['engine_commit']} ({b['label']})")
    print("same adapter: " + b["adapter"] if same else f"DIFFERENT ADAPTERS ({a['adapter']} vs {b['adapter']}): milliseconds are not comparable, triangles are")
    before = {s["id"]: s for s in a["scenes"]}
    for s in b["scenes"]:
        o = before.get(s["id"])
        if not o:
            print(f"  {s['id']:<18} new scene")
            continue
        tris = f"tris {o['tris']:,} -> {s['tris']:,} ({pct(o['tris'], s['tris'])})" if s["tris"] else "tris n/a"
        ms = f"ms {o['ms_median']:.0f} -> {s['ms_median']:.0f} ({pct(o['ms_median'], s['ms_median'])})" if same else "ms not comparable"
        print(f"  {s['id']:<18} {tris}; {ms}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--label", default="")
    r.add_argument("--repeat", type=int, default=5)
    r.add_argument("--profile", default="debug")
    r.add_argument("--no-record", action="store_true")
    r.add_argument("--json-out", help="also write this run's record to a file (CI uploads it)")
    sub.add_parser("compare")
    sub.add_parser("list")
    a = ap.parse_args()
    if a.cmd == "run":
        print(f"render trend, profile {a.profile}, {a.repeat} timed renders per scene")
        rec = record(measure(a.profile, a.repeat), a.label, a.repeat, a.profile)
        print(f"adapter: {rec['adapter']}")
        if a.json_out:
            pathlib.Path(a.json_out).write_text(json.dumps(rec, indent=2) + "\n")
        if not a.no_record:
            runs = load() + [rec]
            HISTORY.parent.mkdir(parents=True, exist_ok=True)
            HISTORY.write_text(json.dumps({"schema": SCHEMA, "runs": runs}, indent=2) + "\n")
            print(f"recorded in {HISTORY.relative_to(ROOT)} ({len(runs)} runs)")
    elif a.cmd == "compare":
        compare(load())
    else:
        for x in load():
            print(f"{x['date'][:16]}  {x['engine_commit']:<8} {x['adapter'][:36]:<36} {x['label']}")


if __name__ == "__main__":
    main()
