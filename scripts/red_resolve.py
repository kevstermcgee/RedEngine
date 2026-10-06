#!/usr/bin/env python3
"""Which `red_engine2` would run, and can it be trusted? One resolver for every entry point.

Read-only and standard-library only: it never builds, installs, downloads or runs the engine, so it works on a fresh checkout with no compiled
binary. `scripts/dev`, the generated `scripts/red`, the MCP adapter (`mcp_server.py`) and `scripts/launchpad.py` use these rules, so they cannot disagree.

    python3 scripts/red_resolve.py [--engine DIR] [--project DIR] [--bin red_engine2] [--json]

Rules (the same ones `scripts/dev` and `scripts/red` already follow):
  engine checkout   RED_ENGINE (override) > the project's game.json pin (`engine.path`, or the `.red/engine` clone of `engine.git`) > this checkout
  target directory  CARGO_TARGET_DIR, else <engine>/target
  profile           RED_PROFILE, else `debug` (the profile the dev workflow builds); other profiles are listed, never preferred
  explicit exe      RED_ENGINE_EXE (used as given; its freshness is still reported)
A candidate is `stale` when a build input (Cargo.toml, Cargo.lock, build.rs, src/, assets/, crates/) is newer than the file: a stale binary is reported,
never selected. A prebuilt install (RED_PREFIX/bin, default ~/.local/bin) records no revision, so it is `uncertain`: used only when the project
pins nothing, and always reported as such. Exit code: 0 ready or uncertain, 1 nothing usable (JSON on stdout either way with --json).
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
import time

SCHEMA = "red-engine-exe/1"
PROFILES = ("debug", "release", "fast")
INPUT_FILES = ("Cargo.toml", "Cargo.lock", "build.rs")
INPUT_DIRS = ("src", "assets", "crates")
NOT_INPUT_SUFFIXES = (".md", ".py", ".txt")


def exe_name(binary="red_engine2"):
    return binary + (".exe" if os.name == "nt" else "")


def _git(root, *args):
    try:
        out = subprocess.run(["git", "-C", root, *args], capture_output=True, text=True, timeout=10)
    except (OSError, subprocess.SubprocessError):
        return None
    return out.stdout.strip() if out.returncode == 0 else None


def git_state(root):
    """HEAD, branch and the number of uncommitted files of a checkout (None where git cannot say)."""
    head = _git(root, "rev-parse", "HEAD")
    if head is None:
        return {"head": None, "branch": None, "dirty_files": None}
    porcelain = _git(root, "status", "--porcelain")
    return {
        "head": head,
        "branch": _git(root, "rev-parse", "--abbrev-ref", "HEAD"),
        "dirty_files": None if porcelain is None else len([line for line in porcelain.splitlines() if line.strip()]),
    }


def read_pin(project):
    """The project's engine pin from game.json: {"path"|"git", "ref"} or None."""
    try:
        with open(os.path.join(project, "game.json"), encoding="utf-8") as f:
            engine = json.load(f).get("engine")
    except (OSError, ValueError):
        return None
    return engine if isinstance(engine, dict) else None


def newest_input(engine_root):
    """(mtime, relative path) of the newest build input of an engine checkout, or (0, None)."""
    best = (0.0, None)

    def consider(path):
        nonlocal best
        try:
            m = os.stat(path).st_mtime
        except OSError:
            return
        if m > best[0]:
            best = (m, os.path.relpath(path, engine_root).replace(os.sep, "/"))

    for name in INPUT_FILES:
        consider(os.path.join(engine_root, name))
    for d in INPUT_DIRS:
        for dirpath, dirnames, filenames in os.walk(os.path.join(engine_root, d)):
            dirnames[:] = [x for x in dirnames if x not in ("target", ".git", "node_modules", "__pycache__")]
            for fn in filenames:
                if not fn.endswith(NOT_INPUT_SUFFIXES):
                    consider(os.path.join(dirpath, fn))
    return best


def _read_text(path):
    try:
        with open(path, encoding="utf-8") as f:
            return f.read().strip()
    except OSError:
        return None


def _build_facts(exe_path, stem):
    """Build mode and revision evidence recorded next to a binary by the build scripts (None where nothing was recorded)."""
    d = os.path.dirname(exe_path)
    modes = []
    for prefix, who in ((".red-dev-", "scripts/dev"), (".red-wrapper-", "scripts/red")):
        p = os.path.join(d, f"{prefix}{stem}.mode")
        text = _read_text(p)
        if text in ("default", "headless"):
            modes.append((os.stat(p).st_mtime, text, who))
    mode = max(modes) if modes else None
    info = None
    text = _read_text(os.path.join(d, f".red-build-{stem}.json"))
    if text:
        try:
            info = json.loads(text)
        except ValueError:
            info = None
    return {
        "features": mode[1] if mode else (info or {}).get("mode", "unknown"),
        "features_evidence": mode[2] + " build stamp" if mode else ("build record" if info and info.get("mode") else None),
        "built_revision": (info or {}).get("rev"),
        "built_dirty_files": (info or {}).get("dirty_files"),
    }


def _candidate(source, path, profile, newest, binary):
    exists = os.path.isfile(path)
    c = {"source": source, "path": path, "profile": profile, "exists": exists}
    if not exists:
        return c
    st = os.stat(path)
    c["modified_utc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(st.st_mtime))
    c["size_bytes"] = st.st_size
    c.update(_build_facts(path, binary))
    if newest[1] is None:
        c["fresh"] = None
        c["stale_because"] = None
    else:
        c["fresh"] = st.st_mtime >= newest[0]
        c["stale_because"] = None if c["fresh"] else f"{newest[1]} is newer than the binary"
    return c


def _engine_root(project, env, default_root):
    """(engine root or None, how it was chosen, pin, notes)."""
    notes = []
    pin = read_pin(project) if project else None
    pinned_root = None
    if pin and pin.get("path"):
        pinned_root = os.path.normpath(os.path.join(project, pin["path"]))
    elif pin and pin.get("git"):
        clone = os.path.join(project, ".red", "engine")
        pinned_root = clone if os.path.isfile(os.path.join(clone, "Cargo.toml")) else None
    override = env.get("RED_ENGINE")
    if override:
        root = os.path.abspath(override)
        if pinned_root and os.path.realpath(root) != os.path.realpath(pinned_root):
            notes.append(f"RED_ENGINE ({root}) overrides the project's pinned engine ({pinned_root}); compatibility with the pin is not checked")
        return root, "RED_ENGINE override", pin, notes
    if pin:
        if pinned_root:
            return pinned_root, "project pin", pin, notes
        return None, "project pin (engine not present)", pin, notes
    if project:
        notes.append("the project has no engine pin in game.json; using this engine checkout")
    return default_root, "this engine checkout", pin, notes


def resolve(engine_root=None, project=None, binary="red_engine2", env=None):
    """Resolve the executable for `binary`. Returns the `red-engine-exe/1` dict (see the module doc)."""
    env = os.environ if env is None else env
    default_root = os.path.abspath(engine_root) if engine_root else os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    project = os.path.abspath(project) if project else None
    root, how, pin, notes = _engine_root(project, env, default_root)
    profile = env.get("RED_PROFILE") or "debug"
    out = {
        "schema": SCHEMA,
        "binary": binary,
        "engine": {"root": root, "chosen_by": how, "pin": pin},
        "profile": profile,
        "target_dir": None,
        "candidates": [],
        "selected": {"status": "missing", "exe": None, "reasons": []},
        "notes": notes,
    }
    reasons = out["selected"]["reasons"]
    if root is None:
        reasons.append("the project pins an engine that is not present yet (the first `scripts/red` command clones it from the pin, which needs the network)")
        out["next_build"] = _build_command(None, project)
        return out
    if not os.path.isfile(os.path.join(root, "Cargo.toml")):
        reasons.append(f"no engine checkout at {root} (no Cargo.toml)")
        out["next_build"] = _build_command(None, project)
        return out
    out["engine"].update(git_state(root))
    target = env.get("CARGO_TARGET_DIR") or os.path.join(root, "target")
    out["target_dir"] = target
    if pin and pin.get("ref") and out["engine"].get("head"):
        want = _git(root, "rev-parse", "--verify", "--quiet", pin["ref"] + "^{commit}")
        if want is None:
            notes.append(f"the pinned ref `{pin['ref']}` is not known to this checkout (fetch it; not checked)")
        elif want != out["engine"]["head"]:
            notes.append(f"the engine checkout is at {out['engine']['head'][:10]} but the project pins {pin['ref']} ({want[:10]}): not the pinned revision")
            out["engine"]["matches_pin"] = False
        else:
            out["engine"]["matches_pin"] = True
    newest = newest_input(root)
    exe = exe_name(binary)
    cands = out["candidates"]
    explicit = env.get("RED_ENGINE_EXE")
    if explicit:
        cands.append(_candidate("RED_ENGINE_EXE", os.path.abspath(explicit), None, newest, binary))
    primary = _candidate("target-dir", os.path.join(target, profile, exe), profile, newest, binary)
    cands.append(primary)
    for p in PROFILES:
        if p != profile and os.path.isfile(os.path.join(target, p, exe)):
            cands.append(_candidate("target-dir (other profile, not selected)", os.path.join(target, p, exe), p, newest, binary))
    prefix = env.get("RED_PREFIX") or os.path.join(os.path.expanduser("~"), ".local")
    installed = os.path.join(prefix, "bin", exe)
    if os.path.isfile(installed):
        cands.append(_candidate("installed (prebuilt)", installed, None, newest, binary))
    on_path = shutil.which(exe, path=env.get("PATH"))
    if on_path and os.path.realpath(on_path) not in {os.path.realpath(c["path"]) for c in cands}:
        cands.append(_candidate("installed (on PATH)", on_path, None, newest, binary))
    sel = out["selected"]
    pinned = bool(pin)
    if explicit:
        c = cands[0]
        if c["exists"]:
            sel.update(status="uncertain", exe=c["path"])
            reasons.append("RED_ENGINE_EXE is used as given; it is not checked against the project pin" + ("" if c.get("fresh") in (True, None) else f" and it is stale ({c['stale_because']})"))
        else:
            reasons.append(f"RED_ENGINE_EXE points at {c['path']}, which does not exist")
    elif primary["exists"] and primary.get("fresh") is not False:
        sel.update(status="ready", exe=primary["path"])
        reasons.append(f"{primary['source']} {profile} binary is newer than every build input")
        if primary.get("features") == "unknown":
            reasons.append("its features (graphics or headless) are unrecorded: it was not built by scripts/dev or scripts/red")
        if out["engine"].get("matches_pin") is False:
            sel["status"] = "uncertain"
            reasons.append("the checkout is not at the pinned revision")
    elif primary["exists"]:
        sel.update(status="stale", exe=None, stale_exe=primary["path"])
        reasons.append(f"the {profile} binary is stale: {primary['stale_because']}; rebuilding is the next step, a stale binary is never selected")
    else:
        reasons.append(f"no {profile} binary under {target} (nothing has been built here)")
    if sel["exe"] is None and sel["status"] in ("missing", "stale"):
        inst = next((c for c in cands if c["source"].startswith("installed")), None)
        if inst:
            if pinned:
                reasons.append(f"a prebuilt install exists ({inst['path']}) but the project pins an engine and the install records no revision: not selected")
            else:
                sel.update(status="uncertain", exe=inst["path"])
                reasons.append(f"using the prebuilt install {inst['path']}: it records no revision, so compatibility with this checkout is uncertain")
    if sel["exe"] is None:
        out["next_build"] = _build_command(root, project)
    return out


def _build_command(root, project):
    """The command that makes a usable binary (it compiles: it is a recommendation, never run by the resolver)."""
    if project:
        argv = ["scripts/red", "describe", "--brief"]
        if os.name == "nt":
            argv = ["powershell", "-File", "scripts\\red.ps1", "describe", "--brief"]
        return {"cwd": project, "argv": argv, "compiles": True, "network": "only when the pinned engine is not cloned yet",
                "success": "exit 0 and the ~1 KB manual on stdout"}
    argv = ["scripts/dev", "red", "describe", "--brief"]
    if os.name == "nt":
        argv = ["powershell", "-File", "scripts\\dev.ps1", "red", "describe", "--brief"]
    return {"cwd": root, "argv": argv, "compiles": True, "network": "no",
            "success": "exit 0 and the ~1 KB manual on stdout (builds the CLI on first use; set RED_PROFILE=fast for an optimised build)"}


def require(engine_root=None, project=None, binary="red_engine2", env=None):
    """The path of a usable executable, or RuntimeError saying exactly why not and what to run."""
    r = resolve(engine_root, project, binary, env)
    if r["selected"]["exe"]:
        return r["selected"]["exe"]
    nb = r.get("next_build")
    cmd = " ".join(nb["argv"]) if nb else "scripts/dev red describe --brief"
    raise RuntimeError(f"{binary}: no usable executable ({'; '.join(r['selected']['reasons'])}). Build it: `{cmd}` in {nb['cwd'] if nb else '<engine checkout>'}")


def render(r):
    sel = r["selected"]
    lines = [f"engine   {r['engine']['root']} ({r['engine']['chosen_by']})"]
    if r["engine"].get("head"):
        dirty = r["engine"].get("dirty_files")
        lines.append(f"revision {r['engine']['head'][:10]} on {r['engine'].get('branch')}, {'unknown' if dirty is None else dirty} uncommitted file(s)")
    lines.append(f"target   {r['target_dir']}  profile {r['profile']}")
    for c in r["candidates"]:
        state = "missing" if not c["exists"] else ("stale" if c.get("fresh") is False else "fresh" if c.get("fresh") else "freshness unknown")
        extra = f", {c['features']}" if c.get("features") not in (None, "unknown") else ""
        lines.append(f"  {state:<8} {c['path']} [{c['source']}{extra}]")
    lines.append(f"selected {sel['status']}: {sel['exe'] or '(none)'}")
    lines += [f"  - {x}" for x in sel["reasons"] + r["notes"]]
    if r.get("next_build"):
        nb = r["next_build"]
        lines.append(f"next     (in {nb['cwd']}) {' '.join(nb['argv'])}")
    return "\n".join(lines)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--engine", help="engine checkout (default: the one this script is in)")
    ap.add_argument("--project", help="a game project directory (reads its game.json pin)")
    ap.add_argument("--bin", default="red_engine2")
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args(argv)
    r = resolve(a.engine, a.project, a.bin)
    print(json.dumps(r, indent=2) if a.json else render(r))
    return 0 if r["selected"]["exe"] else 1


if __name__ == "__main__":
    sys.exit(main())
