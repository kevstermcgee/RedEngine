#!/usr/bin/env python3
"""The fresh-agent benchmark: score an end state, summarise how an agent used the engine, and run a deterministic reference agent (bench/fresh-agent/README.md).

  agent_bench.py score DIR [--engine PATH] [--before FILE] [--no-browser]   scorecard for the game in DIR (run its commands from inside DIR)
  agent_bench.py summary TRACE.jsonl [--transcript SESSION.jsonl]            friction report: documentation read, failures, retries, repairs, source exploration
  agent_bench.py reference WORKDIR [--engine PATH] [--no-browser]            a stand-in agent that follows only what `red_engine2 describe web` says

Standard library only. The engine is `--engine`, else $RED_ENGINE, else target/debug/red_engine2 next to this script's repository.
"""
import argparse, glob, json, os, shutil, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def engine_path(a):
    for c in (getattr(a, "engine", None), os.environ.get("RED_ENGINE"), os.path.join(ROOT, "target/debug/red_engine2")):
        if c and os.path.isfile(c):
            return os.path.abspath(c)
    sys.exit("no red_engine2 binary: pass --engine or set RED_ENGINE")


def run(engine, args, cwd, env=None, timeout=1800):
    e = dict(os.environ)
    e.update(env or {})
    p = subprocess.run([engine] + args, cwd=cwd, capture_output=True, text=True, env=e, timeout=timeout)
    return p.returncode, p.stdout + p.stderr


def status(engine, game, cwd):
    code, out = run(engine, ["--json", "web", "status", game], cwd)
    try:
        return json.loads(out)["data"]
    except Exception:
        return {"valid": False, "errors": [out[:300]]}


# ---- score -------------------------------------------------------------------------------------------------------------------------------------------

def score(a):
    d = os.path.abspath(a.dir)
    engine = engine_path(a)
    gid = "star-dash"
    game = os.path.join(d, gid + ".game2d.json")
    items = []

    def item(name, ok, detail, needs_browser=False):
        if needs_browser and a.no_browser:
            items.append({"name": name, "status": "skipped", "detail": "--no-browser"})
        else:
            items.append({"name": name, "status": "pass" if ok else "FAIL", "detail": detail})

    if not os.path.isfile(game):
        item("the game file exists", False, game + " is missing (the task asks for ./star-dash/star-dash.game2d.json)")
        return finish(items)
    g = json.load(open(game))
    code, out = run(engine, ["validate", gid + ".game2d.json"], d)
    item("the game validates", code == 0, out.strip().splitlines()[0] if out.strip() else "")
    caps = g.get("capabilities", {})
    item("it is a browser game (2d or hybrid, platform web)", caps.get("presentation") in ("2d", "hybrid") and "web" in caps.get("platforms", []), json.dumps({k: caps.get(k) for k in ("presentation", "platforms")}))
    ids = [s.get("id") for s in g.get("scene", [])]
    item("the player has the scene id `p` and the countdown variable is `timeleft`", "p" in ids and "timeleft" in g.get("vars", {}), "ids %s, vars %s" % (ids[:4], list(g.get("vars", {}))[:6]))
    item("at least six things to collect are placed", sum(1 for s in g.get("scene", []) if s.get("id") != "p") >= 6, "%d other placements" % sum(1 for s in g.get("scene", []) if s.get("id") != "p"))
    scs = g.get("checks", {}).get("scenarios", [])
    ends = [next((e.get("ended") for e in sc.get("expect", []) if "ended" in e), None) for sc in scs]
    item("a scripted playthrough wins", "win" in ends, "scenarios end: %s" % ends)
    item("a scripted playthrough loses", "lose" in ends, "scenarios end: %s" % ends)
    bro = g.get("checks", {}).get("browser", [])
    item("a browser check says the saved value survives a reload", any(b.get("persists") for b in bro) and bool(g.get("persist")), "persist %s, browser checks with persists: %s" % (g.get("persist"), [b.get("name") for b in bro if b.get("persists")]))
    item("it plays with touch and has a sound", "touch" in caps.get("input", []) and bool(g.get("sounds")), "input %s, %d sound(s)" % (caps.get("input"), len(g.get("sounds", {}))))
    st = status(engine, gid + ".game2d.json", d)
    item("native scenarios pass", st.get("native", {}).get("ok") is True, st.get("native", {}).get("summary", ""))
    item("the package is current with the game file", st.get("package", {}).get("current") is True, "package %s" % st.get("package", {}).get("package_id"), needs_browser=True)
    br = st.get("browser", {})
    item("the browser record is for this package and passed", br.get("for_this_package") is True and br.get("ok") is True, json.dumps({k: br.get(k) for k in ("for_this_package", "ok", "browser")}), needs_browser=True)
    ev = br.get("evidence", {})
    item("no piece of evidence failed", ev.get("failed", 1) == 0 and ev.get("failed_pieces") == [], "evidence %s" % json.dumps({k: ev.get(k) for k in ("passed", "failed", "not_run", "not_applicable")}), needs_browser=True)
    site = os.path.join(d, "out/site/games", gid, "game.json")
    rec = json.load(open(site)) if os.path.isfile(site) else {}
    builds = rec.get("builds", [])
    item("published to the local site, and the record is for the current build", st.get("publication", {}).get("is_this_build") is True, "publication %s" % json.dumps({k: st.get("publication", {}).get(k) for k in ("exists", "is_this_build")}), needs_browser=True)
    item("the site holds two distinct builds (before and after the change)", len({b.get("build_id") for b in builds}) >= 2, "%d build(s): %s" % (len(builds), [b.get("build_id") for b in builds]), needs_browser=True)
    if a.before:
        before = json.load(open(a.before))
        b0, b1 = before.get("vars", {}).get("timeleft"), g.get("vars", {}).get("timeleft")
        item("the snapshot taken before the change had the 25 second countdown", b0 == 25, "before: timeleft %s" % b0)
        item("the requested change is made: the countdown starts at 15", b1 == 15, "after: timeleft %s" % b1)
    else:
        item("the requested change is made: the countdown starts at 15", g.get("vars", {}).get("timeleft") == 15, "timeleft %s" % g.get("vars", {}).get("timeleft"))
    return finish(items)


def finish(items):
    bad = [i for i in items if i["status"] == "FAIL"]
    for i in items:
        print("%-7s %s: %s" % (i["status"].upper() if i["status"] != "pass" else "pass", i["name"], i["detail"]))
    print(json.dumps({"scorecard": {"passed": sum(1 for i in items if i["status"] == "pass"), "failed": len(bad), "skipped": sum(1 for i in items if i["status"] == "skipped")}}))
    sys.exit(1 if bad else 0)


# ---- friction summary -----------------------------------------------------------------------------------------------------------------------------

def summarise(a):
    rows = [json.loads(l) for l in open(a.trace) if l.strip()]
    docs, searches, src, failures, repairs, retries = [], [], 0, [], 0, 0
    last_fail = {}
    for r in rows:
        argv = r["argv"]
        cmd = argv[0] if argv else ""
        if cmd == "describe":
            docs.append(next((x for x in argv[1:] if not x.startswith("-")), "overview"))
        if cmd == "search":
            searches.append(" ".join(x for x in argv[1:] if not x.startswith("-")))
        if cmd in ("src", "context"):
            src += 1
        key = (cmd, tuple(x for x in argv[1:3] if not x.startswith("-")))
        if r["exit"] != 0:
            failures.append({"command": " ".join(argv[:4]), "error": r.get("error", "")})
            if key in last_fail:
                retries += 1
            last_fail[key] = True
        elif key in last_fail:
            repairs += 1  # the same command failed before and now passes: an edit fixed it
            del last_fail[key]
    out = {
        "commands": len(rows),
        "failed": len(failures),
        "documentation_topics_read": docs,
        "searches": searches,
        "cli_source_exploration": src,
        "retries_of_a_failed_command_without_a_success_between": retries,
        "repair_cycles": repairs,
        "first_failures": failures[:5],
        "used_web_status": any(r["argv"][:2] == ["web", "status"] for r in rows),
        "minutes": round((rows[-1]["t"] - rows[0]["t"]) / 60.0, 1) if rows else 0,
    }
    if a.transcript:
        out["transcript"] = transcript_summary(a.transcript)
    print(json.dumps(out, indent=1))


ENGINE_HINTS = ("/src/", "/crates/", "SPEC.md", "AGENTS.md", "AGENT_REFERENCE", "/docs/", "/tests/")


def transcript_summary(path):
    reads, shell, edits = [], [], 0
    for line in open(path):
        try:
            ev = json.loads(line)
        except ValueError:
            continue
        for blk in (ev.get("message", {}).get("content", []) if isinstance(ev.get("message"), dict) else []):
            if not isinstance(blk, dict) or blk.get("type") != "tool_use":
                continue
            name, inp = blk.get("name"), blk.get("input", {})
            target = inp.get("file_path") or inp.get("path") or inp.get("pattern") or ""
            if name in ("Read", "Grep", "Glob") and any(h in target for h in ENGINE_HINTS):
                reads.append(target)
            elif name == "Bash":
                cmd = inp.get("command", "")
                if any(w in cmd for w in ("cat ", "sed -n", "grep ", "head ", "less ", "rg ")) and any(h in cmd for h in ENGINE_HINTS):
                    shell.append(cmd[:120])
            elif name in ("Edit", "Write"):
                edits += 1
    return {"engine_files_read_with_tools": len(reads), "engine_files_read_with_the_shell": len(shell), "examples": (reads + shell)[:6], "edits": edits}


# ---- reference agent -----------------------------------------------------------------------------------------------------------------------------

def reference(a):
    engine = engine_path(a)
    work = os.path.abspath(a.workdir)
    shutil.rmtree(work, ignore_errors=True)
    os.makedirs(work)
    trace = os.path.join(work, "trace.jsonl")
    env = {"RED_TRACE": trace}
    game_dir = os.path.join(work, "star-dash")
    g = "star-dash.game2d.json"

    def step(args, cwd=work, must=True):
        code, out = run(engine, args, cwd, env)
        print("$ red_engine2 %s -> %d" % (" ".join(args), code))
        if must and code != 0:
            print(out[-1500:])
            sys.exit("reference agent: `%s` failed" % " ".join(args))
        return out

    # 1. discover: the brief, then the one page it points to. (Nothing else is read.)
    brief = step(["describe", "--brief"])
    assert "describe web" in brief, "the brief must point to `describe web`"
    page = step(["describe", "web"])
    for needed in ("new-game DIR --kind 2d", "validate G -> verify G -> web verify G -> publish G", "web status G"):
        assert needed in page, "describe web must say `%s`" % needed
    # 2. a verified starter, then the edits the task asks for, in the file.
    step(["new-game", "star-dash", "--kind", "2d", "--name", "star-dash"])
    path = os.path.join(game_dir, g)
    d = json.load(open(path))
    d["id"], d["title"] = "star-dash", "Star Dash"
    d["description"] = "Collect six stars before the clock runs out; the best result is remembered."
    d["vars"]["timeleft"] = 25
    d["vars"]["gems_left"] = 6
    d["scene"].append({"prefab": "gem", "at": [160, 150]})
    d["prefabs"]["hazard"] = {"tag": "hazard", "shape": {"circle": 6, "color": "#ff4d6d"}, "move": {"patrol": {"axis": "x", "range": 60, "speed": 40}}}
    d["scene"].append({"prefab": "hazard", "at": [160, 110], "id": "h"})
    d["rules"].append({"id": "hazard", "when": {"touch": ["player", "hazard"]}, "do": [{"end": "lose"}]})
    json.dump(d, open(path, "w"), indent=1)
    step(["validate", g], cwd=game_dir)
    step(["sim", g, "--only", "walking"], cwd=game_dir)
    step(["verify", g], cwd=game_dir)
    if not a.no_browser:
        step(["web", "verify", g], cwd=game_dir)
        step(["publish", g], cwd=game_dir)
    shutil.copy(path, os.path.join(game_dir, "before.game2d.json"))
    # 3. the change.
    d = json.load(open(path))
    d["vars"]["timeleft"] = 15
    json.dump(d, open(path, "w"), indent=1)
    step(["verify", g], cwd=game_dir)
    if not a.no_browser:
        step(["web", "verify", g], cwd=game_dir)
        step(["publish", g], cwd=game_dir)
    out = step(["web", "status", g], cwd=game_dir)
    print(out)
    # 4. score it, and say how much friction it had.
    args = ["score", game_dir, "--engine", engine, "--before", os.path.join(game_dir, "before.game2d.json")] + (["--no-browser"] if a.no_browser else [])
    code = subprocess.call([sys.executable, os.path.abspath(__file__)] + args)
    subprocess.call([sys.executable, os.path.abspath(__file__), "summary", trace])
    sys.exit(code)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("score")
    s.add_argument("dir")
    s.add_argument("--engine")
    s.add_argument("--before")
    s.add_argument("--no-browser", action="store_true")
    s.set_defaults(f=score)
    m = sub.add_parser("summary")
    m.add_argument("trace")
    m.add_argument("--transcript")
    m.set_defaults(f=summarise)
    r = sub.add_parser("reference")
    r.add_argument("workdir")
    r.add_argument("--engine")
    r.add_argument("--no-browser", action="store_true")
    r.set_defaults(f=reference)
    a = ap.parse_args()
    a.f(a)


if __name__ == "__main__":
    main()
