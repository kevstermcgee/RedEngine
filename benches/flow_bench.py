#!/usr/bin/env python3
"""Idea-to-game flow benchmark: how long each step of making a game takes, and how much output an agent would read.

A *flow* (`benches/flows/*.json`) is an ordered list of engine CLI steps, the way an agent works through a game: scaffold, build, check,
prove the rules, measure the server, test the network. Each step is run for real against the built CLI and recorded: wall time, exit
code and output size. Output size is the part that costs an agent tokens, so `approx_tokens` (bytes / 4) is kept beside it; it is an
ESTIMATE, not a model measurement. Nothing here calls a model, so a run costs no API tokens.

    python3 benches/flow_bench.py run benches/flows/hello_game.json            # run and append to benches/history/flows.json
    python3 benches/flow_bench.py run benches/flows/hello_game.json --no-record  # run and print only
    python3 benches/flow_bench.py compare hello_game                            # the last two recorded runs of that flow, step by step
    python3 benches/flow_bench.py list                                          # recorded runs

The CLI must already be built (`cargo build --bin red_engine2`, or `--profile fast`; `--profile fast` here selects target/fast). Times are wall
clock on whatever else the machine is doing: the load average at the start is recorded, and a busy machine says so in the output.
Flow file format: {"name", "description", "steps": [{"id", "cmd": [...], "kind"?, "cwd"?, "timeout"?, "expect_exit"?, "continue"?, "edit"?}]}; in `cmd` and `cwd`,
{RED} is the CLI, {ENGINE} the engine checkout (this one, or `--engine PATH`: a warm checkout to measure edit loops in), {WORK} a fresh temporary directory.

`kind` says what a step is for: `discovery` (finding out what to do: describe, context, search), `edit` (changing a file; `edit` below), `build`, `verify` (focused checks while iterating),
`ship` (package, publish, full verification). The totals per kind are the answer to "where did the time go": how much was discovery, how much was verification, how much shipping.
An `edit` step changes a real file and is ALWAYS undone when the flow ends, even when it fails: {"file": "{ENGINE}/src/x.rs", "append": "text"} or {"file": ..., "replace": ["old", "new"]}.
Every step also records CPU seconds (user + system, children included) and peak memory, because wall time on a shared 4-core machine mostly measures what else is running.
"""
import argparse, datetime, json, os, pathlib, platform, shutil, subprocess, sys, tempfile, time

ROOT = pathlib.Path(__file__).resolve().parent.parent
HISTORY = ROOT / "benches" / "history" / "flows.json"
SCHEMA = "red-flows/1"


def cpu_model():
    try:
        for line in open("/proc/cpuinfo"):
            if line.startswith("model name"):
                return line.split(":", 1)[1].strip()
    except OSError:
        pass
    return platform.processor() or "unknown"


def git(*args):
    r = subprocess.run(["git", "-C", str(ROOT), *args], capture_output=True, text=True)
    return r.stdout.strip() if r.returncode == 0 else ""


def cli_path(profile, engine=None):
    d = {"debug": "debug", "dev": "debug", "release": "release", "fast": "fast"}.get(profile, profile)
    target = os.environ.get("CARGO_TARGET_DIR")
    return (pathlib.Path(target) if target else (engine or ROOT) / "target") / d / "red_engine2"


TIME = shutil.which("time") and pathlib.Path("/usr/bin/time").exists() and "/usr/bin/time"


def apply_edit(edit, subst, undo):
    """Changes a file as the flow says, remembering the original so it can be put back."""
    path = pathlib.Path(subst(edit["file"]))
    original = path.read_text()
    undo.setdefault(path, original)
    if "append" in edit:
        path.write_text(original + subst(edit["append"]))
    else:
        old, new = (subst(x) for x in edit["replace"])
        if old not in original:
            raise RuntimeError(f"{path}: the text to replace is not there: {old[:60]!r}")
        path.write_text(original.replace(old, new, 1))


def run_flow(flow_path, profile, record, label, engine=None):
    flow = json.loads(pathlib.Path(flow_path).read_text())
    engine = pathlib.Path(engine).resolve() if engine else ROOT
    red = cli_path(profile, engine)
    if not red.exists():
        sys.exit(f"{red} is not built: cargo build {'--profile ' + profile if profile in ('fast', 'release') else ''} --bin red_engine2".replace("  ", " "))
    work = pathlib.Path(tempfile.mkdtemp(prefix="flow_bench_"))
    subst = lambda s: s.replace("{RED}", str(red)).replace("{ENGINE}", str(engine)).replace("{WORK}", str(work))
    undo = {}
    load1 = os.getloadavg()[0]
    env = dict(os.environ, TERM="dumb", NO_COLOR="1")
    steps, ok_all = [], True
    print(f"flow {flow['name']} on {cpu_model()} ({os.cpu_count()} cores), profile {profile}, load {load1:.2f}")
    if load1 > 1.0:
        print("  WARNING: the machine is busy; wall times include that")
    try:
        for st in flow["steps"]:
            if "edit" in st:
                t0 = time.perf_counter()
                apply_edit(st["edit"], subst, undo)
                dt = time.perf_counter() - t0
                steps.append({"id": st["id"], "kind": "edit", "seconds": round(dt, 2), "cpu_seconds": 0.0, "max_rss_mb": 0, "exit": 0, "ok": True, "output_bytes": 0, "approx_tokens": 0, "first_line": "edited " + pathlib.Path(subst(st["edit"]["file"])).name})
                print(f"  ok   {st['id']:<14} {dt:7.2f} s  (edit, undone at the end)")
                continue
            cmd = [subst(c) for c in st["cmd"]]
            cwd = subst(st["cwd"]) if "cwd" in st else str(work)
            pathlib.Path(cwd).mkdir(parents=True, exist_ok=True)
            stats = pathlib.Path(work) / f".time_{st['id']}"
            timed = [TIME, "-f", "%U %S %M", "-o", str(stats)] + cmd if TIME else cmd
            t0 = time.perf_counter()
            try:
                p = subprocess.run(timed, cwd=cwd, capture_output=True, text=True, env=env, timeout=st.get("timeout", 600))
                code, out = p.returncode, (p.stdout or "") + (p.stderr or "")
            except subprocess.TimeoutExpired as e:
                code, out = 124, f"timed out after {st.get('timeout', 600)} s\n" + ((e.stdout or b"").decode() if isinstance(e.stdout, bytes) else (e.stdout or ""))
            dt = time.perf_counter() - t0
            cpu, rss = 0.0, 0
            try:
                u, sy, kb = stats.read_text().split()[-3:]
                cpu, rss = float(u) + float(sy), int(kb) // 1024
            except (OSError, ValueError):
                pass
            expect = st.get("expect_exit", 0)
            nbytes = len(out.encode())
            first = next((ln for ln in out.splitlines() if ln.strip()), "")[:160]
            rec = {"id": st["id"], "kind": st.get("kind", "verify"), "seconds": round(dt, 2), "cpu_seconds": round(cpu, 2), "max_rss_mb": rss, "exit": code, "ok": code == expect, "output_bytes": nbytes, "approx_tokens": nbytes // 4, "first_line": first}
            steps.append(rec)
            print(f"  {'ok  ' if rec['ok'] else 'FAIL'} {st['id']:<14} {dt:7.2f} s wall {cpu:7.2f} s cpu {rss:5d} MB  {nbytes:7d} B out (~{nbytes // 4} tok)  {first[:50]}")
            if not rec["ok"]:
                ok_all = False
                print("       " + "\n       ".join(out.strip().splitlines()[-6:]))
                if not st.get("continue"):
                    break
    finally:
        for path, original in undo.items():
            path.write_text(original)
    shutil.rmtree(work, ignore_errors=True)
    total_s = round(sum(s["seconds"] for s in steps), 2)
    total_b = sum(s["output_bytes"] for s in steps)
    total_cpu = round(sum(s.get("cpu_seconds", 0) for s in steps), 2)
    by_kind = {}
    for st_ in steps:
        k = by_kind.setdefault(st_["kind"], {"steps": 0, "seconds": 0.0, "cpu_seconds": 0.0, "output_bytes": 0})
        k["steps"] += 1
        k["seconds"] = round(k["seconds"] + st_["seconds"], 2)
        k["cpu_seconds"] = round(k["cpu_seconds"] + st_.get("cpu_seconds", 0), 2)
        k["output_bytes"] += st_["output_bytes"]
    dirty = bool(git("status", "--porcelain", "--untracked-files=no"))
    run = {
        "schema": SCHEMA, "flow": flow["name"], "label": label, "date": datetime.datetime.now().astimezone().isoformat(timespec="seconds"),
        "engine_commit": git("rev-parse", "--short", "HEAD") + ("+dirty" if dirty else ""), "profile": profile,
        "machine": {"cpu": cpu_model(), "cores": os.cpu_count(), "os": platform.platform()}, "load_avg_1m_at_start": round(load1, 2),
        "ok": ok_all, "total_seconds": total_s, "total_cpu_seconds": total_cpu, "total_output_bytes": total_b, "total_approx_tokens": total_b // 4,
        "commands": sum(1 for s in steps if s["kind"] != "edit"), "by_kind": by_kind, "steps": steps,
    }
    print(f"  total {total_s} s wall, {total_cpu} s cpu, {total_b} B out (~{total_b // 4} tokens), {run['commands']} commands, {'all steps ok' if ok_all else 'FAILED'}")
    print("  by kind: " + "; ".join(f"{k} {v['seconds']} s / {v['output_bytes']} B" for k, v in by_kind.items()))
    if record and ok_all:
        data = json.loads(HISTORY.read_text()) if HISTORY.exists() else {"schema": SCHEMA, "runs": []}
        data["runs"].append(run)
        HISTORY.write_text(json.dumps(data, indent=2) + "\n")
        print(f"  recorded in {HISTORY.relative_to(ROOT)} (run {len(data['runs'])})")
    elif record:
        print("  not recorded: a failed flow is not a measurement")
    return 0 if ok_all else 1


def load_runs(flow):
    if not HISTORY.exists():
        return []
    return [r for r in json.loads(HISTORY.read_text())["runs"] if flow is None or r["flow"] == flow]


def compare(flow):
    runs = load_runs(flow)
    if len(runs) < 2:
        sys.exit(f"need two recorded runs of '{flow}' (have {len(runs)})")
    a, b = runs[-2], runs[-1]
    print(f"{flow}: {a['date'][:16]} {a['engine_commit']} ({a['profile']})  ->  {b['date'][:16]} {b['engine_commit']} ({b['profile']})")
    by = {s["id"]: s for s in a["steps"]}
    for s in b["steps"]:
        o = by.get(s["id"])
        if not o:
            print(f"  {s['id']:<14} new step {s['seconds']} s")
            continue
        pct = lambda x, y: f"{(y - x) / x * 100:+.0f}%" if x else "n/a"
        print(f"  {s['id']:<14} {o['seconds']:7.2f} -> {s['seconds']:7.2f} s ({pct(o['seconds'], s['seconds'])})   {o['output_bytes']:7d} -> {s['output_bytes']:7d} B ({pct(o['output_bytes'], s['output_bytes'])})")
    print(f"  {'total':<14} {a['total_seconds']:7.2f} -> {b['total_seconds']:7.2f} s   {a['total_output_bytes']} -> {b['total_output_bytes']} B")
    print("  wall times under a differing load average, or differences under about 20%, are noise")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("flow")
    r.add_argument("--profile", default="debug", help="which built CLI: debug (default), fast, release")
    r.add_argument("--no-record", action="store_true")
    r.add_argument("--label", default="", help="free text stored with the run, e.g. what changed")
    r.add_argument("--engine", default=None, help="the engine checkout to run in (default this one): a warm checkout, for flows that edit and rebuild")
    c = sub.add_parser("compare")
    c.add_argument("flow")
    sub.add_parser("list")
    a = ap.parse_args()
    if a.cmd == "run":
        sys.exit(run_flow(a.flow, a.profile, not a.no_record, a.label, a.engine))
    if a.cmd == "compare":
        compare(a.flow)
    else:
        for r in load_runs(None):
            print(f"{r['date'][:16]}  {r['flow']:<16} {r['engine_commit']:<14} {r['profile']:<8} {r['total_seconds']:8.2f} s  {r['total_output_bytes']:7d} B  {'ok' if r['ok'] else 'FAILED'}  {r['label']}")


if __name__ == "__main__":
    main()
