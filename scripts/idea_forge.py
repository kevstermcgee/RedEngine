#!/usr/bin/env python3
"""Idea Forge: forge a game idea, have an AI build it with RedEngine, and file the engine feedback that run produced (docs/IDEA_FORGE.md).

  idea_forge.py idea [--n N] [--code C] [--no-story] [--json]     the next idea nobody has been given yet (or the one behind CODE)
  idea_forge.py brief [--code C]                                   the prompt an agent gets for an idea
  idea_forge.py run [--code C] [--model M] [--budget USD]          forge an idea, make a worktree, run a headless `claude -p` agent in it, score and measure the result
  idea_forge.py note "TEXT" --area A [--cost-min N] [--fix TEXT]   (the agent) log one snag at the moment it happens
  idea_forge.py feedback --init | --check | --finalize             (the agent, then the CLI) the feedback file: draft it from the notes, validate it, add the measurements
  idea_forge.py ship [RUN]                                         commit ONLY the game and its feedback, push the branch, open a PR on RedEngine (never merges)
  idea_forge.py ledger                                             every run so far
  idea_forge.py digest [--write]                                   the engine backlog: all feedback files, grouped and ranked

Standard library only. State (the seed and the position in the walk, the ledger) is in $IDEA_FORGE_HOME, else the user's state directory, so no idea is handed out twice.
The agent never changes engine code: a missing feature is a finding, not a patch (`ship` refuses a branch that touches anything else).
"""
import argparse
import datetime
import importlib.util
import json
import os
import re
import secrets
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
FORGE_DIR = os.path.join(ROOT, "examples", "2d", "idea-forge")
FEEDBACK_DIR = os.path.join("docs", "analysis", "idea-forge")
RUN_DIRNAME = ".idea-forge"


def rd(path):
    with open(path) as f:
        return f.read()


def wr(path, text, mode="w"):
    with open(path, mode) as f:
        f.write(text)


def jload(path):
    return json.loads(rd(path))


def jdump(obj, path):
    wr(path, json.dumps(obj, indent=1))


AREAS = ("discovery", "docs", "cli", "diagnostics", "format", "runtime", "verify", "publish", "perf", "tooling", "idea-fit", "worked")
PARTS = ("adjective", "noun", "you", "but", "pushback", "goal", "story")


# ---- the idea: the same vocabulary and walk as the game --------------------------------------------------------------------------------------------

_FORGE = []


def _forge_module():
    """examples/2d/idea-forge/build.py (the game's own builder): the vocabulary, the radices and the walk, so the CLI and the game cannot disagree."""
    if _FORGE:
        return _FORGE[0]
    spec = importlib.util.spec_from_file_location("idea_forge_build", os.path.join(FORGE_DIR, "build.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    _FORGE.append(mod)
    return mod


def state_dir():
    d = os.environ.get("IDEA_FORGE_HOME")
    if not d:
        base = os.environ.get("XDG_STATE_HOME") or (os.environ.get("LOCALAPPDATA") if os.name == "nt" else None) or os.path.join(os.path.expanduser("~"), ".local", "state")
        d = os.path.join(base, "idea-forge")
    os.makedirs(d, exist_ok=True)
    return d


def load_state():
    p = os.path.join(state_dir(), "state.json")
    if os.path.exists(p):
        return jload(p)
    forge = _forge_module()
    return {"seed": secrets.randbelow(forge.M), "n": 0, "runs": []}


def save_state(st):
    p = os.path.join(state_dir(), "state.json")
    tmp = p + ".tmp"
    jdump(st, tmp)
    os.replace(tmp, p)


def make_idea(code, story=True):
    forge = _forge_module()
    voc, d = forge.VOC, forge.digits(code)
    idea = {
        "code": code,
        "title": f"{voc['adjectives'][d[0]]} {voc['nouns'][d[1]]}",
        "you": voc["you"][d[2]],
        "but": voc["but"][d[3]],
        "pushback": voc["pushback"][d[4]],
        "goal": voc["goal"][d[5]],
        "story": voc["story"][d[6]] if story else "",
    }
    idea["slug"] = re.sub(r"[^a-z0-9]+", "-", idea["title"].lower()).strip("-")
    return idea


def next_codes(n):
    """The next n ideas of this machine's walk; the position is saved, so a later call never repeats one (the walk visits all ~8e9 before any repeat)."""
    forge = _forge_module()
    st = load_state()
    codes = []
    for _ in range(n):
        codes.append(forge.walk(st["seed"], st["n"]))
        st["n"] += 1
    save_state(st)
    return codes


def card(idea):
    lines = [f"{idea['title'].upper()}   (code {idea['code']:010d})", f"YOU {idea['you']},", f"BUT {idea['but']}.", f"PUSHBACK  {idea['pushback']}", f"GOAL  {idea['goal']}"]
    if idea["story"]:
        lines.append(f"STORY  {idea['story']}")
    return "\n".join(lines)


def cmd_idea(a):
    codes = [a.code] * 1 if a.code is not None else next_codes(a.n)
    ideas = [make_idea(c, not a.no_story) for c in codes]
    if a.json:
        print(json.dumps(ideas if len(ideas) > 1 else ideas[0], indent=1))
    else:
        print("\n\n".join(card(i) for i in ideas))


# ---- the brief: the whole prompt of an agent run ----------------------------------------------------------------------------------------------------

def brief(idea, today=None, cli=None):
    """`cli` is how the agent calls this script: the launching checkout's absolute path (the agent's worktree is cut from origin/main, which may not have the script yet)."""
    today = today or datetime.date.today().isoformat()
    cli = cli or "python3 scripts/idea_forge.py"
    slug, code = idea["slug"], idea["code"]
    story = f"- Story (optional dressing; use it, bend it or drop it): {idea['story']}\n" if idea["story"] else "- Story: none this time; the mechanic carries the game.\n"
    return f"""# Idea Forge run: {idea['title']} (code {code:010d}, slug `{slug}`, {today})

You are an AI game developer working with RedEngine, a data-driven game engine. This run has two products and the second matters as much as the first:

1. a small, finished, verified native 2D game built around the idea below, in this git worktree;
2. an honest, evidence-backed feedback file for the engine's maintainers: what slowed you down, what was missing, what worked, so the next run is faster.

## The idea: the mechanic is the game

- YOU {idea['you']},
- BUT {idea['but']}.
- Pushback: {idea['pushback']}
- Goal: {idea['goal']}
{story}
The card is a seed, not a spec. Before building, write a one-paragraph design: THE mechanic in one sentence (YOU + BUT together are the mechanic), the core loop (what the player does every few seconds), why it is fun,
how the game is won and lost. If parts of the card clash, keep YOU and BUT and adapt the rest, and say what you adapted. The mechanic must be what the player does most, not a power-up on a generic collector or
platformer. Finished and small beats ambitious and broken. If the mechanic cannot be built faithfully, build the nearest faithful subset and say what was cut and what the engine lacked.

## Build it

- Work only in this worktree. The game is ONE file: `examples/2d/{slug}.game2d.json` (`"game2d": 1`, id `{slug}`, a one-sentence `description`). Do not change engine code (`src/`, `crates/`, `tests/`, `scripts/`) or any
  other doc: a missing feature is a finding, not a patch. Work around it in game data and record the workaround.
- Start with `scripts/dev start "<one line>"`, then `scripts/dev red describe 2d`; use `scripts/dev red search "<question>"` and `recipe` before inventing anything. (`scripts/dev red` is the engine CLI.)
- Prove it with the engine, not by reading your own file: `validate`, `sim --only NAME --every S`, `verify`, and `frame G out.png` then LOOK at the picture. Scenarios must include one that exercises the mechanic and asserts
  its effect (not just that the game starts), and a win and a lose path unless the design has neither. Finish with `play2d G --max-ticks 120 --mute` under a virtual display if you have one.
- Sound and a saved value (`persist`) where they fit the game; polish after it works.
- Time: stop after roughly 90 minutes of work. Do not spend an hour on one obstacle: log it, work around it, move on.

## Keep a running issue log: this is the point of the run

At the moment you hit friction (not afterwards) run

    {cli} note "<what happened>" --area <{'|'.join(AREAS)}> --cost-min <minutes it cost> --fix "<the engine change that would have prevented it>"

Log: a failed command whose message did not say the fix; a feature you needed that is missing (and your workaround); a doc that was wrong, stale or missing; a `search` that found nothing; anything that cost more than two
minutes; every time you read engine source or docs outside `describe`/`search` (path and why; `--area docs` or `discovery`); how the idea fit the engine (`--area idea-fit`: a part of the card the engine could not express and the
missing capability); and what worked well (`--area worked`). One line each, concrete, with the command or message.

## Finish

1. `{cli} feedback --init` writes `{FEEDBACK_DIR}/{today}-{slug}.md`, pre-filled from your notes, the engine's command trace and a score. Edit it: rank the findings by what would save the next run the most,
   add evidence and a proposed engine change to each, and fill in "Idea fit" and "What worked".
2. `{cli} feedback --check` must pass.
3. Do not `git push`, open a PR, merge or publish: the CLI does that after review. Commit locally or leave the files uncommitted.
4. Your last message is five lines: the game path, the `verify` result, the mechanic in one sentence, and your top three findings.
"""


def cmd_brief(a):
    code = a.code if a.code is not None else next_codes(1)[0]
    print(brief(make_idea(code, not a.no_story)))


# ---- the run directory: notes, trace, transcript ---------------------------------------------------------------------------------------------------

def find_run_dir(start=None):
    """The run directory of the worktree you are in: $IDEA_FORGE_RUN, else .idea-forge/ in the current directory or a parent."""
    env = os.environ.get("IDEA_FORGE_RUN")
    if env:
        return env
    d = os.path.abspath(start or os.getcwd())
    while True:
        p = os.path.join(d, RUN_DIRNAME)
        if os.path.isfile(os.path.join(p, "run.json")):
            return p
        if os.path.dirname(d) == d:
            sys.exit("not inside an Idea Forge run (no .idea-forge/run.json above here); `idea_forge.py run` makes one")
        d = os.path.dirname(d)


def load_run(run_dir):
    return jload(os.path.join(run_dir, "run.json"))


def cmd_note(a):
    run_dir = find_run_dir(a.run)
    row = {"t": round(time.time(), 1), "area": a.area, "text": a.text, "cost_min": a.cost_min, "fix": a.fix or ""}
    with open(os.path.join(run_dir, "issues.jsonl"), "a") as f:
        f.write(json.dumps(row) + "\n")
    n = sum(1 for _ in rd(os.path.join(run_dir, "issues.jsonl")).splitlines())
    print(f"noted #{n} [{a.area}]")


def read_notes(run_dir):
    p = os.path.join(run_dir, "issues.jsonl")
    return [json.loads(l) for l in rd(p).splitlines() if l.strip()] if os.path.exists(p) else []


# ---- scoring: the engine's own answers --------------------------------------------------------------------------------------------------------------

def engine_cmd(root):
    exe = os.environ.get("RED_ENGINE_EXE")
    if exe:
        return [exe]
    return [os.path.join(root, "scripts", "dev"), "red"]


def run_engine(root, args, timeout=900):
    p = subprocess.run(engine_cmd(root) + args, cwd=root, capture_output=True, text=True, timeout=timeout)
    return p.returncode, p.stdout + p.stderr


def score(root, slug):
    """Facts about the finished game, from `validate` and `verify` and the game file; each is {name, ok, detail}."""
    game = os.path.join("examples", "2d", f"{slug}.game2d.json")
    items = []

    def item(name, ok, detail=""):
        items.append({"name": name, "ok": bool(ok), "detail": detail})

    if not os.path.isfile(os.path.join(root, game)):
        item("the game file exists", False, f"{game} is missing")
        return items
    item("the game file exists", True, game)
    g = jload(os.path.join(root, game))
    code, out = run_engine(root, ["validate", game])
    item("validate passes", code == 0, (out.strip().splitlines() or [""])[0])
    code, out = run_engine(root, ["verify", game])
    last = next((l for l in reversed(out.strip().splitlines()) if re.search(r"\d+ passed", l)), "")
    item("verify passes", code == 0, last)
    scs = g.get("checks", {}).get("scenarios", [])
    item("at least three scenarios", len(scs) >= 3, f"{len(scs)} scenario(s)")
    item("one scenario is marked smoke", any(s.get("smoke") for s in scs))
    item("a one-sentence description", len(g.get("description", "")) > 20, g.get("description", "")[:80])
    item("a sound or music", bool(g.get("sounds") or g.get("music")))
    item("something is saved between runs", bool(g.get("persist")))
    return items


def changed_paths(root, base="origin/main"):
    """Paths this branch changed against `base`, including uncommitted and untracked files."""
    out = set()
    for args in (["diff", "--name-only", base], ["diff", "--name-only"], ["ls-files", "--others", "--exclude-standard"]):
        p = subprocess.run(["git", "-C", root] + args, capture_output=True, text=True)
        out.update(l.strip() for l in p.stdout.splitlines() if l.strip())
    return sorted(out)


def allowed_path(path, slug):
    return path in (f"examples/2d/{slug}.game2d.json",) or (path.startswith(FEEDBACK_DIR + "/") and path.endswith(".md"))


# ---- the feedback file ----------------------------------------------------------------------------------------------------------------------------

FINDINGS_RE = re.compile(r"```json findings\n(.*?)\n```", re.S)
AUTO_BEGIN, AUTO_END = "<!-- automatic-measurements:begin -->", "<!-- automatic-measurements:end -->"
REQUIRED_SECTIONS = ("## Findings", "## Idea fit", "## What worked")


def feedback_path(root, run):
    return os.path.join(root, FEEDBACK_DIR, f"{run['date']}-{run['slug']}.md")


def read_findings(text):
    m = FINDINGS_RE.search(text)
    if not m:
        raise ValueError("no ```json findings block")
    rows = json.loads(m.group(1))
    if not isinstance(rows, list):
        raise ValueError("the findings block must be a JSON list")
    return rows


def check_findings(rows):
    """Problems with a findings list ([] when it is sound)."""
    bad, seen = [], set()
    for i, r in enumerate(rows):
        w = f"finding {i + 1}"
        if not isinstance(r, dict):
            bad.append(f"{w}: not an object")
            continue
        for k in ("id", "area", "title", "severity", "cost_min", "evidence", "proposal"):
            if k not in r:
                bad.append(f"{w}: missing `{k}`")
        if r.get("area") not in AREAS:
            bad.append(f"{w}: area `{r.get('area')}` is not one of {', '.join(AREAS)}")
        if r.get("severity") not in (1, 2, 3):
            bad.append(f"{w}: severity must be 1 (papercut), 2 (cost real time) or 3 (blocked or would block most runs)")
        if not isinstance(r.get("cost_min"), (int, float)):
            bad.append(f"{w}: cost_min must be a number of minutes")
        if r.get("area") != "worked" and not str(r.get("proposal", "")).strip():
            bad.append(f"{w}: say what the engine should change (`proposal`)")
        if not str(r.get("evidence", "")).strip():
            bad.append(f"{w}: `evidence` is empty (the command, the message, the number)")
        if r.get("id") in seen:
            bad.append(f"{w}: duplicate id `{r.get('id')}`")
        seen.add(r.get("id"))
    return bad


def check_feedback_text(text):
    bad = [f"missing section `{s}`" for s in REQUIRED_SECTIONS if s not in text]
    try:
        rows = read_findings(text)
        bad += check_findings(rows)
        if not rows:
            bad.append("no findings: a run with nothing to report did not look hard enough (log `worked` entries too)")
    except ValueError as e:
        bad.append(str(e))
    for section in ("## Idea fit", "## What worked"):
        i = text.find(section)
        if i >= 0:
            body = text[i + len(section):].split("\n## ")[0]
            if len(body.strip()) < 40 or "TODO" in body:
                bad.append(f"`{section}` is empty or still a TODO")
    if "TODO" in text.replace(AUTO_BEGIN, ""):
        bad.append("a TODO is left in the file")
    return bad


def draft_feedback(run, notes, items, today):
    findings = []
    for i, n in enumerate(notes, 1):
        findings.append({"id": f"F{i}", "area": n["area"], "title": n["text"][:110], "severity": 2 if n.get("cost_min", 0) >= 5 else 1, "cost_min": n.get("cost_min", 0),
                         "evidence": "TODO: the command and the message, or the number", "workaround": "", "proposal": n.get("fix") or "TODO: the engine change"})
    idea = run["idea"]
    score_lines = "\n".join(f"- {'pass' if s['ok'] else 'FAIL'}: {s['name']}" + (f" ({s['detail']})" if s["detail"] else "") for s in items)
    return f"""# Idea Forge run: {idea['title']} ({today})

- Idea code: `{idea['code']:010d}`; slug `{run['slug']}`; game `examples/2d/{run['slug']}.game2d.json`
- Engine revision: `{run.get('engine_revision', '')}`; model: `{run.get('model', '')}`
- The card: YOU {idea['you']}, BUT {idea['but']}. Pushback: {idea['pushback']} Goal: {idea['goal']}

## The game

TODO: the design paragraph (the mechanic in one sentence, the core loop, win and lose) and what you cut.

## Score at the time of writing

{score_lines}

## Findings

Ranked: what would save the next run the most first. `severity` 1 = papercut, 2 = cost real time, 3 = blocked this run or would block most runs. `proposal` is an engine change, not a workaround.

```json findings
{json.dumps(findings, indent=1)}
```

## Idea fit

TODO: which parts of the card the engine could express, which it could not, and the capability that was missing.

## What worked

TODO: what the engine did well that should be kept (commands, messages, speed).

## Automatic measurements

{AUTO_BEGIN}
(the CLI fills this in after the run: command trace, failures, repair cycles, documentation read, engine source read)
{AUTO_END}
"""


def cmd_feedback(a):
    run_dir = find_run_dir(a.run)
    run = load_run(run_dir)
    root = os.path.dirname(run_dir)
    path = feedback_path(root, run)
    if a.init:
        if os.path.exists(path) and not a.force:
            sys.exit(f"{path} exists; edit it (or --force to start over from your notes)")
        os.makedirs(os.path.dirname(path), exist_ok=True)
        text = draft_feedback(run, read_notes(run_dir), score(root, run["slug"]), run["date"])
        wr(path, text)
        print(f"wrote {os.path.relpath(path, root)}: edit the TODOs, then `feedback --check`")
    elif a.check:
        if not os.path.exists(path):
            sys.exit(f"{path} does not exist: run `feedback --init`")
        bad = check_feedback_text(rd(path))
        for b in bad:
            print("FAIL", b)
        print("feedback OK" if not bad else f"{len(bad)} problem(s)")
        sys.exit(1 if bad else 0)
    elif a.finalize:
        measurements(run_dir, run, root)
        print("measurements written")


def measurements(run_dir, run, root):
    """Fill the automatic block from the engine's command trace and the agent's transcript (the same friction report the fresh-agent benchmark prints)."""
    path = feedback_path(root, run)
    if not os.path.exists(path):
        return
    out = []
    trace, session = os.path.join(run_dir, "trace.jsonl"), os.path.join(run_dir, "session.jsonl")
    if os.path.exists(trace) and os.path.getsize(trace):
        args = [sys.executable, os.path.join(HERE, "agent_bench.py"), "summary", trace] + (["--transcript", session] if os.path.exists(session) else [])
        p = subprocess.run(args, capture_output=True, text=True)
        out.append("Friction report (`scripts/agent_bench.py summary`):\n\n```json\n" + (p.stdout.strip() or p.stderr.strip()) + "\n```")
    if os.path.exists(session):
        out.append(session_facts(session))
    notes = read_notes(run_dir)
    out.append(f"Notes logged during the run: {len(notes)} ({sum(n.get('cost_min', 0) for n in notes):g} minutes claimed lost).")
    text = rd(path)
    block = f"{AUTO_BEGIN}\n" + "\n\n".join(out) + f"\n{AUTO_END}"
    if AUTO_BEGIN in text:
        text = re.sub(re.escape(AUTO_BEGIN) + r".*?" + re.escape(AUTO_END), lambda _m: block, text, flags=re.S)
    else:
        text += "\n## Automatic measurements\n\n" + block + "\n"
    wr(path, text)


def session_facts(session):
    """Turns, tools and cost from a `claude -p --output-format stream-json` transcript."""
    tools, turns, cost, dur = {}, 0, None, None
    for line in rd(session).splitlines():
        try:
            ev = json.loads(line)
        except ValueError:
            continue
        if ev.get("type") == "assistant":
            turns += 1
            for blk in ev.get("message", {}).get("content", []):
                if isinstance(blk, dict) and blk.get("type") == "tool_use":
                    tools[blk.get("name")] = tools.get(blk.get("name"), 0) + 1
        if ev.get("type") == "result":
            cost, dur = ev.get("total_cost_usd"), ev.get("duration_ms")
    return "Agent session: %d assistant turns; tools %s; cost %s; wall %s." % (
        turns, json.dumps(tools, sort_keys=True), "unknown" if cost is None else f"${cost:.2f}", "unknown" if dur is None else f"{dur / 60000:.1f} min")


# ---- run: worktree + headless agent ----------------------------------------------------------------------------------------------------------------

def git(root, *args, check=True):
    p = subprocess.run(["git", "-C", root] + list(args), capture_output=True, text=True)
    if check and p.returncode:
        sys.exit(f"git {' '.join(args)}: {p.stderr.strip()}")
    return p.stdout.strip()


def make_worktree(slug):
    """A second checkout on its own branch, seeded from this checkout's build (scripts/dev worktree), next to this one."""
    name = f"idea-{slug}"
    wt = os.path.join(os.path.dirname(ROOT), f"{os.path.basename(ROOT)}-{name}")
    if os.path.exists(wt):
        sys.exit(f"{wt} exists: another run of this idea? remove it (git worktree remove) or pick another idea")
    p = subprocess.run([os.path.join(ROOT, "scripts", "dev"), "worktree", name, "origin/main"], cwd=ROOT, capture_output=True, text=True)
    sys.stderr.write(p.stdout + p.stderr)
    if p.returncode or not os.path.isdir(wt):
        sys.exit("could not make the worktree")
    return wt, name


def agent_argv(prompt, a):
    argv = [a.claude, "-p", prompt, "--output-format", "stream-json", "--verbose", "--allowedTools", "Bash Read Edit Write Glob Grep",
            "--disallowedTools", "Bash(git push:*) Bash(gh:*) Bash(curl:*) Bash(wget:*)", "--permission-mode", "acceptEdits"]
    if a.model:
        argv += ["--model", a.model]
    if a.budget:
        argv += ["--max-budget-usd", str(a.budget)]
    return argv


def stream_agent(argv, cwd, env, session_path, timeout, quiet):
    """Run the agent, keep its whole transcript, and print one line per tool call so a human can watch."""
    start = time.time()
    with open(session_path, "w") as out:
        p = subprocess.Popen(argv, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        try:
            for line in p.stdout:
                out.write(line)
                out.flush()
                if quiet:
                    continue
                try:
                    ev = json.loads(line)
                except ValueError:
                    continue
                for blk in ev.get("message", {}).get("content", []) if ev.get("type") == "assistant" and isinstance(ev.get("message"), dict) else []:
                    if isinstance(blk, dict) and blk.get("type") == "tool_use":
                        inp = blk.get("input", {})
                        what = inp.get("command") or inp.get("file_path") or inp.get("pattern") or ""
                        print(f"[{(time.time() - start) / 60:5.1f}m] {blk.get('name')}: {str(what).splitlines()[0][:110] if what else ''}", file=sys.stderr)
                if time.time() - start > timeout:
                    p.kill()
                    print("timeout: the agent was stopped", file=sys.stderr)
                    break
        finally:
            p.wait()
    return p.returncode


def cmd_run(a):
    if not shutil.which(a.claude) and not os.path.isfile(a.claude):
        sys.exit(f"`{a.claude}` not found: install Claude Code, or pass --claude PATH")
    code = a.code if a.code is not None else next_codes(1)[0]
    idea = make_idea(code, not a.no_story)
    today = datetime.date.today().isoformat()
    print(card(idea), file=sys.stderr)
    if a.workdir:
        wt, branch = os.path.abspath(a.workdir), git(os.path.abspath(a.workdir), "rev-parse", "--abbrev-ref", "HEAD")
    else:
        wt, branch = make_worktree(idea["slug"])
    run_dir = os.path.join(wt, RUN_DIRNAME)
    os.makedirs(run_dir, exist_ok=True)
    exclude = git(wt, "rev-parse", "--git-path", "info/exclude")
    exclude = exclude if os.path.isabs(exclude) else os.path.join(wt, exclude)
    os.makedirs(os.path.dirname(exclude), exist_ok=True)
    if RUN_DIRNAME + "/" not in (rd(exclude) if os.path.exists(exclude) else ""):
        wr(exclude, RUN_DIRNAME + "/\n", "a")
    prompt = brief(idea, today, f"python3 {os.path.abspath(__file__)}")
    run = {"idea": idea, "slug": idea["slug"], "date": today, "branch": branch, "worktree": wt, "model": a.model or "default", "started": time.time(),
           "engine_revision": git(wt, "rev-parse", "--short=12", "origin/main", check=False), "budget_usd": a.budget}
    jdump(run, os.path.join(run_dir, "run.json"))
    wr(os.path.join(run_dir, "brief.md"), prompt)
    st = load_state()
    st["runs"] = [r for r in st["runs"] if not (r["code"] == code and r["slug"] == idea["slug"])]   # a re-run (or --workdir after a dry run) replaces its own row
    st["runs"].append({"code": code, "slug": idea["slug"], "title": idea["title"], "date": today, "worktree": wt, "branch": branch, "status": "running"})
    save_state(st)
    print(f"run directory {run_dir}\nbranch {branch}", file=sys.stderr)
    if a.dry_run:
        print(f"dry run: would run {' '.join(agent_argv('<brief.md>', a))} in {wt}", file=sys.stderr)
        return
    env = dict(os.environ, IDEA_FORGE_RUN=run_dir, RED_TRACE=os.path.join(run_dir, "trace.jsonl"), IDEA_FORGE_HOME=state_dir())
    code_rc = stream_agent(agent_argv(prompt, a), wt, env, os.path.join(run_dir, "session.jsonl"), a.timeout_min * 60, a.quiet)
    run["finished"] = time.time()
    run["agent_exit"] = code_rc
    run["score"] = score(wt, idea["slug"])
    jdump(run, os.path.join(run_dir, "run.json"))
    measurements(run_dir, run, wt)
    fb = feedback_path(wt, run)
    fb_problems = check_feedback_text(rd(fb)) if os.path.exists(fb) else ["no feedback file was written"]
    st = load_state()
    for r in st["runs"]:
        if r["code"] == code and r["slug"] == idea["slug"]:
            r.update(status="built" if all(s["ok"] for s in run["score"][:3]) else "incomplete", minutes=round((run["finished"] - run["started"]) / 60, 1),
                     feedback_ok=not fb_problems, notes=len(read_notes(run_dir)))
    save_state(st)
    print(f"\n{'pass' if not fb_problems else 'FAIL'}: feedback file" + ("" if not fb_problems else "\n  " + "\n  ".join(fb_problems)))
    for s in run["score"]:
        print(f"{'pass' if s['ok'] else 'FAIL'}: {s['name']}" + (f" ({s['detail']})" if s["detail"] else ""))
    print(f"\nreview the work in {wt}, then: python3 scripts/idea_forge.py ship {idea['slug']}")


# ---- ship: commit exactly the game and the feedback, push, PR --------------------------------------------------------------------------------------

def find_run_by_slug(slug):
    for r in reversed(load_state()["runs"]):
        if r["slug"] == slug or os.path.abspath(r["worktree"]) == os.path.abspath(slug):
            return r
    sys.exit(f"no run `{slug}` in the ledger (`idea_forge.py ledger`)")


def cmd_ship(a):
    r = find_run_by_slug(a.run)
    wt, slug = r["worktree"], r["slug"]
    stray = [p for p in changed_paths(wt) if not allowed_path(p, slug) and not p.startswith(RUN_DIRNAME + "/")]
    if stray:
        sys.exit("refusing to ship: the run changed more than the game and its feedback (a missing engine feature is a finding, not a patch):\n  " + "\n  ".join(stray))
    run = load_run(os.path.join(wt, RUN_DIRNAME))
    fb = feedback_path(wt, run)
    bad = check_feedback_text(rd(fb)) if os.path.exists(fb) else ["no feedback file"]
    if bad:
        sys.exit("refusing to ship: the feedback is not ready:\n  " + "\n  ".join(bad))
    paths = [os.path.relpath(fb, wt), f"examples/2d/{slug}.game2d.json"]
    git(wt, "add", "--", *paths)
    title = f"Idea Forge run: {run['idea']['title']} (feedback and a game built around one mechanic)"
    msg = f"{title}\n\nGenerated by scripts/idea_forge.py from idea code {run['idea']['code']:010d}; the feedback file is docs/analysis/idea-forge/.\n\nCo-Authored-By: Claude <noreply@anthropic.com>"
    git(wt, "commit", "-q", "-m", msg, check=False)
    if a.no_push:
        print(f"committed on {r['branch']} in {wt}; not pushed (--no-push)")
        return
    git(wt, "push", "-q", "-u", "origin", r["branch"])
    body = (f"## What\nA game built around one mechanic, forged by `scripts/idea_forge.py` (code `{run['idea']['code']:010d}`), and the engine feedback its development produced.\n\n"
            f"- Idea: YOU {run['idea']['you']}, BUT {run['idea']['but']}.\n- Game: `examples/2d/{slug}.game2d.json`\n- Feedback: `{os.path.relpath(fb, wt)}`\n\n"
            "No engine code changed. Merge after reading the findings; the digest (`idea_forge.py digest`) ranks them with every earlier run.\n\n"
            "🤖 Generated with [Claude Code](https://claude.com/claude-code)")
    p = subprocess.run(["gh", "pr", "create", "--base", "main", "--head", r["branch"], "--title", title, "--body", body], cwd=wt, capture_output=True, text=True)
    print(p.stdout.strip() or p.stderr.strip())
    st = load_state()
    for x in st["runs"]:
        if x["slug"] == slug:
            x["status"] = "shipped"
            x["pr"] = p.stdout.strip().splitlines()[-1] if p.returncode == 0 and p.stdout.strip() else ""
    save_state(st)


# ---- ledger and digest -------------------------------------------------------------------------------------------------------------------------------

def cmd_ledger(a):
    runs = load_state()["runs"]
    if not runs:
        print("no runs yet: `idea_forge.py run`")
    for r in runs:
        print(f"{r['date']}  {r['code']:010d}  {r['slug']:<28} {r.get('status', '?'):<10} {r.get('minutes', '-'):>5} min  notes {r.get('notes', '-')}  feedback {'ok' if r.get('feedback_ok') else 'no'}  {r.get('pr', '')}")


def collect_findings(directory):
    rows = []
    for name in sorted(os.listdir(directory)) if os.path.isdir(directory) else []:
        if not name.endswith(".md") or name.startswith("DIGEST"):
            continue
        try:
            for f in read_findings(rd(os.path.join(directory, name))):
                rows.append(dict(f, run=name[:-3]))
        except ValueError:
            continue
    return rows


def digest_text(rows):
    """Findings from every run grouped by area, ranked by (severity, minutes, how many runs hit it): the engine backlog this loop produces."""
    if not rows:
        return "No feedback files yet.\n"
    runs = sorted({r["run"] for r in rows})
    out = [f"# Idea Forge digest\n\n{len(runs)} run(s), {len(rows)} findings, {sum(r.get('cost_min', 0) for r in rows):g} minutes reported lost.\n"]
    by_area = {}
    for r in rows:
        by_area.setdefault(r["area"], []).append(r)
    out.append("| area | findings | runs | minutes lost | worst |\n|---|---|---|---|---|")
    for area, rs in sorted(by_area.items(), key=lambda kv: -sum(x.get("cost_min", 0) for x in kv[1])):
        out.append(f"| {area} | {len(rs)} | {len({x['run'] for x in rs})} | {sum(x.get('cost_min', 0) for x in rs):g} | {max(x.get('severity', 0) for x in rs)} |")
    out.append("\n## Ranked (severity, then minutes lost)\n")
    for r in sorted((x for x in rows if x["area"] != "worked"), key=lambda x: (-x.get("severity", 0), -x.get("cost_min", 0)))[:40]:
        out.append(f"- **[{r['area']}, sev {r.get('severity')}, {r.get('cost_min', 0):g} min]** {r['title']}  \n  proposal: {r.get('proposal', '')}  \n  _({r['run']} {r['id']})_")
    worked = [r for r in rows if r["area"] == "worked"]
    if worked:
        out.append("\n## Keep (reported as working well)\n")
        out += [f"- {r['title']} _({r['run']})_" for r in worked[:20]]
    return "\n".join(out) + "\n"


def cmd_digest(a):
    directory = os.path.join(ROOT, FEEDBACK_DIR)
    text = digest_text(collect_findings(directory))
    if a.write:
        os.makedirs(directory, exist_ok=True)
        wr(os.path.join(directory, "DIGEST.md"), text)
        print(f"wrote {os.path.join(FEEDBACK_DIR, 'DIGEST.md')}")
    else:
        print(text)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)

    s = sub.add_parser("idea")
    s.add_argument("--n", type=int, default=1)
    s.add_argument("--code", type=int)
    s.add_argument("--no-story", action="store_true")
    s.add_argument("--json", action="store_true")
    s.set_defaults(f=cmd_idea)

    s = sub.add_parser("brief")
    s.add_argument("--code", type=int)
    s.add_argument("--no-story", action="store_true")
    s.set_defaults(f=cmd_brief)

    s = sub.add_parser("run")
    s.add_argument("--code", type=int)
    s.add_argument("--no-story", action="store_true")
    s.add_argument("--model")
    s.add_argument("--budget", type=float, help="USD cap for the agent session (claude --max-budget-usd)")
    s.add_argument("--timeout-min", type=int, default=150)
    s.add_argument("--claude", default="claude")
    s.add_argument("--workdir", help="use this existing checkout instead of making a worktree")
    s.add_argument("--dry-run", action="store_true", help="make the worktree and the brief, do not start the agent")
    s.add_argument("--quiet", action="store_true")
    s.set_defaults(f=cmd_run)

    s = sub.add_parser("note")
    s.add_argument("text")
    s.add_argument("--area", required=True, choices=AREAS)
    s.add_argument("--cost-min", type=float, default=0)
    s.add_argument("--fix")
    s.add_argument("--run")
    s.set_defaults(f=cmd_note)

    s = sub.add_parser("feedback")
    g = s.add_mutually_exclusive_group(required=True)
    g.add_argument("--init", action="store_true")
    g.add_argument("--check", action="store_true")
    g.add_argument("--finalize", action="store_true")
    s.add_argument("--force", action="store_true")
    s.add_argument("--run")
    s.set_defaults(f=cmd_feedback)

    s = sub.add_parser("ship")
    s.add_argument("run", help="the slug (or worktree) of a run in the ledger")
    s.add_argument("--no-push", action="store_true")
    s.set_defaults(f=cmd_ship)

    sub.add_parser("ledger").set_defaults(f=cmd_ledger)
    s = sub.add_parser("digest")
    s.add_argument("--write", action="store_true")
    s.set_defaults(f=cmd_digest)

    a = ap.parse_args(argv)
    a.f(a)


if __name__ == "__main__":
    main()
