#!/usr/bin/env python3
"""The AI launchpad: one first command that picks the workflow, names the executable and gives ONE next action.

    scripts/dev start "<task>" [--project DIR] [--target native|headless|multiplayer ...] [--workflow W] [--json] [--no-save]
    scripts/dev next    [--project DIR]     the current next action again (recomputed from what is on disk)
    scripts/dev resume  [--project DIR]     what changed since you stopped, which recorded results still apply, and the next action
    (in a game project: scripts/red start|next|resume; MCP: the `start` tool)

Read-only and standard-library only: it never compiles, installs, downloads, or runs verification, so it works on a fresh checkout with no binary. The only
processes it starts are `git` and, when `scripts/red_resolve.py` finds a FRESH engine executable, the read-only CLI commands `propose`, `capabilities` and
`context` (their answers are the authority for what can be built; nothing here keeps a second capability list). `start` records a compact task file under
`out/launchpad/` (ignored by git; `--no-save` skips it) so `resume` can say what changed. Agent-written notes (`--note`) and results observed by tools are kept
apart, and recorded results are never trusted across an edit: the tool that wrote them (`affected`, `game check`, `verify`) stays the authority.

Workflows: game-create, game-change, engine-change, diagnose, upgrade. A task that matches none is reported as `unrouted` (an incomplete launchpad route,
not an unsupported engine capability) with the choices; `--workflow` forces one.
"""
import argparse
import fnmatch
import hashlib
import json
import os
import re
import shlex
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import red_resolve  # noqa: E402

SCHEMA = "red-launchpad/1"
TASK_SCHEMA = "red-launchpad-task/1"
WORKFLOWS = ("game-create", "game-change", "engine-change", "diagnose", "upgrade")
TARGETS = ("native", "headless", "multiplayer")
# The only engine commands this script will ever run: they read files and print, they do not build, write, download or verify.
READ_ONLY_CLI = ("propose", "capabilities", "context")
STOP = set("a an the to and or of for in on with that this it is be make build create new small simple i want me my please can you your game games".split())
INVOKED = []


# ----------------------------------------------------------------------------------------------- small helpers
def now_utc():
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def words(text):
    return [w for w in re.findall(r"[a-z0-9_]+", text.lower()) if len(w) >= 3 and w not in STOP]


def slug(text, fallback="mygame"):
    ws = words(text)[:3]
    return "-".join(ws) if ws else fallback


def sha(data):
    return hashlib.sha256(data).hexdigest()


def file_sha(path):
    try:
        with open(path, "rb") as f:
            return sha(f.read())
    except OSError:
        return None


def cli(exe, args, timeout=60):
    """Run an allowed read-only engine command; returns (exit, stdout, stderr). Records the call (`invoked`)."""
    if not args or args[0] not in READ_ONLY_CLI:
        raise ValueError(f"launchpad never runs `{args[0] if args else ''}`")
    t0 = time.time()
    try:
        p = subprocess.run([exe, *args], capture_output=True, text=True, timeout=timeout)
        code, out, err = p.returncode, p.stdout, p.stderr
    except (OSError, subprocess.SubprocessError) as e:
        code, out, err = 127, "", str(e)
    INVOKED.append({"argv": [os.path.basename(exe), *args], "ms": int((time.time() - t0) * 1000), "exit": code})
    return code, out, err


def cli_json(exe, args):
    code, out, _ = cli(exe, [*args, "--json"])
    try:
        env = json.loads(out)
    except ValueError:
        return None
    return env.get("data") if isinstance(env, dict) and env.get("ok") else None


def glob_re(pattern):
    """The features.json glob (`*` within a segment, `**` across segments, a trailing `/` a directory) as a regex."""
    if pattern.endswith("/"):
        pattern += "**"
    out, i = "", 0
    while i < len(pattern):
        if pattern.startswith("**/", i):
            out += "(?:.*/)?"
            i += 3
        elif pattern.startswith("**", i):
            out += ".*"
            i += 2
        elif pattern[i] == "*":
            out += "[^/]*"
            i += 1
        else:
            out += re.escape(pattern[i])
            i += 1
    return re.compile("^" + out + "$")


# ----------------------------------------------------------------------------------------------- routing
CUES = {
    "upgrade": [(r"\bupgrad\w*", 4), (r"\bmigrat\w*", 4), (r"\bengine (version|revision|commit|pin)", 4), (r"\bbump\b.*\bengine\b", 3), (r"\brepin\b", 4)],
    "diagnose": [(r"\bwhy\b", 2), (r"\b(fail|fails|failed|failing)\b", 3), (r"\bbroken\b", 3), (r"\bcrash\w*", 3), (r"\berrors?\b", 2), (r"\bnot working\b", 3),
                 (r"\bdoctor\b", 3), (r"\bdiagnos\w*", 4), (r"\bstuck\b", 2), (r"\bwon'?t (build|start|run)\b", 3)],
    "engine-change": [(r"\bthe engine\b", 2), (r"\bengine (feature|code|bug|change)\b", 4), (r"\bsrc/", 4), (r"\.rs\b", 4), (r"\brust\b", 3), (r"\brefactor\w*", 3),
                      (r"\bcrate\b", 2), (r"\bcli command\b", 3), (r"\bprotocol\b", 2), (r"\bshaders?\b", 3), (r"\badr\b", 2), (r"\bsimulation\b", 2),
                      (r"\bimplement\b.*\b(in|for) the engine\b", 3), (r"\bnew (cli )?command\b", 3), (r"\bpublic api\b", 3)],
    "game-create": [(r"\b(make|create|build|write|prototype|start)\b.*\bgame\b", 4), (r"\bnew game\b", 4), (r"\ba game (where|about|that|with|in which)\b", 3),
                    (r"\b(platformer|shooter|kart|racer|racing|puzzle|arcade|runner|roguelike|sokoban|tower defen[cs]e|top.?down|metroidvania|maze)\b", 2)],
    "game-change": [(r"\b(add|change|fix|tweak|update|improve|remove|rename|balance|make the)\b", 2), (r"\b(level|map|blueprint|enemy|enemies|score|rule|rules|room|door|spawn)s?\b", 2)],
}


def classify(task, project_is_game, forced, feature_hits):
    """-> (workflow id, confidence, reasons, alternatives). Every cue that fired is named, so a wrong route is easy to see and override."""
    if forced:
        return forced, "high", [f"--workflow {forced} given"], []
    score, why = {}, {}
    for wf, cues in CUES.items():
        for pat, w in cues:
            m = re.search(pat, task.lower())
            if m:
                score[wf] = score.get(wf, 0) + w
                why.setdefault(wf, []).append(f"“{m.group(0)}” suggests {wf}")
    if feature_hits:
        score["engine-change"] = score.get("engine-change", 0) + 3
        why.setdefault("engine-change", []).append("names engine feature/file: " + ", ".join(feature_hits[:3]))
    if project_is_game:
        score["game-change"] = score.get("game-change", 0) + 3
        why.setdefault("game-change", []).append("--project is an existing game project (game.json)")
        if "game-create" in score:
            score["game-create"] -= 2
    score = {k: v for k, v in score.items() if v > 0}
    if not score:
        return "unrouted", "none", ["no workflow cue in the task"], [{"id": w, "why": "rerun with --workflow " + w} for w in WORKFLOWS]
    ranked = sorted(score.items(), key=lambda kv: (-kv[1], kv[0]))
    top, top_score = ranked[0]
    second = ranked[1] if len(ranked) > 1 else None
    close = second is not None and second[1] >= top_score - 1
    conf = "low" if close else ("high" if top_score >= 4 else "medium")
    alts = [{"id": wf, "why": "; ".join(why.get(wf, [])[:2])} for wf, _ in ranked[1:3]]
    return top, conf, why[top][:3], alts


def infer_constraints(task, targets):
    t = task.lower()
    inferred = []
    got = list(targets)

    def add(x, because):
        if x not in got:
            got.append(x)
            inferred.append(f"{x}: {because}")

    if re.search(r"\b(browser|web|webgl|wasm|online page|itch)\b", t):
        add("native", "the task mentions the browser, but every game is a native executable (windows, linux): there is no browser target")
    if re.search(r"\b(multiplayer|online|co-?op|versus|vs|lan|server)\b", t):
        add("multiplayer", "the task mentions multiplayer")
    if re.search(r"\b(windows|linux|desktop|native|window)\b", t):
        add("native", "the task mentions a native window")
    if re.search(r"\b(headless|server only|no graphics|ci)\b", t):
        add("headless", "the task mentions headless/CI")
    presentation = "3d" if re.search(r"\b(3d|first.?person|fps|shooter|walk around|rooms)\b", t) else ("2d" if re.search(r"\b2d\b|platformer|top.?down|puzzle|arcade|runner|sokoban", t) else None)
    kind = "race" if re.search(r"\b(kart|racer|racing|race|laps)\b", t) else None
    return {"targets": got, "inferred": inferred, "presentation": presentation, "kind": kind}


# ----------------------------------------------------------------------------------------------- engine knowledge from the repository
def load_features(root):
    try:
        with open(os.path.join(root, "docs", "features.json"), encoding="utf-8") as f:
            return json.load(f).get("features", {})
    except (OSError, ValueError):
        return {}


def feature_owners(features, task):
    """The features a task most likely concerns, from docs/features.json (the same index `context`, `impact` and `affected` use)."""
    ws = set(words(task))
    paths = re.findall(r"[\w./-]+\.(?:rs|json|md|py|sh|toml|wgsl|js)\b|(?:src|tests|crates|docs|scripts)/[\w./-]*", task)
    hits = []
    for name, f in features.items():
        s, why = 0, []
        for w in ws:
            if w in name.lower().split("_"):
                s += 3
                why.append(f"name has `{w}`")
            elif re.search(rf"\b{re.escape(w)}", f.get("summary", "").lower()):
                s += 1
        for p in paths:
            if any(glob_re(g).match(p.lstrip("./")) for g in f.get("files", [])):
                s += 6
                why.append(f"owns {p}")
        if s:
            hits.append((s, name, why))
    hits.sort(key=lambda h: (-h[0], h[1]))
    # Weak word matches ("hit" in "hitscan") are noise next to a feature that owns the named file: keep what scores near the best.
    hits = [h for h in hits if h[0] >= 0.4 * hits[0][0]] if hits else hits
    return [{"feature": n, "score": s, "because": w[:3], "summary": features[n].get("summary", "")[:160], "tests": features[n].get("tests", [])[:8],
             "verify_commands": features[n].get("commands", [])[:3], "docs": features[n].get("docs", [])[:4], "depends_on": features[n].get("depends_on", [])[:6]}
            for s, n, w in hits[:3]]


# ----------------------------------------------------------------------------------------------- identity (what a stop point was made from)
def git_changed_files(root):
    out = red_resolve._git(root, "status", "--porcelain")
    files = []
    for line in (out or "").splitlines():
        p = line[3:].strip().split(" -> ")[-1]
        if p:
            files.append(p)
    return files


def engine_inputs(root):
    """Identity of an engine checkout's working tree: HEAD plus a content hash of every changed or untracked file (bounded)."""
    head = red_resolve._git(root, "rev-parse", "HEAD")
    changed = git_changed_files(root)[:400]
    per = {}
    for rel in changed:
        p = os.path.join(root, rel)
        if os.path.isfile(p) and os.path.getsize(p) < 4_000_000:
            per[rel] = file_sha(p)
        else:
            per[rel] = None
    return {"head": head, "files": per, "hash": sha(json.dumps([head, sorted(per.items())], sort_keys=True).encode())}


def project_inputs(project):
    """Identity of a game project's inputs: game.json, blueprints, maps, 2D game files and the project's own scripts (bounded, by content)."""
    per = {}
    for pattern in ("game.json", "blueprints/*.json", "maps/*.json", "*.game2d.json", "assets/*.json", "scripts/red", "scripts/red.ps1"):
        d, _, base = pattern.rpartition("/")
        folder = os.path.join(project, d) if d else project
        try:
            for fn in sorted(os.listdir(folder)):
                if fnmatch.fnmatch(fn, base):
                    rel = (d + "/" if d else "") + fn
                    per[rel] = file_sha(os.path.join(project, rel))
        except OSError:
            pass
    return {"files": per, "hash": sha(json.dumps(sorted(per.items())).encode())}


def verification_config(root):
    """The files whose content decides what verification means for an engine checkout (changing one invalidates what was recorded)."""
    files = ("docs/features.json", "scripts/ci.sh", "Cargo.lock", "rustfmt.toml", "scripts/dev")
    per = {f: file_sha(os.path.join(root, f)) for f in files}
    return {"files": per, "hash": sha(json.dumps(sorted(per.items())).encode())}


def exe_identity(resolution):
    sel = resolution["selected"]
    cand = next((c for c in resolution["candidates"] if c.get("path") in (sel.get("exe"), sel.get("stale_exe"))), None)
    return {
        "status": sel["status"], "path": sel.get("exe") or sel.get("stale_exe"),
        "modified_utc": (cand or {}).get("modified_utc"), "size_bytes": (cand or {}).get("size_bytes"),
        "features": (cand or {}).get("features"), "built_revision": (cand or {}).get("built_revision"),
        "profile": resolution["profile"], "target_dir": resolution["target_dir"],
    }


# ----------------------------------------------------------------------------------------------- recorded results (read, never trusted across edits)
def newest_mtime(paths):
    best = (0.0, None)
    for p in paths:
        try:
            m = os.stat(p).st_mtime
        except OSError:
            continue
        if m > best[0]:
            best = (m, p)
    return best


def recorded_results(root, project, workflow):
    """What existing tools left on disk. Each says what it is, when, and whether any input is newer; none is called passed: the tool that wrote it decides."""
    out = []
    if workflow in ("engine-change", "diagnose") or not project:
        stamp = os.path.join(root, "out", ".affected-green.json")
        if os.path.isfile(stamp):
            changed = [os.path.join(root, f) for f in git_changed_files(root)]
            newest = newest_mtime(changed)
            m = os.stat(stamp).st_mtime
            out.append({
                "claim": "full-or-affected verification", "record": "out/.affected-green.json", "recorded_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(m)),
                "state": "unverified",
                "reason": ("a changed file is newer than the record (" + os.path.relpath(newest[1], root) + ")") if newest[0] > m else
                          "only `scripts/dev affected` can say whether this green still applies (it keys on file content, base commit, features and toolchain)",
                "authority": ["scripts/dev", "affected"]})
    if project:
        cache = [os.path.join(project, "out", "cache", f) for f in os.listdir(os.path.join(project, "out", "cache"))] if os.path.isdir(os.path.join(project, "out", "cache")) else []
        if cache:
            m = newest_mtime(cache)
            inputs = [os.path.join(project, rel) for rel in project_inputs(project)["files"]]
            newer = newest_mtime(inputs)
            out.append({
                "claim": "validation + behavior (map checks)", "record": "out/cache/check-*", "recorded_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(m[0])),
                "state": "unverified",
                "reason": ("an input is newer than the newest record (" + os.path.relpath(newer[1], project) + ")") if newer[0] > m[0] else
                          "`scripts/red check` re-verifies only the maps whose bytes changed: run it, it is cheap",
                "authority": ["scripts/red", "check"]})
    return out


# ----------------------------------------------------------------------------------------------- plans
def dev_argv(root, *args):
    return ["powershell", "-File", "scripts\\dev.ps1", *args] if os.name == "nt" else ["scripts/dev", *args]


def game2d_file(project):
    """The `<name>.game2d.json` of a 2D project, else None. A 2D project is verified with `verify` (and `play2d` for a window); `scripts/red check` is the walk-project check
    (on a 2D starter it reports `server.map '' is not one of the project's maps`), so 2D routes must never recommend it."""
    try:
        return next((f for f in sorted(os.listdir(project)) if f.endswith(".game2d.json")), None)
    except OSError:
        return None


def action(summary, cwd, argv, success, compiles=False, network=False, after=None, kind="run"):
    return {"summary": summary, "kind": kind, "cwd": cwd, "argv": argv, "success": success, "compiles": compiles, "network": network, "then": after}


def plan_game_create(ctx):
    """New small game: propose -> starter -> checks. Capabilities come from the engine's own `propose`; with no fresh binary they are reported unchecked."""
    res, exe, task, cons = ctx["resolution"], ctx["exe"], ctx["task"], ctx["constraints"]
    root = res["engine"]["root"]
    name = slug(task)
    target_dir = ctx["project_arg"] or os.path.join(os.path.dirname(root), name)
    kind = cons["kind"] or ("walk" if cons["presentation"] == "3d" else "2d")
    plan = {"project_dir": target_dir, "kind": kind, "name": os.path.basename(target_dir.rstrip("/")) or name}
    caps = {"source": "red_engine2 propose", "checked": False, "supported": None, "requested": {"presentation": cons["presentation"], "kind": kind, "targets": cons["targets"]},
            "problems": [], "warnings": [], "limitations": []}
    pointers = [
        {"what": "the 2D game format on one page (~8 KB)" if kind == "2d" else "game rules as data", "argv": ctx["red"] + (["describe", "2d"] if kind == "2d" else ["describe", "rules"])},
        {"what": "capability matrix: presentation x platform x networking x input", "argv": ctx["red"] + ["capabilities"], "also": "docs/PLAY_2D.md"},
        {"what": "how this project works (agent entry point)", "file": "AGENTS.md", "section": "Making a game: blueprints and game projects"}]
    blockers, uncertainty, missing = [], [], []
    if exe:
        args = ["propose", *task.split()]
        if cons["presentation"]:
            args += ["--presentation", cons["presentation"]]
        for t in cons["targets"]:
            if t in ("windows", "linux"):
                args += ["--platform", t]
            elif t == "native":
                args += ["--platform", "windows", "--platform", "linux"]
        if "multiplayer" in cons["targets"]:
            args += ["--networking", "authoritative"]
        data = cli_json(exe, args)
        if data:
            caps.update(checked=True, supported=bool(data.get("buildable")), problems=data.get("problems", []), warnings=data.get("warnings", []),
                        proposal={k: data.get(k) for k in ("title", "genre", "capabilities", "complexity", "session_minutes")}, reasons=data.get("reasons", [])[:3])
            pc = (data.get("capabilities") or {})
            if pc.get("presentation") == "3d" and kind == "2d":
                kind = "walk"
                plan["kind"] = kind
            if pc.get("presentation") == "2d":
                kind = "2d"
                plan["kind"] = kind
        else:
            uncertainty.append("`propose` did not return a plan; capabilities are unchecked")
    else:
        missing.append("engine CLI: no fresh executable, so `propose` and `capabilities` could not be asked")
        uncertainty.append(f"capabilities unchecked: the presentation/kind ({kind}) is inferred from words, not from `propose`; check `capabilities` before promising a feature")
    if caps["supported"] is False:
        blockers.append({"id": "unsupported-capability", "detail": caps["problems"] or ["`propose` says this is not buildable as asked"],
                         "route": "do not drop the requested feature: extend the engine (workflow engine-change) or choose a supported combination listed by `capabilities`",
                         "extension": {"argv": ctx["self"] + ["start", "extend the engine so that: " + task, "--workflow", "engine-change"]}})
    rel_engine = os.path.relpath(root, target_dir)
    loops = loop_commands(kind, plan["name"], target_dir, cons)
    for group in loops.values():
        for c in group:
            c["cwd"] = target_dir
    if blockers:
        nxt = action("The requested combination is not supported by the engine: decide between a supported combination and an engine extension (see blockers)", ctx["cwd"],
                     ctx["red"] + ["capabilities"] if exe else ctx["self"] + ["start", "--help"], "the matrix names the supported neighbours", kind="decide")
    elif exe and os.path.isfile(os.path.join(target_dir, "game.json")):
        g2 = game2d_file(target_dir)
        nxt = action("The project already exists here: continue it", target_dir, ["scripts/red", "verify", g2] if g2 else ["scripts/red", "check"], "exit 0 (the project's own checks pass)", kind="run")
    elif exe:
        nxt = action(f"Create the starter ({kind}) pinned to this engine; it is green from its first commit", root,
                     [exe, "new-game", target_dir, "--kind", kind, "--name", plan["name"], "--engine-path", rel_engine],
                     f"exit 0 and {os.path.join(target_dir, 'game.json')} exists; then run `{'scripts/red verify ' + plan['name'] + '.game2d.json' if kind == '2d' else 'scripts/red check'}` there",
                     after={"argv": ["scripts/red", "verify", f"{plan['name']}.game2d.json"] if kind == "2d" else ["scripts/red", "check"], "cwd": target_dir})
    else:
        nb = res.get("next_build") or {}
        nxt = action("Build the engine CLI once (the starter, `propose` and `capabilities` all need it)", nb.get("cwd", root), nb.get("argv", dev_argv(root, "red", "describe", "--brief")),
                     nb.get("success", "exit 0"), compiles=True, after={"argv": ctx["self"] + ["start", task], "cwd": ctx["cwd"], "why": "re-run start: it will then ask `propose`"})
    return {"plan": plan, "capabilities": caps, "context": {"pointers": pointers}, "next_action": nxt, "blockers": blockers, "uncertainty": uncertainty,
            "missing": missing, **loops}


def loop_commands(kind, name, project, cons):
    """Iteration checks and final requirements, as the starters document them, split into the separate claims a result can support (never merged)."""
    g = f"{name}.game2d.json"
    if kind == "2d":
        it = [{"when": "after every edit", "claim": "validation", "argv": ["scripts/red", "validate", g]}, {"when": "after a rule or level change", "claim": "behavior", "argv": ["scripts/red", "sim", g]}]
        final = [{"claim": "validation", "argv": ["scripts/red", "validate", g], "required": True}, {"claim": "behavior", "argv": ["scripts/red", "verify", g], "required": True},
                 {"claim": "visual/input inspection", "argv": ["scripts/red", "frame", g, "out/look.png"], "required": True, "note": "open the picture: only looking proves it looks right"},
                 {"claim": "target execution (native window)", "argv": ["scripts/red", "play2d", g, "--max-ticks", "60"], "required": "native" in cons["targets"], "note": "needs a window (a display); `verify` already ran the same simulation headless"},
                 {"claim": "networking", "argv": None, "required": False, "note": "2D games have no networked play"}]
    else:
        it = [{"when": "after every edit", "claim": "validation", "argv": ["scripts/red", "check"]}]
        final = [{"claim": "validation + behavior", "argv": ["scripts/red", "check"], "required": True},
                 {"claim": "visual/input inspection", "argv": ["scripts/red", "plan", "maps/main.json"], "required": True, "note": "open the plan image"},
                 {"claim": "target execution (native)", "argv": ["scripts/red", "play-local"], "required": "native" in cons["targets"], "note": "needs a window: a person (or `playtest` for pictures)"},
                 {"claim": "networking", "argv": ["scripts/red", "serve"] if kind != "race" else ["scripts/red", "race-test", "maps/main.json"], "required": "multiplayer" in cons["targets"]}]
    return {"iteration_checks": it, "final_requirements": final}


def plan_engine_change(ctx):
    """Focused engine change: owning features, the context packet, then the iterate -> affected ladder. The ladder is CLAUDE.md's; escalation is `affected`'s."""
    res, exe, task = ctx["resolution"], ctx["exe"], ctx["task"]
    root = res["engine"]["root"]
    owners = feature_owners(load_features(root), task)
    uncertainty, missing, blockers = [], [], []
    if not owners:
        uncertainty.append("no feature in docs/features.json matches the task words: name a file or a feature (`scripts/dev context <words>` searches)")
    primary = owners[0]["feature"] if owners else None
    packet = {"owners": owners, "full_packet": {"argv": ctx["dev"] + ["context", primary or "<feature|file|words>"], "approx_bytes": "5-15 KB"}}
    if re.search(r"\b(bug|fail\w*|broken|regress\w*|wrong|crash\w*)\b", task.lower()):
        packet["reproduce_first"] = "write the failing test (or run the owning feature's verify command) before the fix: " + "; ".join((owners[0]["verify_commands"] if owners else [])[:2])
    checks = [{"when": "after every edit (seconds; never verification)", "claim": "type-check + focused unit tests", "argv": ctx["dev"] + ["iterate"]},
              {"when": "after a meaningful step", "claim": "owning features' integration suites", "argv": ctx["dev"] + ["affected", "--quick"]}]
    final = [{"claim": "affected verification (owners + dependents)", "argv": ctx["dev"] + ["affected"], "required": True, "note": "before you say done; escalates to the full run by itself for Cargo.*, src/lib.rs beyond `mod` lines, CI files"},
             {"claim": "full CI", "argv": ctx["dev"] + ["affected", "--full"], "required": True, "note": "before pushing"},
             {"claim": "repository bookkeeping", "argv": ctx["dev"] + ["preflight"], "required": True, "note": "ADR for a decision, `///` on pub items"}]
    if exe:
        nxt = action("Read the work packet for the owning feature (files, public API, tests, ADRs)", root, [exe, "context", *( [primary] if primary else task.split())],
                     "prints a 5-15 KB packet; then make the change and run `iterate`")
    else:
        nb = res.get("next_build") or {}
        missing.append("engine CLI (only for `context`/`describe`; editing and `iterate` need just cargo)")
        nxt = action("Build the engine CLI once, then read the work packet (or skip the CLI: the owners above are from docs/features.json, and `scripts/dev iterate` builds only what it checks)",
                     nb.get("cwd", root), nb.get("argv", dev_argv(root, "red", "describe", "--brief")), nb.get("success", "exit 0"), compiles=True,
                     after={"argv": ctx["dev"] + ["context", primary or "<words>"], "cwd": root})
    return {"plan": {"owners": [o["feature"] for o in owners]}, "capabilities": {"source": "docs/features.json (feature ownership)", "checked": bool(owners), "supported": None,
            "note": "an engine change has no capability to check; whether it is possible is decided by the owning feature's tests"},
            "context": {"packet": packet, "pointers": [{"what": "the workflow and ladder", "file": "CLAUDE.md", "section": "Cheap by default"}]},
            "next_action": nxt, "blockers": blockers, "uncertainty": uncertainty, "missing": missing, "iteration_checks": checks, "final_requirements": final}


def plan_delegated(ctx, workflow):
    """game-change, diagnose and upgrade: routed to the existing tool, not re-implemented here."""
    res, exe = ctx["resolution"], ctx["exe"]
    proj = ctx["project"]
    root = res["engine"]["root"]
    uncertainty = ["this route only delegates: it names the established tool and does not plan the work"]
    missing = [] if exe else ["engine CLI: no fresh executable"]
    if workflow == "game-change":
        base = ["scripts/red"] if proj else ctx["red"]
        g2 = game2d_file(proj) if proj else None
        nxt = action("Re-check the project as it is before changing it", proj or ctx["cwd"], base + (["verify", g2] if g2 else ["check"]),
                     "exit 0 (the game's scenarios, picture and sound pass)" if g2 else "exit 0 (cheap: only changed maps are re-verified)")
        pointers = [{"what": "edit the blueprint, then build and check", "file": "CLAUDE.md (in the project)", "section": "the loop"}, {"what": "rules as data", "argv": ctx["red"] + ["describe", "rules"]}]
        loops = loop_commands("walk", os.path.basename(proj or "game"), proj, ctx["constraints"]) if proj and not any(f.endswith(".game2d.json") for f in os.listdir(proj)) else \
            loop_commands("2d", (next((f[:-len(".game2d.json")] for f in os.listdir(proj) if f.endswith(".game2d.json")), "game")), proj, ctx["constraints"]) if proj else {"iteration_checks": [], "final_requirements": []}
    elif workflow == "upgrade":
        nxt = action("Plan the engine upgrade (read-only; writes a packet, never edits the project)", proj or ctx["cwd"], (["scripts/red"] if proj else ctx["red"]) + ["game", "upgrade", "plan"],
                     "a plan packet naming the target commit, the evidence and the verification stages")
        pointers = [{"what": "staged verification of the plan", "argv": ["scripts/red", "game", "upgrade", "verify", "<packet.json>"]}]
        loops = {"iteration_checks": [], "final_requirements": [{"claim": "staged upgrade verification", "argv": ["scripts/red", "game", "upgrade", "verify", "<packet.json>"], "required": True}]}
    else:
        nxt = action("Ask the machine what it can do and what is missing", root, ctx["dev"] + ["doctor"], "a list of ok/warn lines; fix the first `warn`/missing line")
        pointers = [{"what": "the engine's own probe (builds the CLI)", "argv": ctx["dev"] + ["doctor-full"]}, {"what": "resume facts: git, STATUS.md", "argv": ctx["red"] + ["status"]}]
        loops = {"iteration_checks": [], "final_requirements": []}
    return {"plan": {}, "capabilities": {"source": "not applicable", "checked": False, "supported": None}, "context": {"pointers": pointers}, "next_action": nxt,
            "blockers": [], "uncertainty": uncertainty, "missing": missing, **loops}


def plan_unrouted(ctx):
    return {"plan": {}, "capabilities": {"source": "not applicable", "checked": False, "supported": None},
            "context": {"pointers": [{"what": "the agent entry point", "file": "AGENTS.md"}]},
            "next_action": action("The task did not match a workflow: choose one and rerun (this is an incomplete route, not an unsupported engine capability)", ctx["cwd"],
                                  ctx["self"] + ["start", ctx["task"], "--workflow", "<" + "|".join(WORKFLOWS) + ">"], "the launchpad prints a plan for the chosen workflow", kind="decide"),
            "blockers": [{"id": "unrouted", "detail": ["no workflow cue matched"]}], "uncertainty": [], "missing": [], "iteration_checks": [], "final_requirements": []}


# ----------------------------------------------------------------------------------------------- state
def task_dir(base):
    return os.path.join(base, "out", "launchpad")


def load_task(base, task_id=None):
    d = task_dir(base)
    if not os.path.isdir(d):
        return None
    files = sorted(f for f in os.listdir(d) if f.endswith(".json"))
    if task_id:
        files = [f for f in files if f == task_id + ".json"]
    if not files:
        return None
    # newest by modification time, not by name
    files.sort(key=lambda f: os.path.getmtime(os.path.join(d, f)))
    try:
        with open(os.path.join(d, files[-1]), encoding="utf-8") as fh:
            t = json.load(fh)
        t["_file"] = os.path.join(d, files[-1])
        return t
    except (OSError, ValueError):
        return None


def save_task(base, t):
    d = task_dir(base)
    os.makedirs(d, exist_ok=True)
    path = os.path.join(d, t["id"] + ".json")
    t = {k: v for k, v in t.items() if not k.startswith("_")}
    with open(path, "w", encoding="utf-8") as f:
        json.dump(t, f, indent=1, sort_keys=True)
    return path


def diff_identity(then, now):
    """What changed between two identities, by name. Anything listed invalidates the results recorded under the old one."""
    changes = []
    if then.get("engine_head") != now.get("engine_head"):
        changes.append({"what": "engine revision", "then": (then.get("engine_head") or "")[:10], "now": (now.get("engine_head") or "")[:10]})
    a, b = then.get("inputs", {}).get("files", {}), now.get("inputs", {}).get("files", {})
    edited = sorted(k for k in set(a) | set(b) if a.get(k) != b.get(k))
    if edited:
        changes.append({"what": "source or game inputs", "files": edited[:20], "count": len(edited)})
    ea, eb = then.get("executable", {}), now.get("executable", {})
    if any(ea.get(k) != eb.get(k) for k in ("path", "modified_utc", "size_bytes", "features", "status")):
        changes.append({"what": "executable identity", "then": {k: ea.get(k) for k in ("status", "modified_utc", "features")}, "now": {k: eb.get(k) for k in ("status", "modified_utc", "features")}})
    va, vb = then.get("verification_config", {}).get("files", {}), now.get("verification_config", {}).get("files", {})
    cfg = sorted(k for k in set(va) | set(vb) if va.get(k) != vb.get(k))
    if cfg:
        changes.append({"what": "verification configuration", "files": cfg})
    return changes


# ----------------------------------------------------------------------------------------------- assembling the response
def build_context(args, task):
    here = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    project_arg = os.path.abspath(args.project) if args.project else None
    is_game = bool(project_arg and os.path.isfile(os.path.join(project_arg, "game.json")))
    resolution = red_resolve.resolve(here, project_arg if is_game else None)
    root = resolution["engine"]["root"] or here
    exe = resolution["selected"]["exe"] if resolution["selected"]["status"] in ("ready",) else None
    prefix = ["scripts/red"] if is_game else dev_argv(root, "red")
    return {"here": here, "root": root, "resolution": resolution, "exe": exe, "task": task, "project_arg": project_arg, "project": project_arg if is_game else None,
            "is_game": is_game, "cwd": project_arg if is_game else root, "dev": dev_argv(root), "red": prefix,
            "self": (["scripts/red"] if is_game else dev_argv(root))}


def respond(args, command, task, state=None):
    ctx = build_context(args, task)
    ctx["constraints"] = infer_constraints(task, args.target or [])
    features = load_features(ctx["root"])
    feat_hits = [o["feature"] for o in feature_owners(features, task) if o["score"] >= 6]
    wf, conf, reasons, alts = classify(task, ctx["is_game"], args.workflow, feat_hits)
    if wf == "game-create":
        body = plan_game_create(ctx)
    elif wf == "engine-change":
        body = plan_engine_change(ctx)
    elif wf in ("game-change", "diagnose", "upgrade"):
        body = plan_delegated(ctx, wf)
    else:
        body = plan_unrouted(ctx)
    res = ctx["resolution"]
    sel = res["selected"]
    inputs = project_inputs(ctx["project"]) if ctx["project"] else engine_inputs(ctx["root"])
    ident = {"engine_head": res["engine"].get("head"), "engine_branch": res["engine"].get("branch"), "engine_dirty_files": res["engine"].get("dirty_files"),
             "project": ctx["project"], "inputs": {"hash": inputs["hash"], "files": inputs["files"]}, "verification_config": verification_config(ctx["root"]),
             "executable": exe_identity(res)}
    prereq = [{"id": "engine-cli", "state": {"ready": "ok", "uncertain": "uncertain", "stale": "stale", "missing": "missing"}[sel["status"]], "detail": "; ".join(sel["reasons"] + res["notes"]),
               "fix": res.get("next_build")}]
    if not red_resolve._git(ctx["root"], "rev-parse", "HEAD"):
        prereq.append({"id": "git", "state": "missing", "detail": "the engine checkout is not a git repository: revision and change tracking are unavailable"})
    out = {
        "schema": SCHEMA, "command": command, "read_only": True, "generated_utc": now_utc(),
        "objective": task, "constraints": {"targets": ctx["constraints"]["targets"], "inferred": ctx["constraints"]["inferred"], "workflow_forced": bool(args.workflow),
                                           "project": ctx["project_arg"]},
        "workflow": {"id": wf, "confidence": conf, "reasons": reasons, "alternatives": alts},
        "identity": {k: ident[k] for k in ("engine_head", "engine_branch", "engine_dirty_files", "project")} | {"executable": ident["executable"], "inputs_hash": ident["inputs"]["hash"]},
        "capabilities": body["capabilities"], "prerequisites": prereq, "missing": body["missing"], "uncertainty": body["uncertainty"] + ([] if sel["status"] == "ready" else ["executable: " + sel["status"]]),
        "context": body["context"], "next_action": body["next_action"], "iteration_checks": body["iteration_checks"], "final_requirements": body["final_requirements"],
        "evidence": {"observed": recorded_results(ctx["root"], ctx["project"], wf), "claims": claim_table(body["final_requirements"]), "agent_notes": []},
        "blockers": body["blockers"], "plan": body["plan"],
    }
    return out, ctx, ident


def claim_table(final):
    """Every required claim starts `planned`: planned, passed, failed, skipped and unverified are different states and nothing here marks a claim passed."""
    return [{"claim": f["claim"], "required": f.get("required"), "state": "planned" if f.get("argv") else "not_applicable"} for f in final]


def new_task_id(task):
    return f"{time.strftime('%Y%m%d-%H%M%S', time.gmtime())}-{slug(task, 'task')}"


def cmd_start(args):
    task = " ".join(args.task).strip()
    if not task:
        print("start needs a task: scripts/dev start \"make a small 2d coin game\"", file=sys.stderr)
        return 2
    out, ctx, ident = respond(args, "start", task)
    base = ctx["project"] or ctx["root"]
    out["invoked"] = list(INVOKED)
    if not args.no_save and os.path.isdir(base):
        tid = new_task_id(task)
        t = {"schema": TASK_SCHEMA, "id": tid, "created_utc": now_utc(), "objective": task, "targets": args.target or [], "workflow_forced": args.workflow,
             "workflow": out["workflow"]["id"], "reasons": out["workflow"]["reasons"], "identity": ident, "agent_notes": [], "observed_log": [{"utc": now_utc(), "at": "start", "executable": ident["executable"]["status"]}],
             "next_action": out["next_action"], "project_arg": ctx["project_arg"]}
        try:
            out["task"] = {"id": tid, "file": os.path.relpath(save_task(base, t), base)}
        except OSError as e:
            out["task"] = {"id": None, "error": str(e)}
    emit(args, out)
    return 0


def cmd_resume_or_next(args, command):
    base = os.path.abspath(args.project) if args.project else os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    t = load_task(base, args.task_id)
    if not t:
        print(f"{command}: no saved task under {task_dir(base)}. Start one: scripts/dev start \"<task>\"", file=sys.stderr)
        return 2
    args.workflow = t.get("workflow_forced")
    args.target = t.get("targets") or []
    args.project = t.get("project_arg") or args.project
    out, ctx, ident = respond(args, command, t["objective"])
    changes = diff_identity(t["identity"], ident)
    for note in args.note or []:
        t.setdefault("agent_notes", []).append({"utc": now_utc(), "text": note})
    t["agent_notes"] = t.get("agent_notes", [])
    t.setdefault("observed_log", []).append({"utc": now_utc(), "at": command, "executable": ident["executable"]["status"], "changed": [c["what"] for c in changes]})
    t["observed_log"] = t["observed_log"][-20:]
    t["next_action"] = out["next_action"]
    try:
        save_task(base, t)
    except OSError:
        pass
    out["task"] = {"id": t["id"], "started_utc": t["created_utc"], "file": os.path.relpath(t["_file"], base)}
    out["since_start"] = {"changes": changes, "recorded_results_trusted": not changes,
                          "note": "anything listed in changes invalidates results recorded before it; re-run the check named in `evidence.observed[].authority`" if changes else
                                  "inputs, executable and verification configuration are as at start; recorded results still need their own tool to say they apply"}
    if changes:
        for o in out["evidence"]["observed"]:
            o["state"] = "stale"
            o["reason"] = "inputs or executable changed since the task started: " + ", ".join(c["what"] for c in changes)
        for c in out["evidence"]["claims"]:
            if c["state"] == "planned":
                c["state"] = "unverified"
    out["evidence"]["agent_notes"] = [{"by": "agent", **n} for n in t.get("agent_notes", [])]
    out["invoked"] = list(INVOKED)
    emit(args, out)
    return 0


# ----------------------------------------------------------------------------------------------- output
def emit(args, out):
    if args.json:
        print(json.dumps(out, indent=1))
        return
    print(render(out))


def cmdline(a):
    return shlex.join(a["argv"]) if a.get("argv") else "(no command)"


def render(o):
    w, i, na, sel = o["workflow"], o["identity"], o["next_action"], o["identity"]["executable"]
    L = [f"launchpad {o['command']}" + (f"  task {o['task']['id']}" if o.get("task", {}).get("id") else "  (not saved)"),
         f"objective  {o['objective']}",
         f"workflow   {w['id']} ({w['confidence']}): " + "; ".join(w["reasons"])]
    if w["alternatives"]:
        L.append("  or       " + " | ".join(f"{a['id']}" for a in w["alternatives"]))
    if o["constraints"]["targets"]:
        L.append("targets    " + ", ".join(o["constraints"]["targets"]) + (f"  (inferred: {'; '.join(o['constraints']['inferred'])})" if o["constraints"]["inferred"] else ""))
    L.append(f"engine     {(i.get('engine_head') or 'not a git checkout')[:10]} on {i.get('engine_branch')}, {i.get('engine_dirty_files')} uncommitted" + (f"   project {i['project']}" if i.get("project") else ""))
    L.append(f"executable {sel['status'].upper()}: {sel.get('path') or 'none'}" + (f" ({sel['features']} build)" if sel.get("features") not in (None, "unknown") else ""))
    for p in o["prerequisites"]:
        if p["state"] != "ok":
            L.append(f"  - {p['id']} {p['state']}: {p['detail']}")
    c = o["capabilities"]
    L.append(f"capability {'CHECKED' if c.get('checked') else 'unchecked'} via {c.get('source')}" + ("" if c.get("supported") is None else f": {'supported' if c['supported'] else 'NOT SUPPORTED'}"))
    for p in c.get("problems", [])[:3]:
        L.append(f"  - problem: {p}")
    for b in o["blockers"]:
        L.append(f"BLOCKER    {b['id']}: {'; '.join(b['detail'])}" + (f"\n  route    {b['route']}" if b.get("route") else ""))
    for u in o["uncertainty"][:3]:
        L.append(f"uncertain  {u}")
    pk = o["context"].get("packet")
    if pk and pk.get("owners"):
        for ow in pk["owners"][:3]:
            L.append(f"owner      {ow['feature']}: {ow['summary'][:90]}  tests: {', '.join(ow['tests'][:4])}")
        if pk.get("reproduce_first"):
            L.append(f"reproduce  {pk['reproduce_first']}")
    for p in o["context"].get("pointers", [])[:3]:
        L.append(f"read       {p['what']}: " + (shlex.join(p["argv"]) if p.get("argv") else f"{p.get('file')} {p.get('section', '')}"))
    L.append(f"NEXT       (in {na['cwd']}) {cmdline(na)}")
    L.append(f"  {na['summary']}")
    L.append(f"  success: {na['success']}" + ("   [compiles]" if na.get("compiles") else "") + ("   [network]" if na.get("network") else ""))
    if na.get("then"):
        L.append(f"  then: {cmdline(na['then'])}" + (f"  ({na['then']['why']})" if na["then"].get("why") else ""))
    if o["iteration_checks"]:
        L.append("iterate    " + " ; ".join(f"{shlex.join(c['argv']) if c['argv'] else '-'}  [{c['claim']}]" for c in o["iteration_checks"][:3]))
    if o["final_requirements"]:
        L.append("before done:")
        for f in o["final_requirements"]:
            L.append(f"  {'REQUIRED' if f.get('required') else 'optional':<8} {f['claim']}: {shlex.join(f['argv']) if f.get('argv') else f.get('note', '')}" + (f"   [{f['note']}]" if f.get('argv') and f.get('note') else ""))
    if o.get("since_start"):
        s = o["since_start"]
        L.append("since start: " + ("; ".join(c["what"] for c in s["changes"]) if s["changes"] else "nothing changed (inputs, executable, verification config)"))
    for ob in o["evidence"]["observed"]:
        L.append(f"recorded   {ob['claim']}: {ob['state'].upper()} ({ob['record']}, {ob['recorded_utc']}) {ob['reason']}")
    for n in o["evidence"]["agent_notes"]:
        L.append(f"agent note {n['utc']}: {n['text']}")
    L.append(f"(read-only; builds triggered: 0; engine commands run: {len(o.get('invoked', []))})")
    return "\n".join(L)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0], formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="command", required=True)
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument("--project", help="a game project directory; for `start` a directory that does not exist yet is the place a new game will be created")
    common.add_argument("--target", action="append", choices=TARGETS, help="a requested target (repeatable)")
    common.add_argument("--workflow", choices=WORKFLOWS, help="force a workflow instead of routing from the task words")
    common.add_argument("--json", action="store_true", help=f"the `{SCHEMA}` document instead of text")
    s = sub.add_parser("start", parents=[common], help="pick the workflow and give one next action")
    s.add_argument("task", nargs="*")
    s.add_argument("--no-save", action="store_true", help="do not write out/launchpad/<task>.json")
    for name in ("next", "resume"):
        r = sub.add_parser(name, parents=[common])
        r.add_argument("--task-id")
        r.add_argument("--note", action="append", help="an agent-written note (kept apart from observed results)")
    a = ap.parse_args(argv)
    if a.command == "start":
        return cmd_start(a)
    return cmd_resume_or_next(a, a.command)


if __name__ == "__main__":
    sys.exit(main())
