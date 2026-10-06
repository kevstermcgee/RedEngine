#!/usr/bin/env python3
"""Measure the launchpad on five real situations and append one run to benches/history/launchpad.json.

    python3 scripts/launchpad_measure.py --target-dir DIR [--label NAME] [--no-record] [--skip engine-change]

It records what happened, not what was hoped: wall time, output size, the commands the launchpad itself ran, and the builds triggered (a `cargo` shim logs every
call; the real cargo is behind it). It makes NO claim about model tokens, faster completion or smaller-model success: those need fresh-agent trials
(bench/fresh-agent/). Scenarios: fresh checkout (a clean clone, no target directory), warm checkout (this tree, a fresh binary), a small game-authoring task
carried through the launchpad's own next actions, a focused engine change (context packet, one real edit, `iterate`), an interrupted and resumed task.
Needs a fresh `red_engine2` in --target-dir for the warm scenarios (`scripts/dev red describe --brief` builds it).
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
HISTORY = os.path.join(REPO, "benches", "history", "launchpad.json")
TASK = "make a small 2d coin game for the browser"
ENGINE_TASK = "fix a bug in src/tools/search.rs where a hit is ranked wrongly"


def timed(argv, cwd, env, timeout=1500):
    t0 = time.time()
    p = subprocess.run(argv, cwd=cwd, env=env, capture_output=True, text=True, timeout=timeout)
    return {"argv": argv, "cwd": cwd, "exit": p.returncode, "seconds": round(time.time() - t0, 2), "stdout_bytes": len(p.stdout.encode()), "stderr_bytes": len(p.stderr.encode())}, p


def artifacts_since(target_dir, t0):
    """How many build products were written under a target directory since t0 (binaries, dependency artifacts, fingerprints). A `cargo` on the PATH can be bypassed
    (the scripts source ~/.cargo/env), so what is counted is what a build leaves on disk: 0 means nothing was compiled."""
    n = 0
    for sub in ("debug", "debug/deps", "debug/.fingerprint", "debug/build"):
        d = os.path.join(target_dir, sub)
        try:
            for e in os.scandir(d):
                if e.stat().st_mtime > t0 and not e.name.startswith(".red-"):
                    n += 1
        except OSError:
            pass
    return n


def launch(cwd, env, *extra, script="scripts/dev"):
    r1, p = timed(["bash", os.path.join(cwd, script), "start", TASK, "--no-save", *extra], cwd, env)
    r2, q = timed(["bash", os.path.join(cwd, script), "start", TASK, "--no-save", "--json", *extra], cwd, env)
    doc = json.loads(q.stdout)
    return {"text_seconds": r1["seconds"], "text_bytes": r1["stdout_bytes"], "json_seconds": r2["seconds"], "json_bytes": r2["stdout_bytes"], "exit": [r1["exit"], r2["exit"]],
            "workflow": doc["workflow"]["id"], "executable": doc["identity"]["executable"]["status"], "capabilities_checked": doc["capabilities"]["checked"],
            "engine_commands_run": [c["argv"][:2] for c in doc.get("invoked", [])], "next_action": doc["next_action"]["summary"], "next_compiles": doc["next_action"]["compiles"]}, doc


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--target-dir", required=True)
    ap.add_argument("--label", default="run")
    ap.add_argument("--no-record", action="store_true")
    ap.add_argument("--skip", action="append", default=[])
    a = ap.parse_args()
    tmp = tempfile.mkdtemp(prefix="re2_lpm_")
    # HOME stays: it holds the Rust toolchain. RED_PREFIX hides any prebuilt install from the resolver instead.
    base_env = {**os.environ, "RED_PREFIX": os.path.join(tmp, "prefix")}
    base_env.pop("CARGO_TARGET_DIR", None)
    run = {"label": a.label, "engine_commit": subprocess.run(["git", "-C", REPO, "rev-parse", "--short", "HEAD"], capture_output=True, text=True).stdout.strip(),
           "machine": "4-core Intel N97, 15 GB (the dev box)", "scenarios": {}}

    # 1. fresh checkout: a clean clone of HEAD, no target directory anywhere.
    if "fresh" not in a.skip:
        clone = os.path.join(tmp, "fresh")
        t0 = time.time()
        subprocess.run(["git", "clone", "-q", "--local", REPO, clone], check=True)
        t1 = time.time()
        m, doc = launch(clone, base_env)
        m.update(clone_seconds=round(t1 - t0, 2), target_dir_created=os.path.isdir(os.path.join(clone, "target")), build_artifacts_written=0 if not os.path.isdir(os.path.join(clone, "target")) else artifacts_since(os.path.join(clone, "target"), t1))
        d, _ = timed(["bash", os.path.join(clone, "scripts/dev"), "doctor"], clone, base_env)
        m["reference_scripts_dev_doctor"] = {"seconds": d["seconds"], "bytes": d["stdout_bytes"]}
        run["scenarios"]["fresh_checkout"] = m

    # 2. warm checkout: this tree with a fresh binary in --target-dir.
    env = {**base_env, "CARGO_TARGET_DIR": a.target_dir}
    t1 = time.time()
    m, doc = launch(REPO, env)
    m["build_artifacts_written"] = artifacts_since(a.target_dir, t1)
    run["scenarios"]["warm_checkout"] = m
    exe = doc["identity"]["executable"]["path"]

    # 3. a small game, carried through the launchpad's own next actions.
    proj = os.path.join(tmp, "coin-browser")
    steps = []
    if "game" not in a.skip and doc["next_action"]["argv"][1:2] == ["new-game"]:
        argv = [exe, "new-game", proj, *doc["next_action"]["argv"][3:]]
        argv[argv.index("--engine-path") + 1] = os.path.relpath(REPO, proj)
        t1 = time.time()
        r, p = timed(argv, REPO, env)
        steps.append({"step": "new-game (the launchpad's next action)", **{k: r[k] for k in ("seconds", "exit", "stdout_bytes")}})
        game = "coin-browser.game2d.json"
        for claim, cmd in (("validation", ["validate", game]), ("behavior", ["verify", game]), ("visual/input inspection", ["frame", game, "out/look.png"]),
                           ("target execution (browser)", ["web", "verify", game])):
            r, p = timed(["bash", os.path.join(proj, "scripts", "red"), *cmd], proj, env, timeout=900)
            steps.append({"step": " ".join(cmd), "claim": claim, **{k: r[k] for k in ("seconds", "exit", "stdout_bytes")}, "last_line": (p.stdout.strip().splitlines() or [""])[-1][:160], "stderr_tail": p.stderr.strip()[-200:] if r["exit"] else ""})
        run["scenarios"]["game_authoring"] = {"steps": steps, "build_artifacts_written": artifacts_since(a.target_dir, t1), "total_seconds": round(sum(s["seconds"] for s in steps), 1),
                                              "outcome": "all steps exit 0" if all(s["exit"] == 0 for s in steps) else "a step failed: see steps"}
        # 5. interrupted and resumed: save a task in the project, change an input, resume.
        # `publish` (local backend: no network) leaves out/publish/<game>/publication.json, the existing record `web status` reads: a recorded result for resume to find.
        c_, cp = timed(["bash", os.path.join(proj, "scripts", "red"), "publish", game], proj, env, timeout=900)
        s_, _ = timed(["bash", os.path.join(proj, "scripts", "red"), "start", "make the coins worth more", "--json"], proj, env)
        r_idle, p_idle = timed(["bash", os.path.join(proj, "scripts", "red"), "resume", "--json"], proj, env)
        with open(os.path.join(proj, game)) as f:
            text = f.read()
        with open(os.path.join(proj, game), "w") as f:
            f.write(text.replace('"name"', '"name"', 1) + "\n")
        r_changed, p_changed = timed(["bash", os.path.join(proj, "scripts", "red"), "resume", "--json"], proj, env)
        rd = json.loads(p_changed.stdout)
        run["scenarios"]["interrupted_resumed"] = {
            "project_check": {"exit": c_["exit"], "seconds": c_["seconds"]}, "start_seconds": s_["seconds"], "resume_unchanged": {"seconds": r_idle["seconds"], "bytes": r_idle["stdout_bytes"], "changes": json.loads(p_idle.stdout)["since_start"]["changes"], "recorded_results": [o["state"] for o in json.loads(p_idle.stdout)["evidence"]["observed"]]},
            "resume_after_edit": {"seconds": r_changed["seconds"], "bytes": r_changed["stdout_bytes"], "changes": [c["what"] for c in rd["since_start"]["changes"]],
                                  "recorded_results": [o["state"] for o in rd["evidence"]["observed"]], "next": rd["next_action"]["summary"]}}

    # 4. a focused engine change: the launchpad, the packet, one real edit, `iterate`, then undo the edit.
    if "engine-change" not in a.skip:
        r, p = timed(["bash", os.path.join(REPO, "scripts/dev"), "start", ENGINE_TASK, "--no-save", "--json"], REPO, env)
        doc = json.loads(p.stdout)
        m = {"start_seconds": r["seconds"], "start_bytes": r["stdout_bytes"], "workflow": doc["workflow"]["id"], "owners": [o["feature"] for o in doc["context"]["packet"]["owners"]]}
        ctx, cp = timed(doc["next_action"]["argv"], REPO, env)
        m["context_packet"] = {"seconds": ctx["seconds"], "bytes": ctx["stdout_bytes"], "exit": ctx["exit"]}
        target = os.path.join(REPO, "src", "tools", "search.rs")
        original = open(target).read()
        stat = os.stat(target)
        t2 = time.time()
        try:
            with open(target, "w") as f:
                f.write(original.replace("/// A scored search result with the best-matching fragment.", "/// A scored search result with the best-matching fragment (launchpad measurement edit).", 1))
            it, ip = timed(["bash", os.path.join(REPO, "scripts/dev"), "iterate"], REPO, env, timeout=1500)
            m["iterate_after_one_doc_edit"] = {"seconds": it["seconds"], "exit": it["exit"], "last_line": (ip.stdout.strip().splitlines() or [""])[-1][:160]}
        finally:
            with open(target, "w") as f:
                f.write(original)
            os.utime(target, (stat.st_atime, stat.st_mtime))  # the edit is undone: restore the timestamp so the binary is not left stale
        m["build_artifacts_written_by_iterate"] = artifacts_since(a.target_dir, t2)
        run["scenarios"]["engine_change"] = m
    run["note"] = "no model was involved: these are the launchpad's own costs. Token savings, completion time and smaller-model success are NOT measured here."
    print(json.dumps(run, indent=1))
    if not a.no_record:
        try:
            hist = json.load(open(HISTORY))
        except (OSError, ValueError):
            hist = {"schema": "red-launchpad-measure/1", "purpose": "What the AI launchpad itself costs on a real checkout: start time, output size, commands run, builds triggered. Append a new run; never edit an old one. No token or model-success claims.", "runs": []}
        hist["runs"].append(run)
        os.makedirs(os.path.dirname(HISTORY), exist_ok=True)
        json.dump(hist, open(HISTORY, "w"), indent=1)
    shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
