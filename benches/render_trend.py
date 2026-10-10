#!/usr/bin/env python3
"""What the live renderer draws and how long it takes, for a fixed set of scenes, kept as a trend.

Triangles, shadow triangles and draw calls are exact: the same scene gives the same numbers on every machine, so a change in them is a change in the engine or the
content. Milliseconds are not: they belong to ONE adapter. Where there is no GPU (a dev box with no usable one, a CI runner) that adapter is a software rasteriser
(llvmpipe), which says little about a player's GPU. So every record names its adapter and says whether it is `software` or a `gpu`, and `compare` only ever sets
milliseconds against a run on the SAME adapter (it picks that pair itself). Nothing here gates a build; it makes a number visible.

    python3 benches/render_trend.py run [--label "what changed"] [--repeat 5] [--no-record]   # draw the scenes in benches/render_scenes.json here
    python3 benches/render_trend.py add gpu.json [--label "RTX 3060"]                         # file a record made on ANOTHER machine (see below)
    python3 benches/render_trend.py compare [--adapter llvmpipe]                              # the two newest runs on one adapter, scene by scene
    python3 benches/render_trend.py list

On a real GPU (a player's machine, or this one once its GPU is usable: `red_engine2 doctor` says why not): the same fixed scenes, no Python needed there,

    red_engine2 render-trend --label "RTX 3060, driver 560" --out gpu.json      # (cargo run --release --bin red_engine2 -- render-trend ... in a checkout)

then bring `gpu.json` back and `add` it. Records on a GPU and records on a software rasteriser sit side by side in `benches/history/render.json` and are never compared
with each other: triangles are, milliseconds are not. Needs the CLI built (`cargo build --bin red_engine2`; `--profile fast|release` selects target/fast|release).
"""
import argparse, datetime, json, os, pathlib, platform, subprocess, sys, tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
SCENES = ROOT / "benches" / "render_scenes.json"
HISTORY = pathlib.Path(os.environ.get("RENDER_TREND_HISTORY") or ROOT / "benches" / "history" / "render.json")  # the override is for tests
SCHEMA = "red-render/1"


def git(*args):
    r = subprocess.run(["git", "-C", str(ROOT), *args], capture_output=True, text=True)
    return r.stdout.strip() if r.returncode == 0 else ""


def cli(profile):
    return ROOT / "target" / {"debug": "debug", "dev": "debug"}.get(profile, profile) / ("red_engine2.exe" if os.name == "nt" else "red_engine2")


def measure(profile, repeat, label):
    """Draws the scenes with `red_engine2 render-trend` and returns its record."""
    red = cli(profile)
    if not red.exists():
        sys.exit(f"{red} is not built: cargo build {'--profile ' + profile + ' ' if profile in ('fast', 'release') else ''}--bin red_engine2")
    out = pathlib.Path(tempfile.mkdtemp(prefix="render_trend_")) / "record.json"
    cmd = [str(red), "render-trend", "--scenes", str(SCENES), "--repeat", str(repeat), "--out", str(out), "--label", label]
    r = subprocess.run(cmd, text=True, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if r.returncode != 0:
        sys.exit(f"{' '.join(cmd)}\n{r.stdout}{r.stderr}")
    print("\n".join(l for l in r.stdout.splitlines() if not l.startswith("wrote ")))
    return json.loads(out.read_text())


def record(native, label, profile, source="this machine"):
    """The native record plus what only git and the date know: commit, dirtiness, when."""
    out = dict(native)
    out["label"] = label or native.get("label", "")
    out.setdefault("date", datetime.datetime.now().astimezone().isoformat(timespec="seconds"))
    out.setdefault("engine_commit", git("rev-parse", "--short", "HEAD") if source == "this machine" else "unknown")
    out.setdefault("dirty", bool(git("status", "--porcelain", "--untracked-files=no")) if source == "this machine" else None)
    out["profile"] = profile
    out["source"] = source
    out.setdefault("adapter_kind", "software" if any(w in out["adapter"].lower() for w in ("llvmpipe", "warp", "swiftshader", "cpu")) else "gpu")
    return out


def load():
    return json.loads(HISTORY.read_text())["runs"] if HISTORY.exists() else []


def pct(a, b):
    return "n/a" if not a else f"{(b - a) / a * 100:+.1f}%"


def compare(runs, adapter=None):
    """Sets the newest run against the previous run ON THE SAME ADAPTER (milliseconds on different adapters mean nothing next to each other)."""
    pool = [r for r in runs if not adapter or adapter.lower() in r["adapter"].lower()]
    if not pool:
        sys.exit("no recorded run matches " + repr(adapter) + "; adapters recorded: " + "; ".join(sorted({r["adapter"] for r in runs})))
    b = pool[-1]
    earlier = [r for r in pool[:-1] if r["adapter"] == b["adapter"]]
    others = sorted({r["adapter"] for r in runs if r["adapter"] != b["adapter"]})
    if not earlier:
        print(f"{b['engine_commit']} ({b['label']}) is the only run on {b['adapter']} [{b.get('adapter_kind', '?')}]: nothing to set it against yet")
        if others:
            print("runs on other adapters exist (" + "; ".join(others) + ") but their milliseconds are not comparable; triangle counts are, and `list` shows them")
        return
    a = earlier[-1]
    print(f"{a['engine_commit']} ({a['label']})  ->  {b['engine_commit']} ({b['label']})")
    print(f"same adapter: {b['adapter']} [{b.get('adapter_kind', '?')}]" + ("  (software: say little about a player's GPU)" if b.get("adapter_kind") == "software" else ""))
    before = {s["id"]: s for s in a["scenes"]}
    for s in b["scenes"]:
        o = before.get(s["id"])
        if not o:
            print(f"  {s['id']:<18} new scene")
            continue
        tris = f"tris {o['tris']:,} -> {s['tris']:,} ({pct(o['tris'], s['tris'])})" if s["tris"] else "tris n/a"
        print(f"  {s['id']:<18} {tris}; ms {o['ms_median']:.0f} -> {s['ms_median']:.0f} ({pct(o['ms_median'], s['ms_median'])})")


def save(runs):
    HISTORY.parent.mkdir(parents=True, exist_ok=True)
    HISTORY.write_text(json.dumps({"schema": SCHEMA, "runs": runs}, indent=2) + "\n")
    shown = HISTORY.relative_to(ROOT) if HISTORY.is_relative_to(ROOT) else HISTORY
    print(f"recorded in {shown} ({len(runs)} runs)")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--label", default="")
    r.add_argument("--repeat", type=int, default=5)
    r.add_argument("--profile", default="debug")
    r.add_argument("--no-record", action="store_true")
    r.add_argument("--json-out", help="also write this run's record to a file (CI uploads it)")
    ad = sub.add_parser("add", help="file a record made by `red_engine2 render-trend` on another machine")
    ad.add_argument("file")
    ad.add_argument("--label", default="")
    ad.add_argument("--commit", default=None, help="the engine commit that machine ran, if you know it")
    c = sub.add_parser("compare")
    c.add_argument("--adapter", default=None, help="only runs whose adapter contains this text")
    sub.add_parser("list")
    a = ap.parse_args()
    if a.cmd == "run":
        print(f"render trend, profile {a.profile}, {a.repeat} timed renders per scene")
        rec = record(measure(a.profile, a.repeat, a.label), a.label, a.profile)
        if a.json_out:
            pathlib.Path(a.json_out).write_text(json.dumps(rec, indent=2) + "\n")
        if not a.no_record:
            save(load() + [rec])
    elif a.cmd == "add":
        native = json.loads(pathlib.Path(a.file).read_text())
        if native.get("schema") != "red-render/1" or not native.get("scenes"):
            sys.exit(f"{a.file} is not a `red_engine2 render-trend` record (schema red-render/1 with scenes)")
        if a.commit:
            native["engine_commit"] = a.commit
        rec = record(native, a.label, native.get("profile", "unknown"), source=f"added from {pathlib.Path(a.file).name}")
        print(f"adding a run on {rec['adapter']} [{rec['adapter_kind']}]")
        save(load() + [rec])
    elif a.cmd == "compare":
        compare(load(), a.adapter)
    else:
        for x in load():
            print(f"{x['date'][:16]}  {x['engine_commit']:<8} {x.get('adapter_kind', '?'):<8} {x['adapter'][:40]:<40} {x['label']}")


if __name__ == "__main__":
    main()
