#!/usr/bin/env python3
"""Idea Forge: forge a game idea, have an AI build it with RedEngine, and file the engine feedback that run produced (docs/IDEA_FORGE.md).

  idea_forge.py idea [--n N] [--code C] [--no-story] [--json]     the next idea nobody has been given yet (or the one behind CODE)
  idea_forge.py brief [--code C]                                   the prompt an agent gets for an idea
  idea_forge.py run [--kind 2d|3d] [--code C] [--budget USD]       forge an idea, make a worktree, run a headless `claude -p` agent in it, score and measure the result
  idea_forge.py daily [--next] [--no-ship] [--budget USD]          today's two games, one 2D and one 3D in a random order each day: run what is still pending, ship each as a PR
  idea_forge.py schedule [--times 09:30,17:30] [--install]         the cron entries that run `daily --next` twice a day (printed unless --install; --uninstall removes them)
  idea_forge.py note "TEXT" --area A [--cost-min N] [--fix TEXT]   (the agent) log one snag at the moment it happens
  idea_forge.py feedback --init | --check | --finalize             (the agent, then the CLI) the feedback file: draft it from the notes, validate it, add the measurements
  idea_forge.py ship [RUN] [--hold]                                commit ONLY the game and its feedback, push the branch, open a PR on RedEngine, then archive the run and remove its worktree
  idea_forge.py settle [--wait-min N]                              merge the tool's own PRs whose CI has passed (the default; `config --auto-merge off` stops it)
  idea_forge.py ledger                                             every run so far
  idea_forge.py digest [--write]                                   the engine backlog: all feedback files, grouped and ranked

Standard library only. State (the seed and the position in the walk, the ledger) is in $IDEA_FORGE_HOME, else the user's state directory, so no idea is handed out twice.
A game agent never changes engine code: a missing feature is a finding, not a patch (`ship` refuses a branch that touches anything else). The tool merges ONLY its own PRs, and only after
every CI check has passed (`settle`; `config --auto-merge off` or `ship --hold` stops it).
"""
import argparse
import datetime
import importlib.util
import json
import os
import re
import random
import secrets
import shlex
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
FORGE_DIR = os.path.join(ROOT, "examples", "2d", "idea-forge")
FEEDBACK_DIR = "docs/analysis/idea-forge"   # always forward slashes: git reports paths that way on every OS, and Windows accepts them
RUN_DIRNAME = ".idea-forge"
KINDS = ("2d", "3d")
MAX_ATTEMPTS = 2        # a daily slot whose agent produced nothing that builds is tried once more with a fresh idea
IMPROVE_BUDGET_USD = 25  # the engine-improvement agent edits real code and runs the repo's gates: a larger cap than a game
DAILY_BUDGET_USD = 15   # per game, the `claude --max-budget-usd` cap used by `daily` and `schedule` unless told otherwise
CRON_MARK = "# idea-forge"


def game_file(kind, slug):
    """The one file `verify` runs: a 2D game is a single JSON; a 3D game is a scene inside its own folder (audio scores and the like live beside it)."""
    return f"examples/2d/{slug}.game2d.json" if kind == "2d" else f"examples/3d/{slug}/{slug}.json"


def game_prefix(kind, slug):
    return f"examples/2d/{slug}.game2d.json" if kind == "2d" else f"examples/3d/{slug}/"


def feedback_name(run_or_date, kind=None, slug=None):
    """`<date>-<kind>-<slug>.md`; runs recorded before there were kinds (no `kind` in run.json) keep `<date>-<slug>.md`."""
    if isinstance(run_or_date, dict):
        r = run_or_date
        return f"{r['date']}-{r['kind']}-{r['slug']}.md" if r.get("kind") else f"{r['date']}-{r['slug']}.md"
    return f"{run_or_date}-{kind}-{slug}.md"


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

BUILD_2D = """- Work only in this worktree. The game is ONE file: `examples/2d/{slug}.game2d.json` (`"game2d": 1`, id `{slug}`, a one-sentence `description`). Do not change engine code (`src/`, `crates/`, `tests/`, `scripts/`) or any
  other doc: a missing feature is a finding, not a patch. Work around it in game data and record the workaround.
- Start with `scripts/dev start "<one line>"`, then `scripts/dev red describe 2d`; use `scripts/dev red search "<question>"` and `recipe` before inventing anything. (`scripts/dev red` is the engine CLI.)
- Prove it with the engine, not by reading your own file: `validate`, `sim --only NAME --every S`, `verify`, and `frame G out.png` then LOOK at the picture. Scenarios must include one that exercises the mechanic and asserts
  its effect (not just that the game starts), and a win and a lose path unless the design has neither. Finish with `play2d G --max-ticks 120 --mute` under a virtual display if you have one.
"""

BUILD_3D = """- Work only in this worktree. The game is a first-person 3D RedEngine scene: `examples/3d/{slug}/{slug}.json` (a map with `player`, `spawns`, `objects`, `zones`, `vars`, `rules`, `ui`, `checks`), plus anything it needs beside it under
  `examples/3d/{slug}/` (for example `audio/*.json` scores). Game logic is data (`rules`), never Rust. Do not change engine code (`src/`, `crates/`, `tests/`, `scripts/`) or any other doc: a missing feature is a finding,
  not a patch. Work around it in game data and record the workaround.
- Start with `scripts/dev start "<one line>"`, then `scripts/dev red describe --brief`, `describe rules` and `describe scene`; use `search "<question>"`, `recipe` (`recipe coin_run --new examples/3d/{slug}/{slug}.json` is a complete
  small game to start from), `catalog <word>` for assets and `build --example` for room layouts before inventing anything. (`scripts/dev red` is the engine CLI.)
- Prove it with the engine, not by reading your own file: `lint`, `plan` / `tour` pictures (LOOK at the PNGs), `reach` checks for places the player must get to, `sim` with `checks.sim` scripted playthroughs, and `verify`.
  Scenarios must include one that exercises the mechanic and asserts its effect (not just that the game starts), and a win and a lose path unless the design has neither. If `playtest` (pictures from the player's eyes,
  no window) works on this machine use it; if it does not, log why.
- Keep the world small: one or two rooms or a small outdoor area is plenty. The mechanic must be something a first-person player does with their hands, in the world.
"""

POLISH_2D = "- Sound and a saved value (`persist`) where they fit the game; polish after it works."
POLISH_3D = "- Sound (`audio`) where it fits the game; polish after it works."


def brief(idea, today=None, cli=None, kind="2d", known=""):
    """`cli` is how the agent calls this script: the launching checkout's absolute path (the agent's worktree is cut from origin/main, which may not have the script yet)."""
    today = today or datetime.date.today().isoformat()
    cli = cli or "python3 scripts/idea_forge.py"
    slug, code = idea["slug"], idea["code"]
    fb = f"{FEEDBACK_DIR}/{feedback_name(today, kind, slug)}"
    story = f"- Story (optional dressing; use it, bend it or drop it): {idea['story']}\n" if idea["story"] else "- Story: none this time; the mechanic carries the game.\n"
    build_steps = (BUILD_3D if kind == "3d" else BUILD_2D).format(slug=slug)
    known = known or ""
    polish = POLISH_3D if kind == "3d" else POLISH_2D
    return f"""# Idea Forge run: {idea['title']} ({kind.upper()}, code {code:010d}, slug `{slug}`, {today})

You are an AI game developer working with RedEngine, a data-driven game engine. This run has two products and the second matters as much as the first:

1. a small, finished, verified native {kind.upper()} game built around the idea below, in this git worktree;
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

{build_steps}{polish}
- Time: stop after roughly 90 minutes of work. Do not spend an hour on one obstacle: log it, work around it, move on.

{known}## Keep a running issue log: this is the point of the run

At the moment you hit friction (not afterwards) run

    {cli} note "<what happened>" --area <{'|'.join(AREAS)}> --key <stable-kebab-name-of-the-problem> --cost-min <minutes it really cost> --fix "<the engine change that would have prevented it>"

Log: a failed command whose message did not say the fix; a feature you needed that is missing (and your workaround); a doc that was wrong, stale or missing; a `search` that found nothing; anything that cost more than two
minutes; every time you read engine source or docs outside `describe`/`search` (path and why; `--area docs` or `discovery`); how the idea fit the engine (`--area idea-fit`: a part of the card the engine could not express and the
missing capability); and what worked well (`--area worked`). One line each, concrete, with the command or message. `--cost-min` is wall-clock minutes you actually lost, not a guess at the engine's importance (the total cannot exceed your session; the CLI compares it with the measured time and says so when it does).

## Finish

1. `{cli} feedback --init` writes `{fb}`, pre-filled from your notes, the engine's command trace and a score. Edit it: rank the findings by what would save the next run the most,
   add evidence and a proposed engine change to each, and fill in "Idea fit" and "What worked".
2. `{cli} feedback --check` must pass.
3. Do not `git push`, open a PR, merge or publish: the CLI does that after review. Commit locally or leave the files uncommitted.
4. Your last message is five lines: the game path, the `verify` result, the mechanic in one sentence, and your top three findings.
"""


def cmd_brief(a):
    code = a.code if a.code is not None else next_codes(1)[0]
    print(brief(make_idea(code, not a.no_story), kind=a.kind))


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
    if a.key and not KEY_RE.match(a.key):
        sys.exit(f"--key must be kebab-case (3-60 chars of a-z 0-9 -), got `{a.key}`")
    row = {"t": round(time.time(), 1), "area": a.area, "text": a.text, "cost_min": a.cost_min, "fix": a.fix or "", "key": a.key or ""}
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


def score(root, slug, kind="2d"):
    """Facts about the finished game, from the engine's own answers and the game file; each is {name, ok, detail}. The first three decide whether the run `built`."""
    game = game_file(kind, slug)
    items = []

    def item(name, ok, detail=""):
        items.append({"name": name, "ok": bool(ok), "detail": detail})

    if not os.path.isfile(os.path.join(root, game)):
        item("the game file exists", False, f"{game} is missing")
        return items
    item("the game file exists", True, game)
    g = jload(os.path.join(root, game))
    code, out = run_engine(root, ["validate" if kind == "2d" else "lint", game])
    item("validate passes" if kind == "2d" else "lint passes", code == 0, (out.strip().splitlines() or [""])[0])
    code, out = run_engine(root, ["verify", game])
    last = next((l for l in reversed(out.strip().splitlines()) if re.search(r"\d+ passed|\d+ check\(s\)", l)), "")
    item("verify passes", code == 0, last)
    if kind == "2d":
        scs = g.get("checks", {}).get("scenarios", [])
        item("at least three scenarios", len(scs) >= 3, f"{len(scs)} scenario(s)")
        item("one scenario is marked smoke", any(s_.get("smoke") for s_ in scs))
        item("a one-sentence description", len(g.get("description", "")) > 20, g.get("description", "")[:80])
        item("a sound or music", bool(g.get("sounds") or g.get("music")))
        item("something is saved between runs", bool(g.get("persist")))
    else:
        sims = g.get("checks", {}).get("sim", [])
        item("at least two scripted playthroughs (checks.sim)", len(sims) >= 2, f"{len(sims)} playthrough(s)")
        item("the game logic is data (rules)", bool(g.get("rules")), f"{len(g.get('rules') or [])} rule(s)")
        item("a declared ui (cards, objective)", bool(g.get("ui")))
        item("a sound or music (audio)", bool(g.get("audio")))
    return items


def changed_paths(root, base="origin/main"):
    """Paths this branch changed against `base`, including uncommitted and untracked files."""
    out = set()
    for args in (["diff", "--name-only", base], ["diff", "--name-only"], ["ls-files", "--others", "--exclude-standard"]):
        p = subprocess.run(["git", "-C", root] + args, capture_output=True, text=True)
        out.update(l.strip() for l in p.stdout.splitlines() if l.strip())
    return sorted(out)


def allowed_path(path, slug, kind="2d"):
    """The only paths a run may change: its game (a 3D game owns its folder) and its feedback file."""
    prefix = game_prefix(kind, slug)
    own = path == prefix if kind == "2d" else path.startswith(prefix)
    return own or (path.startswith(FEEDBACK_DIR + "/") and path.endswith(".md"))


# ---- the feedback file ----------------------------------------------------------------------------------------------------------------------------

FINDINGS_RE = re.compile(r"```json findings\n(.*?)\n```", re.S)
AUTO_BEGIN, AUTO_END = "<!-- automatic-measurements:begin -->", "<!-- automatic-measurements:end -->"
REQUIRED_SECTIONS = ("## Findings", "## Idea fit", "## What worked")
KEY_RE = re.compile(r"^[a-z0-9][a-z0-9-]{2,60}$")
FIXES_FILE = FEEDBACK_DIR + "/fixes.json"
AUTO_BANNER = "> Auto-completed by the CLI: the agent did not finish this file. Its notes and the measurements are real; the prose it did not write is marked."
NOT_DONE = "(not completed by the agent: the run ended before this was written)"


def slugify(text, n=50):
    return re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-")[:n].strip("-") or "finding"


def feedback_path(root, run):
    return os.path.join(root, FEEDBACK_DIR, feedback_name(run))


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
        for k in ("id", "area", "title", "severity", "cost_min", "evidence", "proposal", "key"):
            if k not in r:
                bad.append(f"{w}: missing `{k}`")
        if "key" in r and not KEY_RE.match(str(r["key"])):
            bad.append(f"{w}: `key` must be a stable kebab-case name of the problem (3-60 chars of a-z 0-9 -), not `{r['key']}`; reuse the key of a known issue when it is the same problem")
        if r.get("area") not in AREAS:
            bad.append(f"{w}: area `{r.get('area')}` is not one of {', '.join(AREAS)}")
        if r.get("severity") not in (1, 2, 3):
            bad.append(f"{w}: severity must be 1 (papercut), 2 (cost real time) or 3 (blocked or would block most runs)")
        if not isinstance(r.get("cost_min"), (int, float)):
            bad.append(f"{w}: cost_min must be a number of minutes")
        if r.get("area") != "worked" and not str(r.get("proposal", "")).strip():
            bad.append(f"{w}: say what the engine should change (`proposal`)")
        if r.get("area") != "worked" and str(r.get("workaround", "")).strip().lower() in ("", "see game data", "n/a", "-"):
            bad.append(f"{w}: `workaround` must say what you did (or `none`), not a placeholder")
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
        findings.append({"id": f"F{i}", "key": n.get("key") or slugify(n["text"]), "area": n["area"], "title": n["text"][:110], "severity": 2 if n.get("cost_min", 0) >= 5 else 1, "cost_min": n.get("cost_min", 0),
                         "evidence": "TODO: the command and the message, or the number",
                         "workaround": "" if n["area"] == "worked" else "TODO: how you got past it (or `none`)", "proposal": n.get("fix") or "TODO: the engine change"})
    idea = run["idea"]
    score_lines = "\n".join(f"- {'pass' if s['ok'] else 'FAIL'}: {s['name']}" + (f" ({s['detail']})" if s["detail"] else "") for s in items)
    return f"""# Idea Forge run: {idea['title']} ({today})

- Idea code: `{idea['code']:010d}`; kind {run.get('kind', '2d').upper()}; slug `{run['slug']}`; game `{game_file(run.get('kind', '2d'), run['slug'])}`
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
        text = draft_feedback(run, read_notes(run_dir), score(root, run["slug"], run.get("kind", "2d")), run["date"])
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
    claimed = sum(n.get("cost_min", 0) for n in notes)
    line = f"Notes logged during the run: {len(notes)} ({claimed:g} minutes claimed lost)."
    if run.get("finished") and run.get("started"):
        wall = (run["finished"] - run["started"]) / 60
        line += f" Measured wall time {wall:.1f} min."
        if claimed > wall:
            line += " **The claimed minutes exceed the measured wall time: read `cost_min` as a relative weight, not as minutes.**"
    out.append(line)
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


# ---- the backlog: every finding of every run, grouped by key, with what was done about it -----------------------------------------------------------------

def store_dir():
    d = os.path.join(state_dir(), "feedback")
    os.makedirs(d, exist_ok=True)
    return d


def store_feedback(path):
    """Keep a copy of a run's feedback on this machine the moment it exists: the backlog must not depend on a PR having been merged."""
    if path and os.path.isfile(path):
        shutil.copy(path, os.path.join(store_dir(), os.path.basename(path)))


def repo_texts(ref="origin/main"):
    """{file name: text} of the feedback files (and fixes.json) merged on `ref`; {} when this checkout has no such ref."""
    out = {}
    p = subprocess.run(["git", "-C", ROOT, "ls-tree", "-r", "--name-only", ref, "--", FEEDBACK_DIR + "/"], capture_output=True, text=True)
    if p.returncode:
        return out
    for path in p.stdout.split():
        if path.endswith(".md") or path.endswith("fixes.json"):
            q = subprocess.run(["git", "-C", ROOT, "show", f"{ref}:{path}"], capture_output=True, text=True)
            if q.returncode == 0:
                out[os.path.basename(path)] = q.stdout
    return out


def all_sources(ref="origin/main"):
    """Feedback files merged on main plus every file in the local store (the store wins: it has runs whose PR is still open or was never merged)."""
    texts = repo_texts(ref)
    for name in sorted(os.listdir(store_dir())):
        if name.endswith(".md"):
            texts[name] = rd(os.path.join(store_dir(), name))
    return texts


def rows_from(texts):
    rows = []
    for name in sorted(texts):
        if not name.endswith(".md") or name.startswith("DIGEST"):
            continue
        try:
            found = read_findings(texts[name])
        except ValueError:
            continue
        for f in found:
            if isinstance(f, dict) and f.get("title"):
                rows.append(dict(f, run=name[:-3], date=name[:10], key=f.get("key") or f"{name[:-3]}:{f.get('id', '?')}"))
    return rows


def read_fixes(texts):
    try:
        data = json.loads(texts.get("fixes.json", "[]"))
    except ValueError:
        return []
    return [f for f in data if isinstance(f, dict) and f.get("key")]


def build_backlog(rows, fixes, improvements=None):
    """One group per key. status: `open`; `in-progress` (a fix PR is open); `addressed` (a fix is merged on main); `recurring` (reported again in a run dated after the fix);
    `partial` (a merged fix did only part of it: the record says what is `remaining`, and the key stays in the queue); `rejected` (its fix PR was closed unmerged).
    Ranked: worst severity, then how many runs hit it, then minutes lost."""
    improvements = improvements or {}
    covered = {}
    for f in fixes:
        covered[f["key"]] = f
        for ref in f.get("covers", []):
            covered[ref] = f
    groups = {}
    for r in rows:
        if r.get("area") == "worked":
            continue
        g = groups.setdefault(r["key"], {"key": r["key"], "rows": []})
        g["rows"].append(r)
    out = []
    for key, g in groups.items():
        rs = g["rows"]
        worst = max(rs, key=lambda x: (x.get("severity", 0), x.get("cost_min", 0)))
        fix = covered.get(key) or next((covered[f"{x['run']}:{x['id']}"] for x in rs if f"{x['run']}:{x['id']}" in covered), None)
        imp = improvements.get(key) or next((improvements[f"{x['run']}:{x['id']}"] for x in rs if f"{x['run']}:{x['id']}" in improvements), None)
        status = "open"
        if fix:
            status = "recurring" if any(x["date"] > fix.get("date", "0000-00-00") for x in rs) else ("partial" if fix.get("partial") else "addressed")
        elif imp and imp.get("status") == "open":
            status = "in-progress"
        elif imp and imp.get("status") == "closed":
            status = "rejected"
        out.append({"key": key, "status": status, "area": worst.get("area"), "title": worst["title"], "severity": max(x.get("severity", 0) for x in rs),
                    "runs": sorted({x["run"] for x in rs}), "minutes": sum(x.get("cost_min", 0) for x in rs),
                    "proposals": sorted({str(x.get("proposal", "")).strip() for x in rs if str(x.get("proposal", "")).strip() and not str(x.get("proposal", "")).startswith("(")}),
                    "evidence": [f"{x['run']}: {str(x.get('evidence', ''))[:200]}" for x in rs][:4], "fix": fix, "pr": (imp or {}).get("pr", "")})
    out.sort(key=lambda g: (-g["severity"], -len(g["runs"]), -g["minutes"], g["key"]))
    return out


def backlog_text(groups, statuses=("open", "recurring", "partial", "in-progress"), limit=40):
    shown = [g for g in groups if g["status"] in statuses][:limit]
    if not shown:
        return "The engine backlog is empty: nothing open.\n"
    out = [f"# Engine backlog ({len(shown)} of {len(groups)} issues; ranked by severity, then runs, then minutes lost)\n"]
    for g in shown:
        out.append(f"## `{g['key']}` [{g['status']}] {g['area']}, severity {g['severity']}, {len(g['runs'])} run(s), {g['minutes']:g} min\n\n{g['title']}\n")
        if g["proposals"]:
            out.append("Proposed engine change(s):\n" + "\n".join(f"- {x}" for x in g["proposals"][:3]) + "\n")
        out.append("Evidence:\n" + "\n".join(f"- {x}" for x in g["evidence"]) + "\n")
        if g["fix"]:
            if g["status"] == "recurring":
                out.append(f"Earlier fix: {g['fix'].get('summary', '')} ({g['fix'].get('date', '')}); reported again since.\n")
            elif g["status"] == "partial":
                out.append(f"Earlier PARTIAL fix: {g['fix'].get('summary', '')} ({g['fix'].get('date', '')}). Still to do: {g['fix'].get('remaining', '(not recorded)')}\n")
            else:
                out.append(f"Fix: {g['fix'].get('summary', '')}\n")
        if g["pr"]:
            out.append(f"Fix PR: {g['pr']}\n")
    return "\n".join(out)


def current_backlog():
    texts = all_sources()
    st = load_state()
    return build_backlog(rows_from(texts), read_fixes(texts), st.get("improvements", {}))


def known_issues_section(groups, limit=12):
    """What the next agent is told so it does not re-discover the same problems: it reuses the key and adds evidence instead."""
    shown = [g for g in groups if g["status"] in ("open", "recurring", "partial")][:limit]
    if not shown:
        return ""
    lines = "\n".join(f"- `{g['key']}` [{g['area']}, severity {g['severity']}, {len(g['runs'])} run(s)]{' RECURRING after a fix' if g['status'] == 'recurring' else (' (partly fixed; the rest is still open)' if g['status'] == 'partial' else '')}: {g['title'][:110]}" for g in shown)
    return f"""## Known issues (already in the engine backlog)

If you hit one of these, log it with the SAME key (`note ... --key KEY`) and your own evidence and minutes: that is what ranks it. Do not spend time diagnosing it again. A new problem gets a new key.

{lines}

"""


def cmd_backlog(a):
    groups = current_backlog()
    if a.json:
        print(json.dumps(groups, indent=1))
    else:
        print(backlog_text(groups, statuses=tuple(a.status.split(","))))


def autofill_feedback(run_dir, run, root):
    """Make this run's feedback file valid without the agent: its notes and the measurements are real, whatever prose is missing says so. Feedback must arrive
    even when the agent timed out, ran out of budget, or never wrote it (the failed runs are the most useful ones)."""
    path = feedback_path(root, run)
    kind = run.get("kind", "2d")
    if os.path.exists(path):
        text = rd(path)
    else:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        text = draft_feedback(run, read_notes(run_dir), run.get("score") or score(root, run["slug"], kind), run["date"])
    try:
        rows = read_findings(text)
    except ValueError:
        rows = []
    fixed = []
    for i, r in enumerate(rows, 1):
        if not isinstance(r, dict):
            continue
        for k, v in list(r.items()):
            if isinstance(v, str) and "TODO" in v:   # an unfilled draft field is an empty field, repaired below
                r[k] = ""
        r.setdefault("id", f"F{i}")
        if r.get("area") not in AREAS:
            r["area"] = "tooling"
        r.setdefault("title", NOT_DONE)
        r.setdefault("severity", 1)
        if r.get("severity") not in (1, 2, 3):
            r["severity"] = 1
        if not isinstance(r.get("cost_min"), (int, float)):
            r["cost_min"] = 0
        if not str(r.get("evidence", "")).strip():
            r["evidence"] = str(r["title"])
        if r["area"] != "worked":
            if str(r.get("workaround", "")).strip().lower() in ("", "see game data", "n/a", "-"):
                r["workaround"] = "(none recorded)"
            if not str(r.get("proposal", "")).strip():
                r["proposal"] = "(none given)"
        if not KEY_RE.match(str(r.get("key", ""))):
            r["key"] = slugify(str(r["title"]))
        fixed.append(r)
    if not fixed:
        fixed = [{"id": "F1", "key": "run-ended-without-findings", "area": "tooling", "title": "The run ended without any logged finding", "severity": 2, "cost_min": 0,
                  "evidence": "no `note` was logged and the feedback file had no findings", "workaround": "(none recorded)", "proposal": "check the transcript: did the agent run out of budget or time before it started logging?"}]
    block = "```json findings\n" + json.dumps(fixed, indent=1) + "\n```"
    text = FINDINGS_RE.sub(lambda _m: block, text) if FINDINGS_RE.search(text) else text + "\n## Findings\n\n" + block + "\n"
    text = re.sub(r'TODO[^\n"]*', NOT_DONE, text)   # what is left is prose
    for section in ("## Idea fit", "## What worked"):
        if section not in text:
            text += f"\n{section}\n\n{NOT_DONE}\n"
    if AUTO_BANNER not in text:
        head, _, rest = text.partition("\n")
        text = head + "\n\n" + AUTO_BANNER + "\n" + rest
    wr(path, text)
    measurements(run_dir, run, root)
    return path


# ---- run: worktree + headless agent ----------------------------------------------------------------------------------------------------------------

def git(root, *args, check=True):
    p = subprocess.run(["git", "-C", root] + list(args), capture_output=True, text=True)
    if check and p.returncode:
        sys.exit(f"git {' '.join(args)}: {p.stderr.strip()}")
    return p.stdout.strip()


def make_worktree(slug, kind="2d"):
    """A second checkout on its own branch, seeded from this checkout's build (scripts/dev worktree), next to this one."""
    return make_named_worktree(f"idea-{kind}-{slug}")


def make_fix_worktree(name):
    return make_named_worktree(name)


def make_named_worktree(name):
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


def do_run(a, kind, code=None):
    """One agent run for one idea of `kind`: worktree, brief, agent, score, measurements. Returns {run, row, status, feedback_problems}; `status` is `built` when the game
    exists and passes its validation and verify, `incomplete` when it does not, `prepared` after --dry-run."""
    code = code if code is not None else next_codes(1)[0]
    idea = make_idea(code, not a.no_story)
    today = datetime.date.today().isoformat()
    print(card(idea) + f"\n[{kind.upper()}]", file=sys.stderr)
    owned = not getattr(a, "workdir", None)
    if not owned:
        wt, branch = os.path.abspath(a.workdir), git(os.path.abspath(a.workdir), "rev-parse", "--abbrev-ref", "HEAD")
    else:
        wt, branch = make_worktree(idea["slug"], kind)
    run_dir = os.path.join(wt, RUN_DIRNAME)
    os.makedirs(run_dir, exist_ok=True)
    exclude = git(wt, "rev-parse", "--git-path", "info/exclude")
    exclude = exclude if os.path.isabs(exclude) else os.path.join(wt, exclude)
    os.makedirs(os.path.dirname(exclude), exist_ok=True)
    if RUN_DIRNAME + "/" not in (rd(exclude) if os.path.exists(exclude) else ""):
        wr(exclude, RUN_DIRNAME + "/\n", "a")
    try:
        known = known_issues_section(current_backlog())
    except Exception as e:   # the backlog is a help, never a reason not to run
        print(f"(no known-issues list: {e})", file=sys.stderr)
        known = ""
    prompt = brief(idea, today, f"python3 {os.path.abspath(__file__)}", kind, known)
    run = {"idea": idea, "slug": idea["slug"], "kind": kind, "date": today, "branch": branch, "worktree": wt, "model": a.model or "default", "started": time.time(),
           "engine_revision": git(wt, "rev-parse", "--short=12", "origin/main", check=False), "budget_usd": a.budget}
    jdump(run, os.path.join(run_dir, "run.json"))
    wr(os.path.join(run_dir, "brief.md"), prompt)
    st = load_state()
    st["runs"] = [r for r in st["runs"] if not (r["code"] == code and r["slug"] == idea["slug"] and r.get("kind", "2d") == kind)]   # a re-run replaces its own row
    row = {"code": code, "slug": idea["slug"], "kind": kind, "title": idea["title"], "date": today, "worktree": wt, "branch": branch, "status": "running", "owned": owned}
    st["runs"].append(row)
    save_state(st)
    print(f"run directory {run_dir}\nbranch {branch}", file=sys.stderr)
    if a.dry_run:
        print(f"dry run: would run {' '.join(agent_argv('<brief.md>', a))} in {wt}", file=sys.stderr)
        return {"run": run, "row": row, "status": "prepared", "feedback_problems": []}
    env = dict(os.environ, IDEA_FORGE_RUN=run_dir, RED_TRACE=os.path.join(run_dir, "trace.jsonl"), IDEA_FORGE_HOME=state_dir())
    rc = stream_agent(agent_argv(prompt, a), wt, env, os.path.join(run_dir, "session.jsonl"), a.timeout_min * 60, a.quiet)
    run["finished"] = time.time()
    run["agent_exit"] = rc
    run["score"] = score(wt, idea["slug"], kind)
    jdump(run, os.path.join(run_dir, "run.json"))
    measurements(run_dir, run, wt)
    fb = feedback_path(wt, run)
    fb_problems = check_feedback_text(rd(fb)) if os.path.exists(fb) else ["no feedback file was written"]
    status = "built" if all(s_["ok"] for s_ in run["score"][:3]) else "incomplete"
    store_feedback(fb)
    st = load_state()
    for r in st["runs"]:
        if r["code"] == code and r["slug"] == idea["slug"] and r.get("kind", "2d") == kind:
            r.update(status=status, minutes=round((run["finished"] - run["started"]) / 60, 1), feedback_ok=not fb_problems, notes=len(read_notes(run_dir)))
            row = r
    save_state(st)
    print(f"\n{'pass' if not fb_problems else 'FAIL'}: feedback file" + ("" if not fb_problems else "\n  " + "\n  ".join(fb_problems)))
    for s_ in run["score"]:
        print(f"{'pass' if s_['ok'] else 'FAIL'}: {s_['name']}" + (f" ({s_['detail']})" if s_["detail"] else ""))
    return {"run": run, "row": row, "status": status, "feedback_problems": fb_problems}


def cmd_run(a):
    if not shutil.which(a.claude) and not os.path.isfile(a.claude):
        sys.exit(f"`{a.claude}` not found: install Claude Code, or pass --claude PATH")
    res = do_run(a, a.kind, a.code)
    if res["status"] != "prepared":
        print(f"\nreview the work in {res['row']['worktree']}, then: python3 scripts/idea_forge.py ship {a.kind}/{res['run']['slug']}")


# ---- ship: commit exactly the game and the feedback, push, PR --------------------------------------------------------------------------------------

class ShipRefused(Exception):
    """The run is not shippable (it changed more than its game and feedback, or the feedback is unfinished): the message says why."""


def find_run(ref):
    """The most recent ledger row for `slug`, `kind/slug` or a worktree path."""
    for r in reversed(load_state()["runs"]):
        kind = r.get("kind", "2d")
        if ref in (r["slug"], f"{kind}/{r['slug']}") or (r.get("worktree") and os.path.abspath(r["worktree"]) == os.path.abspath(ref)):
            return r
    sys.exit(f"no run `{ref}` in the ledger (`idea_forge.py ledger`)")


def remove_worktree(wt):
    """Drop a worktree this tool made (the branch stays: it is pushed). Failure is harmless, so it is not an error."""
    subprocess.run(["git", "-C", ROOT, "worktree", "remove", "--force", wt], capture_output=True, text=True)


def archive_run(row):
    """Keep what is not in the PR (the brief, notes, command trace, transcript) under the state directory, then remove the worktree: each one holds a 1+ GB seeded build."""
    wt = row.get("worktree")
    if not wt or not os.path.isdir(wt):
        return
    dest = os.path.join(state_dir(), "runs", f"{row['date']}-{row.get('kind', '2d')}-{row['slug']}")
    src = os.path.join(wt, RUN_DIRNAME)
    if os.path.isdir(src):
        shutil.copytree(src, dest, dirs_exist_ok=True)
    remove_worktree(wt)
    st = load_state()
    for r in st["runs"]:
        if r["code"] == row["code"] and r["slug"] == row["slug"] and r.get("kind", "2d") == row.get("kind", "2d"):
            r.update(archived=dest, worktree_removed=True)
    save_state(st)


def do_ship(r, no_push=False, keep=False, feedback_only=False, hold=False):
    """Commit ONLY the game and its feedback (only the feedback when the game did not build), push, open the PR; returns the PR url ('' with no_push).
    Raises ShipRefused with the reason when not shippable."""
    wt, slug, kind = r["worktree"], r["slug"], r.get("kind", "2d")
    stray = [p for p in changed_paths(wt) if not allowed_path(p, slug, kind) and not p.startswith(RUN_DIRNAME + "/")]
    if stray:
        raise ShipRefused("the run changed more than the game and its feedback (a missing engine feature is a finding, not a patch):\n  " + "\n  ".join(stray))
    run = load_run(os.path.join(wt, RUN_DIRNAME))
    fb = feedback_path(wt, run)
    bad = check_feedback_text(rd(fb)) if os.path.exists(fb) else ["no feedback file"]
    if bad:
        raise ShipRefused("the feedback is not ready:\n  " + "\n  ".join(bad))
    store_feedback(fb)
    paths = [os.path.relpath(fb, wt)] + ([] if feedback_only else [game_file(kind, slug).rsplit("/", 1)[0] + "/" if kind == "3d" else game_file(kind, slug)])
    git(wt, "add", "--", *paths)
    title = f"Idea Forge {kind.upper()} run: {run['idea']['title']} (" + ("feedback only: the game did not build" if feedback_only else "feedback and a game built around one mechanic") + ")"
    msg = f"{title}\n\nGenerated by scripts/idea_forge.py from idea code {run['idea']['code']:010d}; the feedback file is {FEEDBACK_DIR}/.\n\nCo-Authored-By: Claude <noreply@anthropic.com>"
    p = subprocess.run(["git", "-C", wt, "commit", "-q", "-m", msg], capture_output=True, text=True)
    if p.returncode and "nothing to commit" not in (p.stdout + p.stderr):
        raise ShipRefused("git commit failed (is user.name / user.email set?):\n" + (p.stderr or p.stdout).strip())
    if no_push:
        print(f"committed on {r['branch']} in {wt}; not pushed (--no-push)")
        return ""
    git(wt, "push", "-q", "-u", "origin", r["branch"])
    body = (f"## What\n" + ("The engine feedback of a run whose game did not build" if feedback_only else f"A {kind.upper()} game built around one mechanic") +
            f", forged by `scripts/idea_forge.py` (code `{run['idea']['code']:010d}`), and the engine feedback its development produced.\n\n"
            f"- Idea: YOU {run['idea']['you']}, BUT {run['idea']['but']}.\n" + ("" if feedback_only else f"- Game: `{game_file(kind, slug)}`\n") + f"- Feedback: `{os.path.relpath(fb, wt)}`\n\n"
            "No engine code changed. The nightly run merges this once CI passes (`idea_forge.py settle`; `config --auto-merge off` or `ship --hold` keeps it for a human); the backlog (`idea_forge.py backlog`) ranks its findings with every earlier run.\n\n"
            "🤖 Generated with [Claude Code](https://claude.com/claude-code)")
    p = subprocess.run(["gh", "pr", "create", "--base", "main", "--head", r["branch"], "--title", title, "--body", body], cwd=wt, capture_output=True, text=True)
    pr = p.stdout.strip().splitlines()[-1] if p.returncode == 0 and p.stdout.strip() else ""
    print(p.stdout.strip() or p.stderr.strip())
    st = load_state()
    for x in st["runs"]:
        if x["slug"] == slug and x.get("kind", "2d") == kind:
            x["status"] = "shipped" if pr else x.get("status", "built")
            x["pr"] = pr
            x["feedback_only"] = feedback_only
            if hold:
                x["hold"] = True
    save_state(st)
    if pr and r.get("owned") and not keep:
        archive_run(r)
    return pr


def cmd_ship(a):
    r = find_run(a.run)
    try:
        do_ship(r, a.no_push, a.keep, hold=a.hold)
    except ShipRefused as e:
        sys.exit("refusing to ship: " + str(e))


# ---- daily: two games a day, one 2D and one 3D, in either order -------------------------------------------------------------------------------------------

def pick_order(rng=None):
    """Today's order: a fresh shuffle each day, so neither dimension is always first."""
    order = list(KINDS)
    (rng or random.SystemRandom()).shuffle(order)
    return order


def day_state(st, date, order=None):
    days = st.setdefault("days", {})
    if date not in days:
        days[date] = {"order": order or pick_order(), "slots": {k: {"status": "pending", "attempts": 0} for k in KINDS}}
    return days[date]


def pending_kinds(day):
    return [k for k in day["order"] if day["slots"][k]["status"] not in ("done", "review") and day["slots"][k]["attempts"] < MAX_ATTEMPTS]


def pid_alive(pid):
    """Is a process with this pid running? (`os.kill(pid, 0)` is a harmless probe on POSIX but TERMINATES the process on Windows, so Windows asks the OS instead.)"""
    if os.name == "nt":
        import ctypes
        handle = ctypes.windll.kernel32.OpenProcess(0x1000, False, int(pid))   # PROCESS_QUERY_LIMITED_INFORMATION
        if handle:
            ctypes.windll.kernel32.CloseHandle(handle)
            return True
        return False
    try:
        os.kill(pid, 0)
        return True
    except (OSError, ValueError):
        return False


class Lock:
    """One daily run at a time (cron could start the second before the first finished): a pid file, stale when its process is gone."""

    def __init__(self):
        self.path = os.path.join(state_dir(), "daily.lock")

    def __enter__(self):
        if os.path.exists(self.path):
            try:
                pid = int(rd(self.path).strip())
            except ValueError:
                pid = 0
            if pid and pid_alive(pid):
                sys.exit(f"another daily run is active (pid {pid}); not starting a second")
        wr(self.path, str(os.getpid()))
        return self

    def __exit__(self, *exc):
        try:
            os.remove(self.path)
        except OSError:
            pass


def produce(a, kind):
    """Make one game of `kind` and get its feedback to the engine repo in every case: the agent's own file when it is sound, otherwise the CLI completes it (the notes and the
    measurements are real) and ships it, with the game when the game built and without it when it did not. Returns (slot status, detail)."""
    res = do_run(a, kind)
    if res["status"] == "prepared":
        return "pending", "dry run"
    row, run = res["row"], res["run"]
    wt = row["worktree"]
    built = res["status"] == "built"
    run_dir = os.path.join(wt, RUN_DIRNAME)
    completed = False
    if res["feedback_problems"]:
        autofill_feedback(run_dir, run, wt)
        completed = True
    if a.no_ship:
        store_feedback(feedback_path(wt, run))
        return ("done" if built else "failed"), f"{'built' if built else 'did not build'}, not shipped (worktree kept): {wt}"
    try:
        pr = do_ship(row, keep=a.keep, feedback_only=not built)
    except ShipRefused as e:
        store_feedback(feedback_path(wt, run))
        return "review", "not shipped (worktree kept; the feedback is saved locally): " + str(e)[:200]
    note = " (feedback auto-completed)" if completed else ""
    if built:
        return "done", (pr or "committed (no PR url)") + note
    return "failed", f"the game does not build ({'; '.join(s_['name'] for s_ in run['score'] if not s_['ok'])[:120]}); feedback shipped: {pr}{note}"


def cmd_daily(a):
    date = a.date or datetime.date.today().isoformat()
    if a.order:
        order = a.order.split(",")
        if sorted(order) != sorted(KINDS):
            sys.exit("--order must name each kind once, e.g. 3d,2d")
    else:
        order = None
    if a.dry_run:
        st = load_state()
        day = day_state(st, date, order)
        save_state(st)   # the preview is truthful: today's order is fixed from here on
        print(f"{date}: order {', '.join(day['order'])}; " + ", ".join(f"{k} {day['slots'][k]['status']} (attempts {day['slots'][k]['attempts']})" for k in day["order"]))
        todo = pending_kinds(day)
        print("would run: " + (", ".join(todo[:1] if a.next else todo) or "nothing, the day is complete"))
        return
    if not shutil.which(a.claude) and not os.path.isfile(a.claude):
        sys.exit(f"`{a.claude}` not found: install Claude Code, or pass --claude PATH")
    with Lock():
        sys.exit(1 if run_daily(a, date, order) else 0)


def run_daily(a, date, order=None):
    """Today's pending games, ONE AT A TIME in the day's order (the next starts only when the previous is finished, shipped and cleaned up). Returns how many failed."""
    st = load_state()
    day = day_state(st, date, order)
    save_state(st)
    todo = pending_kinds(day)
    if getattr(a, "next", False):
        todo = todo[:1]
    print(f"{date}: order {', '.join(day['order'])}; running {', '.join(todo) or 'nothing (the day is complete)'}")
    failed = 0
    for kind in todo:
        status, detail = produce(a, kind)
        st = load_state()
        slot = day_state(st, date)["slots"][kind]
        slot.update(status=status, detail=detail, attempts=slot["attempts"] + 1, finished=round(time.time()))
        save_state(st)
        failed += status == "failed"
        print(f"{date} {kind}: {status}: {detail}")
    return failed


# ---- improve: the nightly engine-improvement agent ---------------------------------------------------------------------------------------------------------

FIX_FORBIDDEN = (".github/", "Cargo.lock", "scripts/idea_forge.py", "scripts/test_idea_forge.py", FEEDBACK_DIR + "/")


def fix_path_ok(path):
    """What an improvement run may change: the engine, its tests, scripts and docs, and the fix record; never CI, dependencies, this tool or earlier feedback."""
    if path == FIXES_FILE:
        return True
    return not (any(path == f or path.startswith(f) for f in FIX_FORBIDDEN) or path == "Cargo.toml" or path.endswith("/Cargo.toml"))


def has_regression_test(root, paths):
    if any(p.startswith("tests/") or re.match(r"scripts/test_[^/]+\.py$", p) for p in paths):
        return True
    out = subprocess.run(["git", "-C", root, "diff", "origin/main"], capture_output=True, text=True).stdout
    return any(l.startswith("+") and "#[test]" in l for l in out.splitlines())


SETTLE_POLL_SEC = 60   # how often `settle` asks GitHub again while checks are running (tests set it to 0)
FAIL_CONCLUSIONS = {"FAILURE", "CANCELLED", "TIMED_OUT", "ACTION_REQUIRED", "STARTUP_FAILURE", "ERROR"}


def auto_merge_enabled():
    """Merging after CI is the default; `config --auto-merge off` or IDEA_FORGE_AUTO_MERGE=0 turns it off, and `ship --hold` exempts one PR."""
    if os.environ.get("IDEA_FORGE_AUTO_MERGE", "").lower() in ("0", "off", "false", "no"):
        return False
    return bool(load_state().get("config", {}).get("auto_merge", True))


def pr_status(url):
    """GitHub's view of a PR as a dict (state, mergeable, checks, files), or None when `gh` cannot say."""
    p = subprocess.run(["gh", "pr", "view", url, "--json", "state,mergeable,statusCheckRollup,files"], capture_output=True, text=True)
    if p.returncode:
        return None
    try:
        return json.loads(p.stdout)
    except ValueError:
        return None


def checks_verdict(rollup):
    """`passed` only when there is at least one check, every check has finished, none failed and one really passed; `failed` when any failed; otherwise `pending` (running, or no
    check has appeared yet: no checks never means passed)."""
    if not rollup:
        return "pending"
    passed = False
    pending = False
    for c in rollup:
        status = (c.get("status") or "COMPLETED").upper()
        outcome = (c.get("conclusion") or c.get("state") or "").upper()
        if outcome in FAIL_CONCLUSIONS:
            return "failed"
        if status != "COMPLETED" or outcome in ("PENDING", "EXPECTED", ""):
            pending = True
        elif outcome == "SUCCESS":
            passed = True
    if pending:
        return "pending"
    return "passed" if passed else "pending"


def tracked_prs():
    """The PRs this tool opened that are not finished: only these are ever merged by `settle`, never an arbitrary PR."""
    st = load_state()
    seen, out = set(), []
    for r in st["runs"]:
        url = r.get("pr", "")
        if url.startswith("http") and r.get("status") == "shipped" and r.get("merge") not in ("merged", "closed") and not r.get("hold") and url not in seen:
            seen.add(url)
            out.append({"url": url, "type": "game", "slug": r["slug"], "kind": r.get("kind", "2d"), "feedback_only": bool(r.get("feedback_only"))})
    for key, imp in st.get("improvements", {}).items():
        url = imp.get("pr", "")
        if url.startswith("http") and imp.get("status") == "open" and url not in seen:
            seen.add(url)
            out.append({"url": url, "type": "fix"})
    return out


def record_pr(url, **fields):
    st = load_state()
    for r in st["runs"]:
        if r.get("pr") == url:
            r.update(fields)
    for imp in st.get("improvements", {}).values():
        if imp.get("pr") == url:
            if fields.get("merge") == "merged":
                imp["status"] = "merged"
            elif fields.get("merge") == "closed":
                imp["status"] = "closed"
            imp["merge"] = fields.get("merge", imp.get("merge"))
    save_state(st)


def pr_paths_ok(pr, files):
    """Re-checked from GitHub's own file list at merge time (not from what the run said it changed): a game PR touches only its game and feedback; a fix PR nothing off limits."""
    paths = [f.get("path", "") for f in files]
    if not paths:
        return False
    if pr["type"] == "fix":
        return all(fix_path_ok(p) for p in paths)
    return all(allowed_path(p, pr["slug"], pr["kind"]) for p in paths) and (not pr["feedback_only"] or all(p.startswith(FEEDBACK_DIR + "/") for p in paths))


def settle_one(pr):
    """Merge one tracked PR if, and only if, it is open, mergeable, within its path limits and every CI check has passed. Returns (verdict, detail)."""
    url = pr["url"]
    s_ = pr_status(url)
    if s_ is None:
        return "unknown", "`gh` could not read the PR"
    state = s_.get("state", "")
    if state == "MERGED":
        record_pr(url, merge="merged")
        return "merged", "already merged"
    if state == "CLOSED":
        record_pr(url, merge="closed")
        return "closed", "closed without merging"
    if not auto_merge_enabled():
        return "held", "auto-merge is off"
    if not pr_paths_ok(pr, s_.get("files") or []):
        record_pr(url, merge="refused-paths")
        return "refused", "the PR changes files outside what its kind may change: left for a human"
    verdict = checks_verdict(s_.get("statusCheckRollup") or [])
    if verdict == "failed":
        record_pr(url, merge="ci-failed")
        return "ci-failed", "a check failed: left open"
    if verdict == "pending" or s_.get("mergeable") == "UNKNOWN":
        return "pending", "checks are still running"
    if s_.get("mergeable") == "CONFLICTING":
        record_pr(url, merge="conflict")
        return "conflict", "the PR conflicts with main: left open"
    p = subprocess.run(["gh", "pr", "merge", url, "--merge"], capture_output=True, text=True)
    if p.returncode:
        record_pr(url, merge="merge-error")
        return "merge-error", (p.stderr or p.stdout).strip()[:200]
    record_pr(url, merge="merged")
    return "merged", "CI passed"


def settle(wait_min=0):
    """Merge every tracked PR whose checks have passed; keep asking until none is pending or `wait_min` minutes have passed (0 = one look)."""
    deadline = time.time() + wait_min * 60
    while True:
        results = [(pr, *settle_one(pr)) for pr in tracked_prs()]
        for pr, verdict, detail in results:
            if verdict != "pending":
                print(f"settle: {verdict}: {pr['url']} ({detail})")
        if not any(v == "pending" for _, v, _ in results) or time.time() >= deadline:
            for pr, verdict, detail in results:
                if verdict == "pending":
                    print(f"settle: still pending: {pr['url']} (it stays tracked: the next run settles it)")
            return results
        time.sleep(SETTLE_POLL_SEC)


def refresh_improvements():
    """Learn whether the fix PRs opened earlier were merged or closed (a merged fix is on main in fixes.json; a closed one is not retried)."""
    st = load_state()
    for key, imp in st.get("improvements", {}).items():
        if imp.get("status") != "open" or not imp.get("pr"):
            continue
        s_ = pr_status(imp["pr"])
        state = (s_ or {}).get("state", "")
        if state in ("MERGED", "CLOSED"):
            imp["status"] = "merged" if state == "MERGED" else "closed"
    save_state(st)


def cmd_settle(a):
    with Lock():
        settle(a.wait_min)


def cmd_config(a):
    st = load_state()
    if a.auto_merge:
        st.setdefault("config", {})["auto_merge"] = a.auto_merge == "on"
        save_state(st)
    cfg = {"auto_merge": auto_merge_enabled()}
    print(json.dumps(cfg))


def improve_brief(groups, today, cli):
    return f"""# Idea Forge engine-improvement run ({today})

You are an AI engineer on RedEngine, a data-driven game engine. In the last days AI agents built games on it and filed feedback about what slowed them down or was missing. Your job now is to fix ONE of those problems in the
engine so the next agent loses less time. This is the loop that makes the engine better every night: a fix that does not remove the friction is worth nothing, and a fix that is not tested will not be trusted.

{backlog_text(groups[:6])}
## Do this

1. Choose ONE issue: the most valuable one you can fix completely and safely in this session. A fix can be code, a clearer error message that names the fix, a line in `describe`/the reference, a recipe, a template: whatever
   removes the friction (prefer making the problem disappear over documenting it). If an issue needs a design decision a human should make, do not build it: choose another, or if none is safe make no change and say why.
2. Follow this repository's own workflow (`CLAUDE.md`): `scripts/dev start "<the fix>"`, `scripts/dev red context <words>` for the files and tests involved, `scripts/dev iterate` while you edit. Keep the change small.
3. Add a regression test that fails without your change (a Rust test, or `scripts/test_*.py`). A fix without a test is refused.
4. If the change is a decision a maintainer should be able to find later, add an ADR (`scripts/dev red adr new "Title" --summary "..."`).
5. Record the fix: append an object to the JSON list in `{FIXES_FILE}` (create the file as `[]` first if it is missing):
   `{{"key": "<the backlog key>", "covers": ["<run>:<id>", "..."], "date": "{today}", "summary": "<one sentence>"}}`. `covers` lists findings that have no key of their own. This is how the backlog knows the problem was fixed
   and notices if it comes back. **If you fixed only PART of the issue, add `"partial": true` and `"remaining": "<what is still to do>"`**: otherwise the whole key is marked solved and the rest is forgotten.
6. `scripts/dev preflight` and `scripts/dev affected --quick` must pass (the CLI re-runs both from scratch, without the result cache, after you finish).

Do not change: `.github/`, `Cargo.toml`/`Cargo.lock` (no new dependencies), `scripts/idea_forge.py`, `scripts/test_idea_forge.py`, or any existing file under `{FEEDBACK_DIR}/` except `fixes.json`. Do not `git push`, open a PR, merge or publish.

Your own friction is feedback too: log it as you go with `{cli} note "<what happened>" --area <{'|'.join(AREAS)}> --key <kebab-name> --cost-min <minutes>`.

Your last message: the key you fixed, what changed (three lines), the test, and what you chose not to do and why.
"""


def run_gates(wt, tail=40):
    """The checks the CLI runs itself on an improvement (the agent's word is not enough): preflight, then the affected quick scope WITHOUT the result cache (a cache hit is the agent's own
    earlier run). IDEA_FORGE_GATES overrides (JSON list of argv lists)."""
    env = os.environ.get("IDEA_FORGE_GATES")
    cmds = json.loads(env) if env else [[os.path.join(wt, "scripts", "dev"), "preflight"], [os.path.join(wt, "scripts", "dev"), "affected", "--quick", "--no-cache"]]
    log = []
    for cmd in cmds:
        p = subprocess.run(cmd, cwd=wt, capture_output=True, text=True, timeout=3600)
        out = (p.stdout + p.stderr).strip().splitlines()
        log.append(f"$ {' '.join(os.path.basename(c) if i == 0 else c for i, c in enumerate(cmd))} -> {p.returncode}\n" + "\n".join(out[-tail:]))
        if p.returncode:
            return False, "\n\n".join(log)
    return True, "\n\n".join(log)


def last_result_text(session):
    text = ""
    if os.path.exists(session):
        for line in rd(session).splitlines():
            try:
                ev = json.loads(line)
            except ValueError:
                continue
            if ev.get("type") == "result" and isinstance(ev.get("result"), str):
                text = ev["result"]
    return text


def do_improve(a):
    """One engine-improvement run. Returns (status, detail): `nothing` (empty backlog), `no-change`, `refused`, `no-test`, `not-recorded`, `gates-failed` (all keep the worktree),
    or `shipped`."""
    refresh_improvements()
    groups = current_backlog()
    cands = [g for g in groups if g["status"] in ("open", "recurring", "partial")]
    if getattr(a, "key", None):
        cands = [g for g in groups if g["key"] == a.key]
    if not cands:
        print("improve: the backlog has nothing open")
        return "nothing", "the backlog has nothing open"
    today = datetime.date.today().isoformat()
    name = f"engine-fix-{today}-{datetime.datetime.now().strftime('%H%M%S')}-{secrets.token_hex(2)}"   # unique even for two runs in one second
    if a.dry_run:
        print(f"improve: would work on one of: {', '.join(g['key'] for g in cands[:6])}")
        return "pending", "dry run"
    wt, branch = make_fix_worktree(name)
    run_dir = os.path.join(wt, RUN_DIRNAME)
    os.makedirs(run_dir, exist_ok=True)
    exclude = git(wt, "rev-parse", "--git-path", "info/exclude")
    exclude = exclude if os.path.isabs(exclude) else os.path.join(wt, exclude)
    os.makedirs(os.path.dirname(exclude), exist_ok=True)
    if RUN_DIRNAME + "/" not in (rd(exclude) if os.path.exists(exclude) else ""):
        wr(exclude, RUN_DIRNAME + "/\n", "a")
    prompt = improve_brief(cands, today, f"python3 {os.path.abspath(__file__)}")
    run = {"kind": "fix", "slug": name, "date": today, "branch": branch, "worktree": wt, "started": time.time(), "candidates": [g["key"] for g in cands[:6]]}
    jdump(run, os.path.join(run_dir, "run.json"))
    wr(os.path.join(run_dir, "brief.md"), prompt)
    env = dict(os.environ, IDEA_FORGE_RUN=run_dir, RED_TRACE=os.path.join(run_dir, "trace.jsonl"), IDEA_FORGE_HOME=state_dir())
    session = os.path.join(run_dir, "session.jsonl")
    stream_agent(agent_argv(prompt, a), wt, env, session, a.timeout_min * 60, a.quiet)
    changed = [p for p in changed_paths(wt) if not p.startswith(RUN_DIRNAME + "/")]

    def keep(status, detail):
        print(f"improve: {status}: {detail}")
        return status, detail

    if not changed:
        archive_run({"worktree": wt, "date": today, "kind": "fix", "slug": name, "code": 0})
        return "no-change", "the agent made no change (it found nothing it could fix safely)"
    bad = [p for p in changed if not fix_path_ok(p)]
    if bad:
        return keep("refused", f"changed paths that are off limits ({wt} kept): " + ", ".join(bad))
    if not has_regression_test(wt, changed):
        return keep("no-test", f"no regression test in the change ({wt} kept)")
    try:
        fixes = json.loads(rd(os.path.join(wt, FIXES_FILE))) if os.path.exists(os.path.join(wt, FIXES_FILE)) else []
    except ValueError:
        fixes = []
    known = {g["key"] for g in groups}
    old = {f.get("key") for f in read_fixes(repo_texts())}
    new = [f for f in fixes if isinstance(f, dict) and f.get("key") and f["key"] not in old]
    if not new or not any(f["key"] in known or f.get("covers") for f in new):
        return keep("not-recorded", f"{FIXES_FILE} has no new entry for a backlog key ({wt} kept)")
    ok, log = run_gates(wt)
    if not ok:
        return keep("gates-failed", f"the CLI's own checks failed ({wt} kept):\n" + log[-1500:])
    keys = [f["key"] for f in new]
    summary = "; ".join(f.get("summary", "") for f in new)[:120] or keys[0]
    title = f"Engine: {summary} (Idea Forge finding {', '.join(keys)})"
    result = last_result_text(session)
    detail = "\n\n".join(f"### `{g['key']}` [{g['status']}] {g['title']}\n" + "\n".join(f"- {e}" for e in g["evidence"]) for g in cands if g["key"] in keys or any(f"{r}:" in " ".join(f.get('covers', [])) for f in new for r in g["runs"]))
    body = (f"## What\nAn engine fix chosen by the nightly improvement agent from the Idea Forge backlog (`scripts/idea_forge.py improve`).\n\n{detail}\n\n## The agent's summary\n{result or '(none)'}\n\n"
            f"## Checks the CLI ran itself\n```\n{log[-3000:]}\n```\n\nHosted CI runs on this PR; the nightly run merges it once every check has passed (`idea_forge.py settle`; `config --auto-merge off` keeps it for a human). Merging records the fix in `{FIXES_FILE}`; the backlog then watches for the problem coming back.\n\n"
            "🤖 Generated with [Claude Code](https://claude.com/claude-code)")
    git(wt, "add", "-A", "--", ".")
    git(wt, "reset", "-q", "--", RUN_DIRNAME)
    msg = f"{title}\n\nCo-Authored-By: Claude <noreply@anthropic.com>"
    p = subprocess.run(["git", "-C", wt, "commit", "-q", "-m", msg], capture_output=True, text=True)
    if p.returncode:
        return keep("refused", "git commit failed (is user.name / user.email set?): " + (p.stderr or p.stdout).strip()[:200])
    if a.no_ship:
        return keep("done", f"committed on {branch} in {wt}; not pushed (--no-ship)")
    git(wt, "push", "-q", "-u", "origin", branch)
    p = subprocess.run(["gh", "pr", "create", "--base", "main", "--head", branch, "--title", title, "--body", body], cwd=wt, capture_output=True, text=True)
    pr = p.stdout.strip().splitlines()[-1] if p.returncode == 0 and p.stdout.strip() else ""
    st = load_state()
    for k in keys:
        st.setdefault("improvements", {})[k] = {"pr": pr, "branch": branch, "date": today, "status": "open"}
    save_state(st)
    archive_run({"worktree": wt, "date": today, "kind": "fix", "slug": name, "code": 0})
    print(f"improve: shipped {pr}")
    return "shipped", pr


def cmd_improve(a):
    if not shutil.which(a.claude) and not os.path.isfile(a.claude):
        sys.exit(f"`{a.claude}` not found: install Claude Code, or pass --claude PATH")
    with Lock():
        subprocess.run(["git", "-C", ROOT, "fetch", "-q", "origin", "main"], capture_output=True, text=True)
        status, detail = do_improve(a)
        sys.exit(0 if status in ("shipped", "nothing", "no-change", "pending", "done") else 1)


def cmd_nightly(a):
    """The whole night, under one lock and strictly one agent at a time: today's games (feedback always delivered), then one engine improvement from the backlog they fed."""
    date = a.date or datetime.date.today().isoformat()
    order = a.order.split(",") if a.order else None
    if order and sorted(order) != sorted(KINDS):
        sys.exit("--order must name each kind once, e.g. 3d,2d")
    if not a.dry_run and not shutil.which(a.claude) and not os.path.isfile(a.claude):
        sys.exit(f"`{a.claude}` not found: install Claude Code, or pass --claude PATH")
    with Lock():
        if not a.dry_run:
            if a.no_merge:
                os.environ["IDEA_FORGE_AUTO_MERGE"] = "0"
            settle(0)   # yesterday's PRs whose checks have finished: merged before today's worktrees are cut from main
            subprocess.run(["git", "-C", ROOT, "fetch", "-q", "origin", "main"], capture_output=True, text=True)
        failed = 0
        if not a.skip_games:
            ns = argparse.Namespace(**vars(a))
            ns.next = False
            if a.dry_run:
                st = load_state()
                day = day_state(st, date, order)
                save_state(st)
                print(f"{date}: games order {', '.join(day['order'])}; would run: {', '.join(pending_kinds(day)) or 'nothing'}")
            else:
                failed = run_daily(ns, date, order)
        result = ("skipped", "")
        if not a.skip_improve:
            ns = argparse.Namespace(**vars(a))
            ns.budget, ns.timeout_min, ns.key = a.improve_budget, a.improve_timeout_min, None
            result = do_improve(ns)
        merged = []
        if not a.dry_run:
            merged = [pr["url"] for pr, verdict, _ in settle(a.settle_timeout_min) if verdict == "merged"]
        st = load_state()
        st.setdefault("nights", {})[date] = {"games_failed": failed, "improve": result[0], "detail": str(result[1])[:300], "merged": merged, "at": round(time.time())}
        save_state(st)
        print(f"{date}: nightly done: games failed {failed}; improvement {result[0]}; merged {len(merged)} PR(s)")
        sys.exit(0)


def cron_lines(times, budget, model=None, improve_budget=None, command="nightly"):
    """One cron line per time of day. cron has no PATH worth the name, so the current one is baked in (it must reach claude, gh, git, python3 and cargo)."""
    home = state_dir()
    path = os.environ.get("PATH", "")
    cargo = os.path.join(os.path.expanduser("~"), ".cargo", "bin")
    if os.path.isdir(cargo) and cargo not in path.split(os.pathsep):
        path = path + os.pathsep + cargo
    cmd = (f"cd {shlex.quote(ROOT)} && PATH={shlex.quote(path)} IDEA_FORGE_HOME={shlex.quote(home)} {shlex.quote(sys.executable)} "
           f"{shlex.quote(os.path.join(HERE, 'idea_forge.py'))} {command}" + (" --next" if command == "daily" else "") + f" --budget {budget:g}"
           + (f" --improve-budget {improve_budget:g}" if command == "nightly" and improve_budget else "") + (f" --model {shlex.quote(model)}" if model else "")
           + f" >> {shlex.quote(os.path.join(home, 'nightly.log'))} 2>&1")
    lines = []
    for t in times:
        m = re.fullmatch(r"(\d{1,2}):(\d{2})", t.strip())
        if not m or int(m.group(1)) > 23 or int(m.group(2)) > 59:
            sys.exit(f"bad time `{t}`: use HH:MM, 24-hour")
        lines.append(f"{int(m.group(2))} {int(m.group(1))} * * * {cmd}  {CRON_MARK}")
    return lines


def read_crontab():
    p = subprocess.run(["crontab", "-l"], capture_output=True, text=True)
    return p.stdout.splitlines() if p.returncode == 0 else []


def write_crontab(lines):
    p = subprocess.run(["crontab", "-"], input="\n".join(lines) + "\n", capture_output=True, text=True)
    if p.returncode:
        sys.exit("crontab failed: " + p.stderr.strip())


def cmd_schedule(a):
    mine = [l for l in read_crontab() if CRON_MARK in l] if shutil.which("crontab") else []
    if a.uninstall:
        write_crontab([l for l in read_crontab() if CRON_MARK not in l])
        print(f"removed {len(mine)} idea-forge cron line(s)")
        return
    lines = cron_lines(a.times.split(","), a.budget, a.model, a.improve_budget, a.command)
    if not a.install:
        print("\n".join(lines))
        per = f"games one at a time (up to ${a.budget:g} each) then one engine improvement (up to ${a.improve_budget:g})" if a.command == "nightly" else f"one game (up to ${a.budget:g}) per run"
        print(f"\n(not installed: add --install to put these in your crontab. Each run: {per}; PRs only, nothing is merged; --uninstall removes them)")
        if mine:
            print(f"\ninstalled now: {len(mine)} line(s)")
        return
    if not shutil.which("crontab"):
        sys.exit("no `crontab` on this machine: use the lines above in your scheduler of choice (systemd timer, launchd, Task Scheduler)")
    write_crontab([l for l in read_crontab() if CRON_MARK not in l] + lines)
    print(f"installed {len(lines)} cron line(s); log: {os.path.join(state_dir(), 'nightly.log')}")


# ---- ledger and digest -------------------------------------------------------------------------------------------------------------------------------

def cmd_ledger(a):
    runs = load_state()["runs"]
    if not runs:
        print("no runs yet: `idea_forge.py run`")
    for r in runs:
        print(f"{r['date']}  {r.get('kind', '2d')}  {r['code']:010d}  {r['slug']:<28} {r.get('status', '?'):<10} {r.get('minutes', '-'):>5} min  notes {r.get('notes', '-')}  feedback {'ok' if r.get('feedback_ok') else 'no'}  {r.get('pr', '')} {r.get('merge', '')}")


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


def kind_of(run_name):
    """`<date>-3d-<slug>` is a 3D run; `<date>-2d-<slug>` and the files from before kinds existed are 2D."""
    return "3d" if re.match(r"\d{4}-\d{2}-\d{2}-3d-", run_name) else "2d"


def digest_text(rows):
    """Findings from every run grouped by area, ranked by (severity, minutes, how many runs hit it): the engine backlog this loop produces."""
    if not rows:
        return "No feedback files yet.\n"
    runs = sorted({r["run"] for r in rows})
    kinds = {k: sum(1 for x in runs if kind_of(x) == k) for k in KINDS}
    out = [f"# Idea Forge digest\n\n{len(runs)} run(s) ({kinds['2d']} 2D, {kinds['3d']} 3D), {len(rows)} findings, {sum(r.get('cost_min', 0) for r in rows):g} minutes reported lost.\n"]
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
    s.add_argument("--kind", choices=KINDS, default="2d")
    s.add_argument("--no-story", action="store_true")
    s.set_defaults(f=cmd_brief)

    s = sub.add_parser("run")
    s.add_argument("--kind", choices=KINDS, default="2d")
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
    s.add_argument("--key", help="a stable kebab-case name for the problem; reuse the key of a known issue (see the brief) when it is the same one")
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
    s.add_argument("run", help="the slug, kind/slug or worktree of a run in the ledger")
    s.add_argument("--no-push", action="store_true")
    s.add_argument("--keep", action="store_true", help="keep the worktree after shipping (default: archive the run's notes and transcript, then remove it)")
    s.add_argument("--hold", action="store_true", help="do not auto-merge this PR once CI passes: leave it for a human")
    s.set_defaults(f=cmd_ship)

    s = sub.add_parser("daily")
    s.add_argument("--next", action="store_true", help="only the next pending game (cron runs this twice a day)")
    s.add_argument("--no-ship", action="store_true", help="build and keep the worktree; do not push or open PRs")
    s.add_argument("--keep", action="store_true")
    s.add_argument("--budget", type=float, default=DAILY_BUDGET_USD, help="USD cap per game")
    s.add_argument("--model")
    s.add_argument("--timeout-min", type=int, default=150)
    s.add_argument("--claude", default="claude")
    s.add_argument("--date", help="YYYY-MM-DD (default today)")
    s.add_argument("--order", help="force today's order, e.g. 3d,2d (default: shuffled once per day)")
    s.add_argument("--dry-run", action="store_true", help="print today's order and what would run, run nothing")
    s.add_argument("--quiet", action="store_true")
    s.set_defaults(f=cmd_daily, no_story=False, dry_run=False, workdir=None)

    def agent_opts(s, budget_default):
        s.add_argument("--budget", type=float, default=budget_default, help="USD cap per agent session")
        s.add_argument("--model")
        s.add_argument("--timeout-min", type=int, default=150)
        s.add_argument("--claude", default="claude")
        s.add_argument("--dry-run", action="store_true")
        s.add_argument("--quiet", action="store_true")

    s = sub.add_parser("improve", help="one engine fix chosen from the backlog (a PR; `settle` merges it once CI passes)")
    agent_opts(s, IMPROVE_BUDGET_USD)
    s.add_argument("--key", help="fix this backlog key instead of the best-ranked one")
    s.add_argument("--no-ship", action="store_true", help="commit on the branch, do not push or open a PR")
    s.set_defaults(f=cmd_improve)

    s = sub.add_parser("nightly", help="today's games one at a time, then one engine improvement: what cron runs")
    agent_opts(s, DAILY_BUDGET_USD)
    s.add_argument("--improve-budget", type=float, default=IMPROVE_BUDGET_USD)
    s.add_argument("--improve-timeout-min", type=int, default=120)
    s.add_argument("--no-ship", action="store_true")
    s.add_argument("--keep", action="store_true")
    s.add_argument("--date")
    s.add_argument("--order", help="force today's game order, e.g. 3d,2d (default: shuffled once per day)")
    s.add_argument("--skip-games", action="store_true")
    s.add_argument("--skip-improve", action="store_true")
    s.add_argument("--settle-timeout-min", type=int, default=120, help="how long to wait at the end for tonight's PRs' checks, then merge those that passed (the next run settles the rest)")
    s.add_argument("--no-merge", action="store_true", help="open the PRs but do not merge any, this run")
    s.set_defaults(f=cmd_nightly, no_story=False, workdir=None)

    s = sub.add_parser("settle", help="merge the tool's own PRs whose CI has passed")
    s.add_argument("--wait-min", type=int, default=0, help="keep waiting up to this long for running checks")
    s.set_defaults(f=cmd_settle)

    s = sub.add_parser("config", help="show or change settings")
    s.add_argument("--auto-merge", choices=("on", "off"), help="merge the tool's PRs once CI passes (default on)")
    s.set_defaults(f=cmd_config)

    s = sub.add_parser("schedule")
    s.add_argument("--times", default="06:00", help="HH:MM,HH:MM local times of day (default 06:00)")
    s.add_argument("--budget", type=float, default=DAILY_BUDGET_USD, help="USD cap per game")
    s.add_argument("--improve-budget", type=float, default=IMPROVE_BUDGET_USD, help="USD cap for the engine-improvement step")
    s.add_argument("--command", choices=("nightly", "daily"), default="nightly", help="nightly = both games one at a time, then an engine improvement (default); daily = one game per run (--next)")
    s.add_argument("--model")
    g = s.add_mutually_exclusive_group()
    g.add_argument("--install", action="store_true")
    g.add_argument("--uninstall", action="store_true")
    s.set_defaults(f=cmd_schedule)

    s = sub.add_parser("backlog")
    s.add_argument("--json", action="store_true")
    s.add_argument("--status", default="open,recurring,in-progress", help="comma-separated: open, recurring, in-progress, addressed, rejected")
    s.set_defaults(f=cmd_backlog)

    sub.add_parser("ledger").set_defaults(f=cmd_ledger)
    s = sub.add_parser("digest")
    s.add_argument("--write", action="store_true")
    s.set_defaults(f=cmd_digest)

    a = ap.parse_args(argv)
    a.f(a)


if __name__ == "__main__":
    main()
