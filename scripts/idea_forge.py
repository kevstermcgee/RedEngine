#!/usr/bin/env python3
"""Idea Forge: forge a game idea, have an AI build it with RedEngine, and file the engine feedback that run produced (docs/IDEA_FORGE.md).

  idea_forge.py idea [--n N] [--code C] [--no-story] [--json]     the next idea nobody has been given yet (or the one behind CODE)
  idea_forge.py brief [--code C]                                   the prompt an agent gets for an idea
  idea_forge.py run [--kind 2d|3d] [--code C] [--budget USD]       forge an idea, make a worktree, run a headless `claude -p` agent in it, score and measure the result
  idea_forge.py daily [--next] [--no-ship] [--budget USD]          today's two games, one 2D and one 3D in a random order each day: run what is still pending, ship each as a PR
  idea_forge.py schedule [--times 09:30,17:30] [--install]         the cron entries that run `daily --next` twice a day (printed unless --install; --uninstall removes them)
  idea_forge.py note "TEXT" --area A [--cost-min N] [--fix TEXT]   (the agent) log one snag at the moment it happens
  idea_forge.py wiki --init | --preview | --check                 (the agent) the game's wiki page: rules, story, characters, gallery, why it was made; `publish` draws its pictures
  idea_forge.py feedback --init | --check | --finalize             (the agent, then the CLI) the feedback file: draft it from the notes, validate it, add the measurements
  idea_forge.py ship [RUN] [--hold]                                commit ONLY the game, its wiki page and its feedback, push the branch, open a PR on RedEngine, then archive the run and remove its worktree (refuses a game whose page cannot be published)
  idea_forge.py settle [--wait-min N]                              merge the tool's own PRs whose CI has passed and that carry their game's wiki page (the default; `config --auto-merge off` stops it)
  idea_forge.py publish [--dry-run]                                put the merged games that RedEngineGames does not have yet there (nightly does this; `config --publish off` stops it); a game it cannot publish is recorded in the ledger with why
  idea_forge.py ledger                                             every run so far
  idea_forge.py digest [--write]                                   the engine backlog: all feedback files, grouped and ranked

Standard library only. State (the seed and the position in the walk, the ledger) is in $IDEA_FORGE_HOME, else the user's state directory, so no idea is handed out twice.
A game agent never changes engine code: a missing feature is a finding, not a patch (`ship` refuses a branch that touches anything else). The tool merges ONLY its own PRs, and only after
every CI check has passed (`settle`; `config --auto-merge off` or `ship --hold` stops it).
"""
import argparse
import hashlib
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
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, HERE)
import proc_supervisor  # noqa: E402  (every child process here runs under a deadline it cannot miss; see that module)
import game_generators  # noqa: E402  (a game's generator, kept beside it, still writes the committed game)
FORGE_DIR = os.path.join(ROOT, "examples", "2d", "idea-forge")
FEEDBACK_DIR = "docs/analysis/idea-forge"   # always forward slashes: git reports paths that way on every OS, and Windows accepts them
# Everyone else's feedback notes (a maintainer's AI session, a person): the same findings block, so one problem hit anywhere ranks as one key
# (scripts/feedback.py, docs/feedback/README.md). Fixes are still recorded only in FEEDBACK_DIR/fixes.json.
NOTES_DIR = "docs/feedback"
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


AREAS = ("discovery", "docs", "cli", "diagnostics", "format", "runtime", "verify", "publish", "perf", "tooling", "ui", "network", "idea-fit", "worked")
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


def state_home():
    """Where this machine keeps its state; nothing is created (a reader that must not leave a directory behind asks this, not `state_dir`)."""
    d = os.environ.get("IDEA_FORGE_HOME")
    if not d:
        base = os.environ.get("XDG_STATE_HOME") or (os.environ.get("LOCALAPPDATA") if os.name == "nt" else None) or os.path.join(os.path.expanduser("~"), ".local", "state")
        d = os.path.join(base, "idea-forge")
    return d


def state_dir():
    d = state_home()
    os.makedirs(d, exist_ok=True)
    return d


def ledger_runs():
    """The ledger's runs for a reader that only looks: None when this machine has no ledger (nothing is created, nothing is written)."""
    p = os.path.join(state_home(), "state.json")
    try:
        runs = jload(p).get("runs")
    except (OSError, ValueError, AttributeError):
        return None
    return runs if isinstance(runs, list) else None


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
- Writing rules, levels or entities with a script is fine; keep the script: it is `examples/2d/{slug}/build.py` (helpers beside it), written there from the start, never in /tmp, and committed with
  the game (`examples/2d/honeybound/` is the pattern). It uses only the standard library and paths relative to itself (`Path(__file__).parent`), and running it again rewrites the game byte for byte
  (`python3 scripts/game_generators.py check examples/2d/{slug}` says so; `feedback --check` runs it). Change the game by editing the script and re-running it, not by editing its output.
- Start with `scripts/dev start "<one line>"`, then `scripts/dev red describe 2d`; use `scripts/dev red search "<question>"` and `recipe` before inventing anything. (`scripts/dev red` is the engine CLI.)
- Prove it with the engine, not by reading your own file: `validate`, `sim --only NAME --every S`, `verify`, and `frame G out.png` then LOOK at the picture. Scenarios must include one that exercises the mechanic and asserts
  its effect (not just that the game starts), and a win and a lose path unless the design has neither. Finish with `play2d G --max-ticks 120 --mute` under a virtual display if you have one.
"""

BUILD_3D = """- Work only in this worktree. The game is a first-person 3D RedEngine scene: `examples/3d/{slug}/{slug}.json` (a map with `player`, `spawns`, `objects`, `zones`, `vars`, `rules`, `ui`, `checks`), plus anything it needs beside it under
  `examples/3d/{slug}/` (for example `audio/*.json` scores). Declare `checks.lint` in the scene (`ignore` lists the lint codes a deliberate pit or prop trips): the run is judged by `verify`, which honors it. Game logic is data (`rules`), never Rust. Do not change engine code (`src/`, `crates/`, `tests/`, `scripts/`) or any other doc: a missing feature is a finding,
  not a patch. Work around it in game data and record the workaround.
- Writing rules or objects with a script is fine; keep the script: it is `examples/3d/{slug}/build.py` (helpers beside it), written there from the start, never in /tmp, and committed with the scene
  (`examples/2d/honeybound/` is the pattern). It uses only the standard library and paths relative to itself (`Path(__file__).parent`), and running it again rewrites the scene byte for byte
  (`python3 scripts/game_generators.py check examples/3d/{slug}` says so; `feedback --check` runs it). Change the game by editing the script and re-running it, not by editing its output.
- Start with `scripts/dev start "<one line>"`, then `scripts/dev red describe --brief`, `describe rules` and `describe scene`; use `search "<question>"`, `recipe` (`recipe coin_run --new examples/3d/{slug}/{slug}.json` is a complete
  small game to start from) and `build --example` for room layouts. (`scripts/dev red` is the engine CLI.)
- Make it look like its own game, not like every other game on this engine (`describe style`). The floor: a `style` block with a palette of 6-8 roles of YOUR colours (not the
  defaults it lists), and its light and sky; at least {own_prefabs} prefabs of your own in the scene's `prefabs` (the things that carry the idea; `@role` colours, `material.preset`); its own
  sounds (a score under `examples/3d/{slug}/audio/`). Use `catalog <word>` for everything else: shared prefabs take your palette's colours. LOOK at what you made:
  `scripts/dev red catalog --scene examples/3d/{slug}/{slug}.json --local --sheet out/own.png` (your own prefabs, in your style) and `scripts/dev red style-report examples/3d/{slug}/{slug}.json`.
- Prove it with the engine, not by reading your own file: `lint`, `plan` / `tour` pictures (LOOK at the PNGs), `reach` checks for places the player must get to, `sim` with `checks.sim` scripted playthroughs, and `verify`.
  Scenarios must include one that exercises the mechanic and asserts its effect (not just that the game starts), and a win and a lose path unless the design has neither. If `playtest` (pictures from the player's eyes,
  no window) works on this machine use it; if it does not, log why.
- Keep the world small: one or two rooms or a small outdoor area is plenty. The mechanic must be something a first-person player does with their hands, in the world.
"""

STYLE_MIN_ROLES = 6        # the 3D floor: palette roles that are not the default colour
STYLE_MIN_OWN_PREFABS = 3  # the 3D floor: prefabs the game defines itself

POLISH_2D = "- Sound and a saved value (`persist`) where they fit the game; polish after it works."
POLISH_3D = "- Sound (`audio`) where it fits the game; polish after it works."


def brief(idea, today=None, cli=None, kind="2d", known=""):
    """`cli` is how the agent calls this script: the launching checkout's absolute path (the agent's worktree is cut from origin/main, which may not have the script yet)."""
    today = today or datetime.date.today().isoformat()
    cli = cli or "python3 scripts/idea_forge.py"
    slug, code = idea["slug"], idea["code"]
    fb = f"{FEEDBACK_DIR}/{feedback_name(today, kind, slug)}"
    story = f"- Story (optional dressing; use it, bend it or drop it): {idea['story']}\n" if idea["story"] else "- Story: none this time; the mechanic carries the game.\n"
    build_steps = (BUILD_3D if kind == "3d" else BUILD_2D).format(slug=slug, own_prefabs=STYLE_MIN_OWN_PREFABS)
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
The card is a seed, not a spec, and it is random: some cards are odd. YOUR JOB IS A GAME PEOPLE WANT TO PLAY, not a faithful rendering of the card. Fun, uniqueness and clarity (the next section) beat fidelity to the card.
Before building, write a one-paragraph design: THE mechanic in one sentence, the core loop (what the player does every few seconds), why it is fun, how a first-time player learns it, how the game is won and lost.
Keep the spirit of YOU and the most fun reading of BUT. If the literal reading would be awkward, strange or hard to explain, reinterpret it into something a player grasps at once, soften it, or drop the BUT, and say what you changed.
The mechanic must be what the player does most, not a power-up on a generic collector or platformer. Finished and small beats ambitious and broken.
If the mechanic cannot be built faithfully, build the nearest fun subset and say what was cut and what the engine lacked.

## Make it fun, unique and clear (this decides whether the run counts)

Earlier games from this tool were judged unfun, awkward, strange and confusing. Do not repeat that. The test is a stranger who has never seen the card: in the first ten seconds they know what to do, within a minute they are enjoying it, and when it ends they want one more go.

- **Fun.** One verb the player does constantly must feel good by itself (responsive, with an immediate visible and audible reaction: a sound, a flash, a shake, a number popping up). Add a small decision, risk or skill in that verb, a goal that pulls the player forward, and
  a difficulty that ramps: an easy first success within 30 seconds, then harder. A round lasts 3 to 5 minutes. Dying or failing restarts in a second, and the player can see why they lost.
- **Unique.** Before designing, skim what already exists (`ls examples/2d examples/3d`, and the `description` of a few) and do not rebuild a mechanic that is there. Make something that would be worth describing to a friend in one sentence, with a twist that is
  the game, not decoration on a collect-the-coins loop. Still, one strong, simple idea beats three clever ones.
- **Clear.** No confusing games: the player must never have to guess. Show a title and a one-sentence goal on screen at the start, the controls for the one or two buttons that matter, and keep the goal and progress (score, timer, counter) visible while playing. Teach by doing: the first screen or room
  is safe and shows only the core mechanic, and each further rule is introduced one at a time, never two new things at once. Rules the player cannot see (invisible state, hidden timers, an unexplained meter) do not belong in the game: make every state visible and give every action visible feedback. Prefer
  plain, familiar controls and readable art (strong contrast between the player, hazards and goals) over cleverness. Awkward camera, floaty or slippery controls, unfair deaths and walls of text are bugs: fix them.
- **Check it as a stranger.** After it works, play it (or step through `frame` / `playtest` pictures) using ONLY what is on screen. If you had to remember something from the design paragraph to understand it, add it to the screen. Record in your feedback what you changed after this pass.

## Build it

{build_steps}{polish}
- Time: stop after roughly 90 minutes of work. Do not spend an hour on one obstacle: log it, work around it, move on.

{known}## Write the game's wiki page (it is published with the game, and a game without a valid one is NOT published)

RedEngineGames shows every game as a small wiki: what it is, the story, the rules and controls, the characters with a picture of each, a gallery of different moments, and why it exists. You write that page as data, once the game works. It is committed with the game: `ship` puts `{WIKI_DIR}/{slug}.json` in the same pull request as the game and the feedback, and CHECKS it with `wiki --check` first. A game whose page is missing or fails the check is not shipped and not merged (it could never be published) and waits for a human, so write the page before you stop:

    {cli} wiki --init       writes `{WIKI_DIR}/{slug}.json` from a template: replace every TODO
    {cli} wiki --preview    draws every picture to `out/wiki/`: LOOK at each PNG
    {cli} wiki --check      must pass

- Write for a stranger who has never seen the game, in plain words: no jargon, no engine terms. The overview says what the game is and what makes it different; the story gives the setting (invent one if the card has none); the rules are the real rules of YOUR game, matching what `verify` proves, one per line;
  the controls list every input that matters.
- `characters` are everyone and everything the player meets (the player, enemies, helpers, the key objects), each with a name, a role, a description and a `shot`. `gallery` has at least 3 pictures of DIFFERENT moments (start, mid-play, a win or a loss) with a caption.
- A `shot` says how the engine draws a picture: `{{"t": seconds}}` plus, for a 2D game, `"scenario"` (a scenario name from the game file); for a 3D scene `"eye": "x,y,z"`, `"at": "x,y,z"` and optionally `"fov"` and `"hide"` (object ids) to put the camera on that character. A character picture must actually show that character (a 2D game has one fixed camera: pick the `t` and `scenario` where the character is on screen). Re-draw with `--preview` until each picture shows what its caption says.
- `why_made` is YOU speaking, first person: why you chose to build this particular game from the card, what drew you to it, what you changed and why. It is published next to the game under "Why this game exists", so be honest and specific, not generic.

## Keep a running issue log: this is the point of the run

At the moment you hit friction (not afterwards) run

    {cli} note "<what happened>" --area <{'|'.join(AREAS)}> --key <stable-kebab-name-of-the-problem> --cost-min <minutes it really cost> --fix "<the engine change that would have prevented it>"

Log: a failed command whose message did not say the fix; a feature you needed that is missing (and your workaround); a doc that was wrong, stale or missing; a `search` that found nothing; anything that cost more than two
minutes; every time you read engine source or docs outside `describe`/`search` (path and why; `--area docs` or `discovery`); how the idea fit the engine (`--area idea-fit`: a part of the card the engine could not express and the
missing capability); and what worked well (`--area worked`). One line each, concrete, with the command or message. `--cost-min` is wall-clock minutes you actually lost, not a guess at the engine's importance (the total cannot exceed your session; the CLI compares it with the measured time and says so when it does).

## Finish

1. `{cli} feedback --init` writes `{fb}`, pre-filled from your notes, the engine's command trace and a score. Edit it: rank the findings by what would save the next run the most,
   add evidence and a proposed engine change to each, and fill in "Idea fit" and "What worked".
2. `{cli} feedback --check` must pass.
3. `{cli} wiki --check` must pass (after you have LOOKED at the `wiki --preview` pictures): `ship` runs the same check on `{WIKI_DIR}/{slug}.json` and refuses the game if it fails or the file is missing.
4. Do not `git push`, open a PR, merge or publish: the CLI does that after review. Commit locally or leave the files uncommitted.
5. Your last message is six lines: the game path, the `verify` result, the mechanic in one sentence, why a stranger would find it fun, how a first-time player learns it, and your top three findings (one line).
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
    p = proc_supervisor.run_capture(engine_cmd(root) + args, cwd=root, timeout=timeout)   # a timeout stops the whole tree and raises TimeoutExpired
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
    if kind == "3d" and g.get("checks", {}).get("lint") is not None:
        # a scene that declares `checks.lint` (with `ignore` for deliberate pits) is judged by `verify`, the only command that honors it: bare `lint` exits 1 on those pits
        # and failed every 3D game that had one (2026-10-09 and 10-10), so the game was never shipped
        code, out = run_engine(root, ["verify", game, "--only", "lint", "--no-views"])
        found = next((l for l in out.splitlines() if re.match(r"\s*(PASS|FAIL) +lint\b", l)), "")
        item("lint passes", code == 0 and found.strip().startswith("PASS"), found.strip() or (out.strip().splitlines() or [""])[0])
    else:
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
        # The look (style-report): quality facts, not the build gate. The thresholds are the brief's floor.
        code, out = run_engine(root, ["--json", "style-report", game])
        rep = {}
        try:
            env = json.loads(out[out.index("{"):]) if "{" in out else {}
            rep = env.get("data") if isinstance(env.get("data"), dict) else env
        except ValueError:
            pass
        if code != 0 or not rep:
            item("style-report ran", False, (out.strip().splitlines() or [""])[-1][:120])
        else:
            item(f"a palette of its own (>= {STYLE_MIN_ROLES} roles not the default)", rep.get("palette_custom_roles", 0) >= STYLE_MIN_ROLES,
                 f"{rep.get('palette_custom_roles', 0)} of {rep.get('palette_roles', 0)} role(s); mean distance from the defaults {rep.get('palette_distance', 0):.0f}")
            item(f"prefabs of its own (>= {STYLE_MIN_OWN_PREFABS})", len(rep.get("own_prefabs") or []) >= STYLE_MIN_OWN_PREFABS,
                 f"{len(rep.get('own_prefabs') or [])}: {', '.join(rep.get('own_prefabs') or [])[:80]}; shared catalogue {100 * rep.get('shared_share', 0):.0f}% of {rep.get('placed', 0)} placed")
            item("its own sound files", rep.get("sounds", 0) >= 1, f"{rep.get('sounds', 0)} file(s)")
            item("no default look kept (lint style-*)", not rep.get("defaults_kept"), ", ".join(rep.get("defaults_kept") or []) or "none")
    gen = game_generators.for_game(root, kind, slug)
    if gen:
        ok, secs, problems = game_generators.check_one(*gen, root=root)
        item("its generator rewrites the game byte for byte", ok, f"{gen[0].name} {secs:.1f} s" if ok else "; ".join(problems)[:200])
    return items


def changed_paths(root, base="origin/main"):
    """Paths this branch changed since it left `base`, including uncommitted and untracked files. Measured from the merge-base, not from the tip of `base`: a `main` that moved on while
    the run worked (or while a built run waited) must not show up as hundreds of files the run "changed" (the two stranded runs of 2026-10-12 and 13 each looked like that)."""
    fork = subprocess.run(["git", "-C", root, "merge-base", "HEAD", base], capture_output=True, text=True)
    fork = fork.stdout.strip() if fork.returncode == 0 and fork.stdout.strip() else base
    out = set()
    for args in (["diff", "--name-only", fork], ["diff", "--name-only"], ["ls-files", "--others", "--exclude-standard"]):
        p = subprocess.run(["git", "-C", root] + args, capture_output=True, text=True)
        out.update(l.strip() for l in p.stdout.splitlines() if l.strip())
    return sorted(out)


WIKI_DIR = FEEDBACK_DIR + "/wiki"


def wiki_file(slug):
    """The game's wiki page as its agent writes it: data with `shot` specs instead of pictures; `publish` draws the pictures (RedEngineGames' site/wiki.py defines the page)."""
    return f"{WIKI_DIR}/{slug}.json"


SHOT_KEYS = {"t", "scenario", "eye", "at", "fov", "hide", "hour"}
WIKI_TEXT_MIN = {"overview": 200, "story": 150, "why_made": 100}


def wiki_problems(data):
    """What is wrong with an agent's wiki file, as sentences it can act on (RedEngineGames' site/wiki.py checks the finished page again, with pictures)."""
    if not isinstance(data, dict):
        return ["the wiki file must be a JSON object"]
    ok = lambda v, n=1: isinstance(v, str) and len(v.strip()) >= n   # noqa: E731
    out = [f"`{k}` is missing" + (f" or shorter than {WIKI_TEXT_MIN[k]} characters" if k in WIKI_TEXT_MIN else "") for k in ("tagline", "overview", "story", "goal", "why_made") if not ok(data.get(k), WIKI_TEXT_MIN.get(k, 1))]
    if not (isinstance(data.get("rules"), list) and len(data["rules"]) >= 3 and all(ok(r) for r in data["rules"])):
        out.append("`rules` needs at least 3 non-empty rules")
    c = data.get("controls")
    if not (isinstance(c, list) and c and all(isinstance(x, dict) and ok(x.get("input")) and ok(x.get("action")) for x in c)):
        out.append("`controls` needs at least one {input, action}")

    def shot_bad(x):
        sh = x.get("shot") if isinstance(x, dict) else None
        if not isinstance(sh, dict):
            return "needs a `shot` (how to draw it: {t, scenario, eye, at, fov, hide, hour}; `{}` for the default view)"
        extra = set(sh) - SHOT_KEYS
        return f"has unknown shot keys {sorted(extra)}" if extra else ""
    chars = data.get("characters")
    if not (isinstance(chars, list) and chars):
        out.append("`characters` needs at least one entry: everyone and everything you meet (the player, enemies, helpers, key objects)")
    else:
        for i, x in enumerate(chars):
            if not (isinstance(x, dict) and all(ok(x.get(k)) for k in ("name", "role", "description"))):
                out.append(f"character {i + 1} needs name, role and description")
            elif shot_bad(x):
                out.append(f"character `{x['name']}` {shot_bad(x)}")
    gal = data.get("gallery")
    if not (isinstance(gal, list) and len(gal) >= 3):
        out.append("`gallery` needs at least 3 pictures of different moments")
    else:
        for i, x in enumerate(gal):
            if not (isinstance(x, dict) and ok(x.get("caption"))):
                out.append(f"gallery picture {i + 1} needs a caption")
            elif shot_bad(x):
                out.append(f"gallery picture {i + 1} {shot_bad(x)}")
        if len({json.dumps(x.get("shot"), sort_keys=True) for x in gal if isinstance(x, dict)}) < len(gal):
            out.append("`gallery` repeats a view: each picture needs a different shot")
    return out


def wiki_check(data):
    """`wiki --check`'s whole verdict on a page's data: its problems, and any TODO left in it. The one check the agent's `wiki --check`, `ship` and `publish` all hold a page to."""
    bad = wiki_problems(data)
    if isinstance(data, dict):
        bad += [f"`{k}` still says TODO" for k, v in data.items() if re.search(r"\bTODO\b", json.dumps(v))]   # a word: MASTODON is not a leftover
    return bad


def worktree_wiki_problems(wt, slug, hints=True):
    """Why the wiki page in a run's worktree could not be published, one sentence each, with the way out unless `hints` is off ([] when it is sound). This is `wiki --check` applied where it
    still matters: before the game is shipped, because a merged game with no complete page waits in RedEngine unpublished (RedEngineGames' site build refuses it)."""
    path = os.path.join(wt, wiki_file(slug))
    me = f"python3 {shlex.quote(os.path.abspath(__file__))}"
    where = f"--run {shlex.quote(wt)}"

    def fix(how):
        return f" (fix: {how})" if hints else ""
    if not os.path.isfile(path):
        return [f"no wiki page: {wiki_file(slug)} does not exist" + fix(f"{me} wiki --init {where}; replace every TODO; wiki --preview and LOOK at the pictures; wiki --check")]
    try:
        data = jload(path)
    except ValueError as e:
        return [f"the wiki page {wiki_file(slug)} is not valid JSON: {e}" + fix(f"edit it, then {me} wiki --check {where}")]
    bad = wiki_check(data)
    return [f"the wiki page {wiki_file(slug)} is incomplete: " + "; ".join(bad[:5]) + (f"; and {len(bad) - 5} more" if len(bad) > 5 else "") + fix(f"edit it, then {me} wiki --check {where}")] if bad else []


def game_dir(kind, slug):
    """The folder a game's own sources live in: a 3D game's scene and scores, a 2D game's generator (`examples/2d/<slug>/` beside its one JSON file)."""
    return f"examples/{kind}/{slug}/"


def allowed_path(path, slug, kind="2d"):
    """The only paths a run may change: its game (a 3D game owns its folder; a 2D game its file and the folder of its generator) and its feedback file."""
    prefix = game_prefix(kind, slug)
    own = path == prefix or path.startswith(game_dir(kind, slug))
    return own or path == wiki_file(slug) or (path.startswith(FEEDBACK_DIR + "/") and path.endswith(".md"))


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
        gen = game_generators.for_game(root, run.get("kind", "2d"), run["slug"])
        if gen:
            bad += game_generators.check_one(*gen, root=root)[2]
        for b in bad:
            print("FAIL", b)
        print("feedback OK" if not bad else f"{len(bad)} problem(s)")
        sys.exit(1 if bad else 0)
    elif a.finalize:
        measurements(run_dir, run, root)
        print("measurements written")


def shot_args(shot, kind):
    """`frame` options for one wiki shot."""
    a = []
    if kind == "2d":
        a += ["--t", str(shot.get("t", 1))] + (["--scenario", str(shot["scenario"])] if shot.get("scenario") else [])
    else:
        for k in ("t", "hour", "eye", "at", "fov"):
            if shot.get(k) is not None:
                a += [f"--{k}", str(shot[k])]
        for h in ([shot["hide"]] if isinstance(shot.get("hide"), str) else shot.get("hide") or []):
            a += ["--hide", str(h)]
    return a


def draw_shot(root, scene, out, shot, kind, size):
    """One wiki picture as the engine draws it; True only for a real PNG with something in it (see `render_thumbnail`)."""
    os.makedirs(os.path.dirname(out), exist_ok=True)
    if os.path.exists(out):
        os.remove(out)
    try:
        run_engine(root, ["frame", scene, out, "--size", size] + shot_args(shot, kind), timeout=300)
    except subprocess.TimeoutExpired:
        pass
    if os.path.isfile(out) and open(out, "rb").read(8) == PNG_MAGIC and os.path.getsize(out) > 2000:
        return True
    if os.path.exists(out):
        os.remove(out)
    return False


GALLERY_SIZE, CHARACTER_SIZE = "960x540", "480x360"


def cmd_wiki(a):
    run_dir = find_run_dir(a.run)
    run = load_run(run_dir)
    root, kind, slug = os.path.dirname(run_dir), run.get("kind", "2d"), run["slug"]
    path = os.path.join(root, wiki_file(slug))
    if a.init:
        if os.path.exists(path):
            sys.exit(f"{wiki_file(slug)} exists; edit it")
        os.makedirs(os.path.dirname(path), exist_ok=True)
        jdump(WIKI_TEMPLATE, path)
        print(f"wrote {wiki_file(slug)}: replace every TODO, then `wiki --preview` (LOOK at the pictures) and `wiki --check`")
        return
    if not os.path.exists(path):
        sys.exit(f"{wiki_file(slug)} does not exist: run `wiki --init`")
    try:
        data = jload(path)
    except ValueError as e:
        sys.exit(f"{wiki_file(slug)} is not valid JSON: {e}")
    bad = wiki_check(data)
    if a.preview:
        scene, out = os.path.join(root, game_file(kind, slug)), os.path.join(root, "out", "wiki")
        shots = [(f"character-{i + 1}-{x.get('name', '')}", x, CHARACTER_SIZE) for i, x in enumerate(data.get("characters") or []) if isinstance(x, dict)] + \
                [(f"gallery-{i + 1}", x, GALLERY_SIZE) for i, x in enumerate(data.get("gallery") or []) if isinstance(x, dict)]
        for name, x, size in shots:
            f = os.path.join(out, re.sub(r"[^A-Za-z0-9_.-]+", "-", name) + ".png")
            ok = isinstance(x.get("shot"), dict) and draw_shot(root, scene, f, x["shot"], kind, size)
            print(("drew " + os.path.relpath(f, root)) if ok else f"FAIL {name}: the engine drew nothing (a blank or missing picture)")
            if not ok:
                bad.append(f"{name} cannot be drawn")
        print("now LOOK at each PNG: a character picture must show that character, and each gallery picture a different moment")
    for b in bad:
        print("FAIL", b)
    print("wiki OK" if not bad else f"{len(bad)} problem(s)")
    sys.exit(1 if bad else 0)


WIKI_TEMPLATE = {
    "tagline": "TODO one sentence: what you do in this game",
    "overview": "TODO what the game is, what makes it different, why it is fun (200+ characters; plain words for someone who has never seen it)",
    "story": "TODO the setting and what is going on (150+ characters)",
    "goal": "TODO how you win and how you lose",
    "rules": ["TODO rule 1", "TODO rule 2", "TODO rule 3"],
    "controls": [{"input": "TODO key or button", "action": "TODO what it does"}],
    "characters": [{"name": "TODO", "role": "TODO player / enemy / helper / key object", "description": "TODO what it is and does", "shot": {}}],
    "tips": ["TODO optional tip"],
    "gallery": [{"caption": "TODO a moment of play", "shot": {"t": 1}}, {"caption": "TODO another moment", "shot": {"t": 6}}, {"caption": "TODO a third moment", "shot": {"t": 12}}],
    "why_made": "TODO why YOU chose to make this particular game from the card: what you found interesting or fun in it, what you changed and why (100+ characters, first person)",
}


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
        keep = game_dir(run.get("kind", "2d"), run["slug"])
        lost = scripts_not_kept(session, root, keep)
        if lost:
            out.append(f"Python scripts written outside the game's folder, so not kept with it (the brief asks for `{keep}build.py`): " + ", ".join(f"`{x}`" for x in lost) + ".")
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


def scripts_not_kept(session, root, keep):
    """The `.py` files the agent wrote (Write, or a `cat > x.py` heredoc) outside `keep` (the game's folder, relative to the worktree `root`), from the transcript:
    a generator in /tmp is lost when the run ends, and one elsewhere in the worktree is a stray path `ship` refuses."""
    found = []
    for line in rd(session).splitlines():
        try:
            ev = json.loads(line)
        except ValueError:
            continue
        msg = ev.get("message") if isinstance(ev.get("message"), dict) else {}
        for c in msg.get("content") if isinstance(msg.get("content"), list) else []:
            if not isinstance(c, dict) or c.get("type") != "tool_use":
                continue
            inp = c.get("input") if isinstance(c.get("input"), dict) else {}
            paths = [inp.get("file_path", "")] if c.get("name") == "Write" else re.findall(r"cat\s*>\s*(\S+\.py)\b", inp.get("command", "")) if c.get("name") == "Bash" else []
            for p in paths:
                if not p.endswith(".py") or "idea_forge" in p:
                    continue
                full = os.path.normpath(p if os.path.isabs(p) else os.path.join(root, p))
                if not full.startswith(os.path.normpath(os.path.join(root, keep)) + os.sep) and full not in found:
                    found.append(full)
    return found


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
    """{file name: text} of the feedback files (and fixes.json) merged on `ref`, Idea Forge runs and everyone's notes (NOTES_DIR) alike; {} when this
    checkout has no such ref."""
    out = {}
    p = subprocess.run(["git", "-C", ROOT, "ls-tree", "-r", "--name-only", ref, "--", FEEDBACK_DIR + "/", NOTES_DIR + "/"], capture_output=True, text=True)
    if p.returncode:
        return out
    for path in p.stdout.split():
        if path.startswith(NOTES_DIR + "/") and (os.path.basename(path) == "README.md" or not path.endswith(".md")):
            continue
        if path.endswith(".md") or path == FIXES_FILE:
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


def rows_from(texts, skipped=None):
    """Every finding of every feedback text, as rows. A text whose findings block is missing or malformed is skipped (one bad note must not stop the backlog), but not silently:
    pass a list as `skipped` and it gets (name, why) for each, which `digest` and `feedback.py check` print."""
    rows = []
    for name in sorted(texts):
        if not name.endswith(".md") or name.startswith("DIGEST"):
            continue
        try:
            found = read_findings(texts[name])
        except ValueError as e:
            if skipped is not None:
                skipped.append((name, str(e)))
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


def current_backlog(skipped=None):
    texts = all_sources()
    st = load_state()
    return build_backlog(rows_from(texts, skipped), read_fixes(texts), st.get("improvements", {}))


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
    skipped = []
    groups = current_backlog(skipped)
    if skipped:
        print(skipped_line(skipped), file=sys.stderr)   # stderr: `--json` output stays parseable
    if a.export:
        print("export " + export_feedback_note(groups))
        return
    if a.json:
        print(json.dumps(groups, indent=1))
    else:
        print(backlog_text(groups, statuses=tuple(a.status.split(","))))


EXPORT_NAME = "idea-forge-feedback.md"
EXPORT_HEADER = """# Idea Forge feedback for the engine (generated by scripts/idea_forge.py; do not edit)

Findings from AI agents that build games on RedEngine, one entry per problem (`key`), ranked by severity, then how many independent runs hit it, then minutes lost. The evidence comes from real runs and
the proposed engine change is the agent's own: treat both as evidence to verify. When a problem is fixed on main, appending `{"key": "<key>", "date": "YYYY-MM-DD", "summary": "<one sentence>"}` to
`docs/analysis/idea-forge/fixes.json` (add `"partial": true, "remaining": "..."` if only part was fixed) makes tomorrow's file drop it, and flags it `recurring` if a later run hits it again.

"""


def export_path():
    """Where the backlog is written for the owner's existing nightly job: a `*feedback*.md` file directly in the workspace, which that job's feedback gate already watches."""
    return os.environ.get("IDEA_FORGE_EXPORT") or os.path.join(os.path.dirname(ROOT), EXPORT_NAME)


def export_feedback_note(groups=None):
    """Write the open backlog to `export_path()` when it changed (an unchanged file keeps its modification time, so the nightly's gate is not woken for nothing). Returns the verdict."""
    groups = current_backlog() if groups is None else groups
    text = EXPORT_HEADER + backlog_text(groups, statuses=("open", "recurring", "partial"))
    path = export_path()
    if os.path.exists(path) and rd(path) == text:
        return f"unchanged: {path}"
    os.makedirs(os.path.dirname(path), exist_ok=True)
    wr(path, text)
    return f"updated: {path}"


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


def print_tool_call(line, start):
    """One line on stderr for a tool call in an agent's stream-json output, so a human can watch (anything else in the stream is ignored)."""
    try:
        ev = json.loads(line)
    except ValueError:
        return
    for blk in ev.get("message", {}).get("content", []) if ev.get("type") == "assistant" and isinstance(ev.get("message"), dict) else []:
        if isinstance(blk, dict) and blk.get("type") == "tool_use":
            inp = blk.get("input", {})
            what = inp.get("command") or inp.get("file_path") or inp.get("pattern") or ""
            print(f"[{(time.monotonic() - start) / 60:5.1f}m] {blk.get('name')}: {str(what).splitlines()[0][:110] if what else ''}", file=sys.stderr)


def stream_agent(argv, cwd, env, session_path, timeout, quiet, cancel=None):
    """Run the agent under a deadline, keep its whole transcript, and print one line per tool call so a human can watch (`quiet`: no lines; the deadline holds either way).

    The deadline is monotonic and does not depend on the agent printing anything (`proc_supervisor`): at `timeout` seconds the agent's whole process tree is stopped, SIGTERM then SIGKILL
    (Windows: `taskkill /T /F`), within `proc_supervisor.overrun_bound()` seconds. What it printed up to then stays in `session_path`. `cancel` (a `threading.Event`) stops it the same way. Ctrl-C and
    SIGTERM stop the tree too, then end the program as usual.

    Returns the outcome dict: `status` is one of completed, failed (non-zero exit), timeout, cancelled, spawn_error, with `returncode`, `elapsed_s`, `reason`, `tail` and more (the same dict is written
    next to the transcript as `agent_end.json`)."""
    start = time.monotonic()
    end_record = os.path.join(os.path.dirname(os.path.abspath(session_path)), "agent_end.json")

    def record(outcome):
        jdump(dict(outcome), end_record)
        if outcome["status"] != "completed":
            print(f"agent {outcome['status']}: {outcome['reason']} ({outcome['output_bytes']} bytes of transcript kept in {session_path})", file=sys.stderr)

    with open(session_path, "wb") as out, proc_supervisor.exit_on_sigterm():
        return proc_supervisor.supervise(argv, cwd=cwd, env=env, timeout=timeout, log=out, cancel=cancel, report=record,
                                         on_line=None if quiet else (lambda line: print_tool_call(line, start)))


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
    try:   # what players said about the published games (docs/player-reviews), so the next game does more of what they enjoyed
        import player_reviews
        known += player_reviews.players_section(ROOT)
    except Exception as e:
        print(f"(no player reviews: {e})", file=sys.stderr)
    prompt = brief(idea, today, f"python3 {os.path.abspath(__file__)}", kind, known)
    run = {"idea": idea, "slug": idea["slug"], "kind": kind, "date": today, "branch": branch, "worktree": wt, "model": a.model or "default", "started": time.time(),
           "engine_revision": git(wt, "rev-parse", "--short=12", "origin/main", check=False), "budget_usd": a.budget}
    jdump(run, os.path.join(run_dir, "run.json"))
    wr(os.path.join(run_dir, "brief.md"), prompt)
    st = load_state()
    st["runs"] = [r for r in st["runs"] if not (isinstance(r, dict) and r.get("code") == code and r.get("slug") == idea["slug"] and r.get("kind", "2d") == kind)]   # a re-run replaces its own row
    row = {"code": code, "slug": idea["slug"], "kind": kind, "title": idea["title"], "date": today, "worktree": wt, "branch": branch, "status": "running", "owned": owned}
    st["runs"].append(row)
    save_state(st)
    print(f"run directory {run_dir}\nbranch {branch}", file=sys.stderr)
    if a.dry_run:
        print(f"dry run: would run {' '.join(agent_argv('<brief.md>', a))} in {wt}", file=sys.stderr)
        return {"run": run, "row": row, "status": "prepared", "feedback_problems": []}
    env = dict(os.environ, IDEA_FORGE_RUN=run_dir, RED_TRACE=os.path.join(run_dir, "trace.jsonl"), IDEA_FORGE_HOME=state_dir())
    ended = stream_agent(agent_argv(prompt, a), wt, env, os.path.join(run_dir, "session.jsonl"), a.timeout_min * 60, a.quiet)
    run["finished"] = time.time()
    run["agent_exit"] = ended["returncode"]
    run["agent_status"] = ended["status"]   # completed | failed | timeout | cancelled | spawn_error: a timeout is not a failure, and neither is the same as the agent finishing
    run["score"] = score(wt, idea["slug"], kind)
    jdump(run, os.path.join(run_dir, "run.json"))
    measurements(run_dir, run, wt)
    fb = feedback_path(wt, run)
    fb_problems = check_feedback_text(rd(fb)) if os.path.exists(fb) else ["no feedback file was written"]
    status = "built" if all(s_["ok"] for s_ in run["score"][:3]) else "incomplete"
    store_feedback(fb)
    st = load_state()
    for r in ledger_rows(st["runs"]):
        if r.get("code") == code and r.get("slug") == idea["slug"] and r.get("kind", "2d") == kind:
            r.update(status=status, minutes=round((run["finished"] - run["started"]) / 60, 1), feedback_ok=not fb_problems, notes=len(read_notes(run_dir)), agent=ended["status"])
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
    for r in reversed(ledger_rows(load_state()["runs"])):
        kind, slug = r.get("kind", "2d"), row_slug(r)
        if slug and ref in (slug, f"{kind}/{slug}") or (r.get("worktree") and os.path.abspath(r["worktree"]) == os.path.abspath(ref)):
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
    for r in ledger_rows(st["runs"]):
        if same_run(r, row):
            r.update(archived=dest, worktree_removed=True)
    save_state(st)


def ship_problems(r, feedback_only=False, hints=True):
    """Every reason `ship` would refuse this run right now, as sentences ([] when it would ship). Nothing is changed: `do_ship` refuses on exactly these, and the digest
    shows them for a run that was left behind. A game that cannot be published is one of them (its wiki page is missing or fails `wiki --check`)."""
    wt, slug, kind = r.get("worktree"), r["slug"], r.get("kind", "2d")
    if not wt or not os.path.isdir(wt):
        return [f"its worktree is gone ({wt or 'none recorded'}), so there is nothing to ship"]
    if not os.path.isfile(os.path.join(wt, RUN_DIRNAME, "run.json")):
        return [f"{wt} has no {RUN_DIRNAME}/run.json: it is not an Idea Forge run"]
    out = []
    stray = [p for p in changed_paths(wt) if not allowed_path(p, slug, kind) and not p.startswith(RUN_DIRNAME + "/")]
    if stray:
        out.append("the run changed more than the game and its feedback (a missing engine feature is a finding, not a patch):\n  " + "\n  ".join(stray[:12] + ([f"... and {len(stray) - 12} more"] if len(stray) > 12 else [])))
    run = load_run(os.path.join(wt, RUN_DIRNAME))
    fb = feedback_path(wt, run)
    bad = check_feedback_text(rd(fb)) if os.path.exists(fb) else ["no feedback file"]
    if bad:
        out.append("the feedback is not ready:\n  " + "\n  ".join(bad))
    if not feedback_only:
        bad = worktree_wiki_problems(wt, slug, hints)
        if bad:
            out.append("no complete wiki page, so the game cannot be published and is not shipped (the run is kept):\n  " + "\n  ".join(bad))
    return out


def do_ship(r, no_push=False, keep=False, feedback_only=False, hold=False):
    """Commit ONLY the game, its wiki page and its feedback (only the feedback when the game did not build), push, open the PR; returns the PR url ('' with no_push).
    Raises ShipRefused with the reason when not shippable: a refused ship changes nothing (no commit, no push, the run's worktree stays), because the worktree is archived and
    removed only at the very end, after the PR exists."""
    wt, slug, kind = r["worktree"], r["slug"], r.get("kind", "2d")
    problems = ship_problems(r, feedback_only)
    if problems:
        raise ShipRefused("\n".join(problems))
    run = load_run(os.path.join(wt, RUN_DIRNAME))
    fb = feedback_path(wt, run)
    store_feedback(fb)
    own = [game_dir(kind, slug)] if kind == "3d" else [game_file(kind, slug)] + ([game_dir(kind, slug)] if os.path.isdir(os.path.join(wt, game_dir(kind, slug))) else [])
    sent = bool(r.get("feedback_pr")) and not feedback_only and feedback_already_on_main(wt, fb)   # an earlier feedback-only PR of this run is merged: this PR carries the game and its page only
    paths = ([] if sent else [os.path.relpath(fb, wt)]) + ([] if feedback_only else own + [wiki_file(slug)])
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
            f"- Idea: YOU {run['idea']['you']}, BUT {run['idea']['but']}.\n" + ("" if feedback_only else f"- Game: `{game_file(kind, slug)}`\n- Wiki page: `{wiki_file(slug)}` (checked at ship; `publish` draws its pictures)\n") +
            (f"- Feedback: `{os.path.relpath(fb, wt)}`" + (f" (already on main: sent earlier as {r['feedback_pr']})" if sent else "") + "\n\n") +
            "No engine code changed. The nightly run merges this once CI passes and the game's page is in the PR (`idea_forge.py settle`; `config --auto-merge off` or `ship --hold` keeps it for a human); the backlog (`idea_forge.py backlog`) ranks its findings with every earlier run.\n\n"
            "🤖 Generated with [Claude Code](https://claude.com/claude-code)")
    p = subprocess.run(["gh", "pr", "create", "--base", "main", "--head", r["branch"], "--title", title, "--body", body], cwd=wt, capture_output=True, text=True)
    pr = p.stdout.strip().splitlines()[-1] if p.returncode == 0 and p.stdout.strip() else ""
    print(p.stdout.strip() or p.stderr.strip())
    st = load_state()
    for x in ledger_rows(st["runs"]):
        if x.get("slug") == slug and x.get("kind", "2d") == kind:
            x["status"] = "shipped" if pr else x.get("status", "built")
            x["pr"] = pr
            x["feedback_only"] = feedback_only
            if hold:
                x["hold"] = True
    save_state(st)
    if pr and r.get("owned") and not keep:
        archive_run(r)
    return pr


FEEDBACK_BRANCH_SUFFIX = "-feedback"   # the branch a kept run's feedback-only PR is pushed on: <run branch>-feedback


def feedback_already_on_main(wt, fb):
    """Is the run's feedback file `fb` on `origin/main` byte for byte (after a fetch)? False when that cannot be told: the caller then simply includes the file, which git merges without
    a conflict when main has the same bytes."""
    rel = os.path.relpath(fb, wt).replace(os.sep, "/")
    subprocess.run(["git", "-C", wt, "fetch", "-q", "origin", "main"], capture_output=True)
    there = subprocess.run(["git", "-C", wt, "rev-parse", "--verify", "-q", f"origin/main:{rel}"], capture_output=True, text=True)
    mine = subprocess.run(["git", "-C", wt, "hash-object", "--", fb], capture_output=True, text=True)
    return there.returncode == 0 and mine.returncode == 0 and there.stdout.strip() == mine.stdout.strip()


def send_kept_feedback(r):
    """A built game that `ship` refuses for its wiki page ALONE is kept for a maintainer (merging it would put a game on main that `publish` refuses), but its engine feedback must not wait
    with it: ADR 2026-10-08 says `produce` always delivers feedback. So the feedback file goes out ALONE, as a feedback-only PR on its own branch (`<branch>-feedback`, built with git plumbing
    from `origin/main` and a temporary index, so the run's branch, index and worktree are exactly as they were), tracked by `settle` as `feedback_pr` on the run. The run is NOT marked
    shipped, keeps its worktree and its `built` status, so a later `ship` still ships the game with its page (and leaves the feedback out when this PR is already merged: see `do_ship`).
    Returns (the PR url, ""); ("", "") when this does not apply (the run is refused for something besides its page: the feedback then stays local until the run is fixed and shipped);
    ("", why) when it applied and failed. Never raises: the night goes on."""
    try:
        if r.get("feedback_pr"):
            return r["feedback_pr"], ""
        if not ship_problems(r, hints=False) or ship_problems(r, feedback_only=True, hints=False):
            return "", ""
        wt, kind = r["worktree"], r.get("kind", "2d")
        run = load_run(os.path.join(wt, RUN_DIRNAME))
        fb = feedback_path(wt, run)
        rel = os.path.relpath(fb, wt).replace(os.sep, "/")
        branch = r["branch"] + FEEDBACK_BRANCH_SUFFIX
        subprocess.run(["git", "-C", wt, "fetch", "-q", "origin", "main"], capture_output=True)
        base = git(wt, "rev-parse", "--verify", "origin/main")
        title = f"Idea Forge {kind.upper()} run: {run['idea']['title']} (feedback only: the game's wiki page is not written yet)"
        msg = f"{title}\n\nGenerated by scripts/idea_forge.py from idea code {run['idea']['code']:010d}; the feedback file is {FEEDBACK_DIR}/.\n\nCo-Authored-By: Claude <noreply@anthropic.com>"
        with tempfile.TemporaryDirectory() as tmp:
            env = dict(os.environ, GIT_INDEX_FILE=os.path.join(tmp, "index"))

            def plumb(*args):
                p = subprocess.run(["git", "-C", wt, *args], capture_output=True, text=True, env=env)
                if p.returncode:
                    raise RuntimeError(f"git {args[0]} failed: " + (p.stderr or p.stdout).strip())
                return p.stdout.strip()
            plumb("read-tree", base)
            plumb("update-index", "--add", "--cacheinfo", f"100644,{plumb('hash-object', '-w', '--', fb)},{rel}")
            tree = plumb("write-tree")
            if tree == plumb("rev-parse", f"{base}^{{tree}}"):
                return "", ""   # already on main (an earlier PR of this run was merged): nothing to send
            commit = plumb("commit-tree", tree, "-p", base, "-m", msg)
        git(wt, "push", "-q", "origin", f"{commit}:refs/heads/{branch}")
        body = (f"## What\nThe engine feedback of a run whose game was built but is NOT shipped, forged by `scripts/idea_forge.py` (code `{run['idea']['code']:010d}`).\n\n"
                f"- Idea: YOU {run['idea']['you']}, BUT {run['idea']['but']}.\n- Feedback: `{rel}`\n\n"
                f"The game `{r['slug']}` is kept in a worktree for a maintainer: its wiki page `{wiki_file(r['slug'])}` is missing or fails `wiki --check`, and a game merged without its page can never be "
                "published. It ships later as a PR of its own (`idea_forge.py ship`), once its page is written. This PR carries no game, so the findings reach the repository tonight.\n\n"
                "No engine code changed.\n\n🤖 Generated with [Claude Code](https://claude.com/claude-code)")
        p = subprocess.run(["gh", "pr", "create", "--base", "main", "--head", branch, "--title", title, "--body", body], cwd=wt, capture_output=True, text=True)
        pr = p.stdout.strip().splitlines()[-1] if p.returncode == 0 and p.stdout.strip() else ""
        if not pr:
            raise RuntimeError(f"`gh pr create` made no PR (the branch {branch} is pushed: open its PR by hand): " + " ".join((p.stderr or p.stdout).split())[:200])
        st = load_state()
        for x in ledger_rows(st["runs"]):
            if same_run(x, r):
                x["feedback_pr"], x["feedback_branch"] = pr, branch
        save_state(st)
        r["feedback_pr"], r["feedback_branch"] = pr, branch
        return pr, ""
    except (Exception, SystemExit) as e:
        return "", "the feedback-only PR could not be made: " + " ".join(str(e).split())[:240]


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
            if pid and pid != os.getpid() and pid_alive(pid):   # our own pid: this process re-executed itself (`restart_if_refreshed`) and still holds the lock
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
        why = " ".join(str(e).split())[:320]
        sent, failed = send_kept_feedback(row) if built else ("", "")   # refused for its page alone: the findings still reach the repo, the game waits
        if sent:
            return "review", f"not shipped (worktree kept; its feedback went out as a feedback-only PR: {sent}; the game ships once its page is written): " + why
        return "review", "not shipped (worktree kept; the feedback is saved locally" + (f"; {failed}" if failed else "") + "): " + why
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


GH_FILES_CAP = 100       # `gh pr view --json files` lists at most this many files, whatever the PR changes (checked with gh 2.45: a 167-file PR gives 100)
API_FILES_LIMIT = 3000   # and GitHub's own files API stops listing at this many


def pr_files_all(url):
    """Every path a PR changes, read page by page from GitHub's API (`gh api --paginate`); None when it cannot be read (not a GitHub PR url, `gh` failed, or the PR is bigger than the API lists)."""
    m = re.match(r"https?://[^/]+/([^/]+)/([^/]+)/pull/(\d+)", url)
    if not m:
        return None
    p = subprocess.run(["gh", "api", "--paginate", f"repos/{m.group(1)}/{m.group(2)}/pulls/{m.group(3)}/files", "--jq", ".[].filename"], capture_output=True, text=True)
    paths = [l for l in p.stdout.splitlines() if l]
    return paths if p.returncode == 0 and paths and len(paths) < API_FILES_LIMIT else None


def pr_status(url):
    """GitHub's view of a PR as a dict (state, mergeable, checks, files), or None when `gh` cannot say. A list of exactly GH_FILES_CAP files may be cut off, so the whole list is fetched; when that
    fails `files_incomplete` is set and `settle` merges nothing it could not see all of."""
    p = subprocess.run(["gh", "pr", "view", url, "--json", "state,mergeable,statusCheckRollup,files"], capture_output=True, text=True)
    if p.returncode:
        return None
    try:
        s_ = json.loads(p.stdout)
    except ValueError:
        return None
    if isinstance(s_, dict) and len(s_.get("files") or []) >= GH_FILES_CAP:
        full = pr_files_all(url)
        if full is not None:
            s_["files"] = [{"path": x} for x in full]
        else:
            s_["files_incomplete"] = True
    return s_


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
    for r in ledger_rows(st["runs"]):   # a row that is not a row, or has no slug, is skipped: one bad entry must not stop the night's settle
        url, slug = r.get("pr") or "", row_slug(r)
        if slug and isinstance(url, str) and url.startswith("http") and r.get("status") == "shipped" and r.get("merge") not in ("merged", "closed") and not r.get("hold") and url not in seen:
            seen.add(url)
            out.append({"url": url, "type": "game", "slug": slug, "kind": r.get("kind", "2d"), "feedback_only": bool(r.get("feedback_only"))})
        fbu = r.get("feedback_pr") or ""   # the feedback-only PR of a run that is kept (built, not shipped): merged like any feedback-only PR, recorded as `feedback_merge`
        if slug and isinstance(fbu, str) and fbu.startswith("http") and r.get("feedback_merge") not in ("merged", "closed") and fbu not in seen:
            seen.add(fbu)
            out.append({"url": fbu, "type": "game", "slug": slug, "kind": r.get("kind", "2d"), "feedback_only": True})
    for key, imp in st.get("improvements", {}).items():
        url = imp.get("pr", "")
        if url.startswith("http") and imp.get("status") == "open" and url not in seen:
            seen.add(url)
            out.append({"url": url, "type": "fix"})
    return out


def record_pr(url, **fields):
    st = load_state()
    for r in ledger_rows(st["runs"]):
        if r.get("pr") == url:
            r.update(fields)
        if r.get("feedback_pr") == url and "merge" in fields:
            r["feedback_merge"] = fields["merge"]   # not `merge`: that is the verdict on the run's game PR, which does not exist yet
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


def pr_missing_wiki(pr, files):
    """Why to hold (not merge) a PR that adds a game without the game's wiki page in the same PR; "" when it is fine. Merging such a game is how four games sat in RedEngine
    unpublished from 2026-10-19: `publish` refuses a game with no page on main. A feedback-only PR carries no game and is never held for this."""
    if pr["type"] != "game" or pr["feedback_only"]:
        return ""
    paths = {f.get("path", "") for f in files}
    slug, kind = pr["slug"], pr["kind"]
    if not any(p == game_prefix(kind, slug) or p.startswith(game_dir(kind, slug)) for p in paths) or wiki_file(slug) in paths:
        return ""
    return f"the PR adds the game {slug} without its wiki page {wiki_file(slug)}, so it could not be published: add the page to the PR's branch (idea_forge.py wiki --check)"


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
    if s_.get("files_incomplete"):
        record_pr(url, merge="refused-paths")
        return "refused", f"the PR changes {GH_FILES_CAP} or more files and GitHub's whole list could not be read, so its paths cannot all be checked: left for a human"
    if not pr_paths_ok(pr, s_.get("files") or []):
        record_pr(url, merge="refused-paths")
        return "refused", "the PR changes files outside what its kind may change: left for a human"
    why = pr_missing_wiki(pr, s_.get("files") or [])
    if why:
        record_pr(url, merge="held-no-wiki")   # still tracked: once the page is on the PR's branch the next settle merges it
        return "held", why
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
    """Merge every tracked PR whose checks have passed; keep asking until none is pending or `wait_min` minutes have passed (0 = one look).
    Each PR is reported once, when its verdict is final; only the pending ones are asked about again."""
    deadline = time.time() + wait_min * 60
    final = {}        # url -> (pr, verdict, detail) for every PR whose verdict will not change this call
    pending = {}
    while True:
        pending = {}
        for pr in tracked_prs():
            if pr["url"] in final:
                continue
            verdict, detail = settle_one(pr)
            if verdict == "pending":
                pending[pr["url"]] = (pr, verdict, detail)
            else:
                final[pr["url"]] = (pr, verdict, detail)
                print(f"settle: {verdict}: {pr['url']} ({detail})")
        if not pending or time.time() >= deadline:
            for url, (pr, verdict, detail) in pending.items():
                print(f"settle: still pending: {url} (it stays tracked: the next run settles it)")
            return list(final.values()) + list(pending.values())
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


# ---- publish: merged games to RedEngineGames --------------------------------------------------------------------------------------------------------

GAMES_REPO = "kevstermcgee/RedEngineGames"
GAMES_MANIFEST = ".release-games.json"
PUBLISH_PUSH_TRIES = 3   # RedEngine's own publish workflow commits to RedEngineGames main all day: a rejected push is fetched, rebased and retried


def publish_enabled():
    """Publishing merged games to RedEngineGames is the default (that is where a player finds them); `config --publish off` or IDEA_FORGE_PUBLISH=0 turns it off."""
    if os.environ.get("IDEA_FORGE_PUBLISH", "").lower() in ("0", "off", "false", "no"):
        return False
    return bool(load_state().get("config", {}).get("publish", True))


def games_remote():
    return os.environ.get("IDEA_FORGE_GAMES_REMOTE") or f"https://github.com/{GAMES_REPO}.git"


def merged_game_files(root, kind, slug, ref="origin/main"):
    """(path on `ref`, path inside projects/<slug>/) for every file of a game that is on `ref`; empty when the game is not there (not merged yet, or its run failed); None when git could not
    be asked at all (no such ref, not a repository): "not there" and "could not look" are different answers, and only the first may clear a game's `publish_blocked`."""
    game = game_file(kind, slug)
    prefix = game_prefix(kind, slug)
    p = subprocess.run(["git", "-C", root, "ls-tree", "-r", "--name-only", ref, "--", prefix.rstrip("/")], capture_output=True, text=True)
    if p.returncode:
        return None
    paths = [l for l in p.stdout.splitlines() if l]
    if game not in paths:
        return []
    base = os.path.dirname(game)
    return [(x, x[len(base) + 1:]) for x in paths]


def publishable(root, ref="origin/main", unknown=None):
    """The ledger rows whose game is on main and not yet in RedEngineGames, oldest first, each with its files. A game whose run did not build (feedback only) never is. A row whose game git
    could not be asked about is appended to `unknown` (a list, when one is given): it is neither publishable nor known to be gone."""
    out = []
    for r in ledger_rows(load_state()["runs"]):
        if not row_slug(r) or r.get("status") != "shipped" or r.get("feedback_only") or r.get("published"):
            continue
        files = merged_game_files(root, r.get("kind", "2d"), r["slug"], ref)
        if files is None:
            if unknown is not None:
                unknown.append(r)
        elif files:
            out.append((r, files))
    return out


def playable_entry(r):
    """The `.release-games.json` entry that makes RedEngineGames build, release and list the game (a 2D game is played by `re2d`: `kind: "2d"`)."""
    slug, kind = r["slug"], r.get("kind", "2d")
    entry = {"slug": slug, "name": r.get("title") or slug.replace("-", " ").title()}
    if kind == "2d":
        entry["kind"] = "2d"
    entry["files"] = [f"projects/{slug}"]
    entry["arguments"] = [f"content/projects/{slug}/{os.path.basename(game_file(kind, slug))}"]
    return entry


def add_playables(manifest_text, entries):
    """The manifest with `entries` appended to `data_playables`, edited as text so the diff is only the new entries (the file is hand-formatted)."""
    tail = '\n  ],\n  "native_playables"'
    if tail not in manifest_text:
        raise ValueError(f"{GAMES_MANIFEST} has an unexpected shape: no `data_playables` list followed by `native_playables`")
    blocks = []
    for e in entries:
        lines = [f'      "{k}": {json.dumps(v)}' for k, v in e.items()]
        blocks.append("    {\n" + ",\n".join(lines) + "\n    }")
    return manifest_text.replace(tail, ",\n" + ",\n".join(blocks) + tail, 1)


def games_checkout():
    """A scratch clone of RedEngineGames at its main, reset every time (this tool's own copy; the owner's clone is never touched)."""
    d = os.path.join(state_dir(), "games-repo")
    remote = games_remote()
    if not os.path.isdir(os.path.join(d, ".git")):
        shutil.rmtree(d, ignore_errors=True)
        git(os.path.dirname(d), "clone", "-q", remote, d)
    else:
        git(d, "remote", "set-url", "origin", remote)
        git(d, "fetch", "-q", "origin", "main")
        git(d, "checkout", "-q", "-B", "main", "origin/main")
        git(d, "clean", "-fdq")
    return d


THUMB_SIZE = "640x360"   # what site/gen_thumbs.py makes and the site's cards and the installer icon expect
PNG_MAGIC = b"\x89PNG\r\n\x1a\n"


def thumb_path(clone, slug):
    return os.path.join(clone, "site", "thumbs", f"{slug}.png")


def render_thumbnail(root, scene, out, kind="3d"):
    """A screenshot of the game as the engine draws it (`frame`: the scene's own camera, a 2D game a second into its first scenario). True only when a real PNG with something in it
    was written: a missing or blank picture is a failure, because every game on the site has to have one."""
    os.makedirs(os.path.dirname(out), exist_ok=True)
    if os.path.exists(out):
        os.remove(out)
    args = ["frame", scene, out, "--size", THUMB_SIZE] + (["--t", "1"] if kind == "2d" else [])
    try:
        run_engine(root, args, timeout=300)
    except subprocess.TimeoutExpired:
        pass
    if os.path.isfile(out):
        with open(out, "rb") as f:
            head = f.read(8)
        if head == PNG_MAGIC and os.path.getsize(out) > 2000:   # a flat single-colour 640x360 PNG compresses to well under 2 KB
            return True
        os.remove(out)   # whatever a failed draw left must never be committed as the game's picture
    return False


def playable_scene(clone, playable):
    """The file `frame` draws for a playable: its first `.json` argument (`content/<path>` is `<path>` in the repository), or None."""
    for arg in playable.get("arguments", []):
        if arg.endswith(".json"):
            path = os.path.join(clone, *arg.removeprefix("content/").split("/"))
            return path if os.path.isfile(path) else None
    return None


def fill_thumbnails(root, clone):
    """A screenshot for every listed game that has none (the catalog's own games and anything an earlier run could not draw). Returns (made slugs, failed slugs)."""
    plays = json.loads(rd(os.path.join(clone, ".games-catalog.json"))).get("playables", []) + json.loads(rd(os.path.join(clone, GAMES_MANIFEST))).get("data_playables", [])
    made, failed = [], []
    for p in plays:
        if os.path.exists(thumb_path(clone, p["slug"])):
            continue
        scene = playable_scene(clone, p)
        if scene and render_thumbnail(root, scene, thumb_path(clone, p["slug"]), p.get("kind", "3d")):
            made.append(p["slug"])
        else:
            failed.append(p["slug"])
    return made, failed


def stage_games(root, clone, items, ref="origin/main"):
    """Copy each game into projects/<slug>/, draw its screenshot and list it in the manifest. Returns (staged rows, skipped: [(row, why)], failed: [(row, why)]). A slug already published
    by hand is never overwritten (skipped: recorded, not asked about again); a game whose screenshot cannot be drawn is not published (failed: tried again next run)."""
    manifest_path = os.path.join(clone, GAMES_MANIFEST)
    text = rd(manifest_path)
    have = {p["slug"] for p in json.loads(text).get("data_playables", [])}
    staged, skipped, failed = [], [], []
    for r, files in items:
        slug = r["slug"]
        if slug in have or os.path.exists(os.path.join(clone, "projects", slug)):
            skipped.append((r, f"projects/{slug} already exists in RedEngineGames"))
            continue
        why = merged_wiki(root, slug, ref)[1]   # the cheap check first: a game without its page must not cost a copy and a screenshot (up to 300 s) every night, twice a night
        if why:
            failed.append((r, why))
            continue
        for src, rel in files:
            blob = subprocess.run(["git", "-C", root, "show", f"{ref}:{src}"], capture_output=True)
            if blob.returncode:
                raise RuntimeError(f"git show {ref}:{src} failed: {blob.stderr.decode(errors='replace').strip()}")
            dest = os.path.join(clone, "projects", slug, *rel.split("/"))
            os.makedirs(os.path.dirname(dest), exist_ok=True)
            with open(dest, "wb") as f:
                f.write(blob.stdout)
        entry = playable_entry(r)
        if not render_thumbnail(root, playable_scene(clone, entry), thumb_path(clone, slug), r.get("kind", "2d")):
            shutil.rmtree(os.path.join(clone, "projects", slug), ignore_errors=True)
            failed.append((r, "the engine could not draw a screenshot of it"))
            continue
        why = build_wiki(root, clone, r, entry, ref)
        if why:
            shutil.rmtree(os.path.join(clone, "projects", slug), ignore_errors=True)
            shutil.rmtree(os.path.join(clone, "site", "wiki", slug), ignore_errors=True)
            failed.append((r, why))
            continue
        staged.append(r)
    if staged:
        wr(manifest_path, add_playables(text, [playable_entry(r) for r in staged]))
        mark_authors(clone, [r["slug"] for r in staged], "ai")
    return staged, skipped, failed


GAMES_META = "site/games-meta.json"


def merged_wiki(root, slug, ref="origin/main"):
    """(the page's data, "") when the game's wiki page is on `ref` and passes `wiki --check`'s rules; (None, why) when it is missing or not sound: the first thing `publish` needs
    and the cheap one (no engine, no pictures), so `publish --dry-run` can say which games it would refuse."""
    blob = subprocess.run(["git", "-C", root, "show", f"{ref}:{wiki_file(slug)}"], capture_output=True)
    if blob.returncode:
        return None, f"no wiki page ({wiki_file(slug)} is not on main)"
    try:
        data = json.loads(blob.stdout)
    except ValueError as e:
        return None, f"its wiki page is not valid JSON: {e}"
    bad = wiki_check(data)
    if bad:
        return None, "its wiki page is incomplete: " + "; ".join(bad[:3])
    return data, ""


def build_wiki(root, clone, r, entry, ref="origin/main"):
    """The game's wiki page for RedEngineGames: its agent's data from `ref`, every `shot` drawn by the engine into `site/wiki/<slug>/`, then RedEngineGames' own `site/wiki.py` as the judge.
    Returns "" when the page is in the clone, else why the game cannot be published yet (a game without a complete page is not listed: the site refuses it)."""
    slug, kind = r["slug"], r.get("kind", "2d")
    data, why = merged_wiki(root, slug, ref)
    if why:
        return why
    scene, folder = playable_scene(clone, entry), os.path.join(clone, "site", "wiki", slug)
    shutil.rmtree(folder, ignore_errors=True)
    seen = set()
    for key, prefix, size in (("characters", "c", CHARACTER_SIZE), ("gallery", "g", GALLERY_SIZE)):
        for i, x in enumerate(data[key]):
            name = f"{prefix}{i + 1}.png"
            if not draw_shot(root, scene, os.path.join(folder, name), x.pop("shot"), kind, size):
                return f"the engine could not draw wiki picture {name} ({x.get('caption') or x.get('name')})"
            x["image"] = name
            if key == "gallery":
                digest = hashlib.sha256(open(os.path.join(folder, name), "rb").read()).hexdigest()
                if digest in seen:
                    return f"wiki picture {name} is identical to an earlier gallery picture: the shots must show different moments"
                seen.add(digest)
    wr(os.path.join(folder, "wiki.json"), json.dumps(data, indent=1, ensure_ascii=False) + "\n")
    if os.path.exists(os.path.join(clone, "site", "wiki.py")):
        check = "import sys, json, pathlib; sys.path.insert(0, 'site'); import wiki; f = pathlib.Path(sys.argv[1]); p = wiki.problems(json.loads((f / 'wiki.json').read_text()), 'ai', f); print('\\n'.join(p)); sys.exit(1 if p else 0)"
        p = subprocess.run([sys.executable, "-c", check, folder], cwd=clone, capture_output=True, text=True)
        if p.returncode:
            return "RedEngineGames would reject its wiki page: " + (p.stdout + p.stderr).strip()[-300:]
    return ""


def mark_authors(clone, slugs, author):
    """Record who designed each game in RedEngineGames' `site/games-meta.json` (`"author": "ai"`), which the site reads to badge a game and to filter AI-designed from human-designed ones.
    Every game this tool publishes is AI-designed; a slug without an author is a human's. The file is rewritten with its other entries untouched."""
    path = os.path.join(clone, GAMES_META)
    meta = jload(path) if os.path.exists(path) else {}
    for slug in slugs:
        meta.setdefault(slug, {})["author"] = author
    wr(path, json.dumps(meta, indent=2, ensure_ascii=False) + "\n")


def check_release_manifest(clone):
    """RedEngineGames' own loader must accept the result (safe slugs, no duplicates, every published path present): what its release workflow runs first."""
    code = "import sys; sys.path.insert(0, 'distribution'); import release_tool as rt; print(len(rt.load_playables()))"
    p = subprocess.run([sys.executable, "-I", "-B", "-c", code], cwd=clone, capture_output=True, text=True, env=dict(os.environ, PYTHONPATH=""))
    if p.returncode:
        raise RuntimeError("RedEngineGames would reject the manifest: " + (p.stderr or p.stdout).strip()[-300:])


def push_games(clone, message):
    """Commit and push to RedEngineGames main; returns the commit id. A rejected push (the publish workflow committed meanwhile) is rebased and retried."""
    git(clone, "add", "--", "projects", "site/thumbs", GAMES_MANIFEST, *[x for x in (GAMES_META, "site/wiki") if os.path.exists(os.path.join(clone, x))])   # only what was staged: RedEngineGames tracks files (a .pyc) that running its own tools rewrites
    p = subprocess.run(["git", "-C", clone, "commit", "-q", "-m", message], capture_output=True, text=True)
    if p.returncode:
        raise RuntimeError("git commit failed (is user.name / user.email set?): " + (p.stderr or p.stdout).strip())
    last = ""
    for _ in range(PUBLISH_PUSH_TRIES):
        p = subprocess.run(["git", "-C", clone, "push", "-q", "origin", "HEAD:main"], capture_output=True, text=True)
        if p.returncode == 0:
            return git(clone, "rev-parse", "--short=12", "HEAD")
        last = (p.stderr or p.stdout).strip()
        git(clone, "fetch", "-q", "origin", "main")
        if subprocess.run(["git", "-C", clone, "rebase", "-q", "origin/main"], capture_output=True, text=True).returncode:
            subprocess.run(["git", "-C", clone, "rebase", "--abort"], capture_output=True)
            raise RuntimeError("the push was rejected and the rebase conflicted: " + last[-200:])
    raise RuntimeError(f"the push was rejected {PUBLISH_PUSH_TRIES} times: " + last[-200:])


def same_run(x, r):
    """Are two ledger rows the same run? (a run is its idea code, slug and kind)"""
    return isinstance(x, dict) and x.get("code") == r.get("code") and x.get("slug") == r.get("slug") and x.get("kind", "2d") == r.get("kind", "2d")


def iso_date(value, default):
    """`value` when it is a YYYY-MM-DD date string, else `default`."""
    try:
        return datetime.date.fromisoformat(value).isoformat()
    except (TypeError, ValueError):
        return default


def record_publish_blocks(blocked, today=None, unknown=()):
    """Write into the ledger why each merged game could not be published: `publish_blocked: {reason, since, last}` on its run. `since` is when the game started waiting: kept while it stays
    blocked, else the run's own date (it was shipped that day and merged soon after: the first night this code looked is not when the wait began). `blocked` is [(row, why)] of this run of
    `publish`, which looked at every merged unpublished game, so any other run loses its entry (it published, was listed by hand, or is no longer on main) EXCEPT the rows in `unknown`,
    the ones git could not be asked about: a failed lookup says nothing, so their entries stay. `unpublished_games` and the feedback inbox read this: a game that cannot publish must never
    wait in silence (2026-10-19: four merged games did)."""
    today = today or datetime.date.today().isoformat()
    st = load_state()
    for x in ledger_rows(st["runs"]):
        hit = [why for r, why in blocked if same_run(x, r)]
        if hit:
            x["publish_blocked"] = {"reason": hit[0], "since": (x.get("publish_blocked") or {}).get("since") or iso_date(x.get("date"), today), "last": today}
        elif not any(same_run(x, r) for r in unknown):
            x.pop("publish_blocked", None)
    save_state(st)


STRANDED_DAYS = 2   # a run that built a game and was never shipped counts as stranded once it is older than this


def ledger_rows(runs):
    """The ledger rows that are dicts. A hand-edited or damaged ledger (`[]`, `"x"` or `null` among the runs, or `runs` not a list at all) must not stop a reader: such a row is skipped."""
    return [r for r in runs if isinstance(r, dict)] if isinstance(runs, list) else []


def row_slug(r):
    """A row's slug when it has a usable one, else "" (a row without one cannot be named, so the readers skip it)."""
    slug = r.get("slug")
    return slug if isinstance(slug, str) else ""


def unpublished_games(runs=None):
    """The merged games `publish` could not publish, from the ledger (None runs: this machine's ledger; nothing when it has none): [{slug, kind, pr, reason, since, last}], oldest first."""
    runs = ledger_runs() if runs is None else runs
    out = []
    for r in ledger_rows(runs):
        b, slug = r.get("publish_blocked"), row_slug(r)
        if slug and isinstance(b, dict) and r.get("status") == "shipped" and not r.get("feedback_only") and not r.get("published"):
            out.append({"slug": slug, "kind": str(r.get("kind", "2d")), "pr": str(r.get("pr", "")), "reason": str(b.get("reason", "")), "since": str(b.get("since", "")), "last": str(b.get("last", ""))})
    return sorted(out, key=lambda g: (g["since"], g["slug"]))


def held_prs(runs=None):
    """The game PRs `settle` holds because the PR has no wiki page for its game, from the ledger: [{slug, kind, pr, date}], oldest first. (A PR held on purpose, by `ship --hold` or the off switch,
    is not listed: someone chose that.)"""
    runs = ledger_runs() if runs is None else runs
    out = []
    for r in ledger_rows(runs):
        slug = row_slug(r)
        if slug and r.get("status") == "shipped" and r.get("merge") == "held-no-wiki":
            out.append({"slug": slug, "kind": str(r.get("kind", "2d")), "pr": str(r.get("pr", "")), "date": str(r.get("date", ""))})
    return sorted(out, key=lambda g: (g["date"], g["slug"]))


def stranded_runs(runs=None, today=None):
    """The runs that built a game and were never shipped (status `built`) for more than STRANDED_DAYS, each with what `ship` would refuse today: [{slug, kind, date, days, reason}], oldest first.
    Read only: the worktree is looked at, never changed."""
    runs = ledger_runs() if runs is None else runs
    today = datetime.date.fromisoformat(today) if isinstance(today, str) else (today or datetime.date.today())
    out = []
    for r in ledger_rows(runs):
        slug = row_slug(r)
        if not slug or r.get("status") != "built":
            continue
        try:
            days = (today - datetime.date.fromisoformat(r["date"])).days
        except (KeyError, ValueError, TypeError):
            continue
        if days <= STRANDED_DAYS:
            continue
        try:
            reasons = ship_problems(r, hints=False)
        except Exception as e:   # a half-removed worktree (or a row missing a field) must not stop the report
            reasons = [f"could not be checked: {e}"]
        why = "; ".join(" ".join(x.split()) for x in reasons) or "nothing blocks it now: it is ready to ship"
        if r.get("feedback_pr"):
            why += f" (its feedback is already on its own PR: {r['feedback_pr']})"
        out.append({"slug": slug, "kind": str(r.get("kind", "2d")), "date": r["date"], "days": days, "worktree": str(r.get("worktree") or ""), "reason": why})
    return sorted(out, key=lambda g: (g["date"], g["slug"]))


def stuck_lines(runs=None, today=None):
    """What the Idea Forge made that no player can have yet, as log lines (the night's last lines print them; [] when nothing is stuck)."""
    out = [f"stuck: {g['kind']} {g['slug']} is merged ({g['pr']}) but NOT published since {g['since']}: {g['reason']}" for g in unpublished_games(runs)]
    out += [f"stuck: {g['kind']} {g['slug']} built {g['days']} days ago ({g['date']}) and was never shipped (worktree {g['worktree'] or 'none'}); ship would refuse: {g['reason']}" for g in stranded_runs(runs, today)]
    return out


def publish_games(root=None, dry_run=False):
    """Put every merged, unpublished game into RedEngineGames: its files under projects/<slug>/ and an entry in `.release-games.json`, which is all RedEngineGames needs to build a
    Windows installer, cut a release and list the game on its site. One commit to main for all of them. Returns the published slugs. Never raises: the night goes on."""
    root = root or ROOT
    try:
        subprocess.run(["git", "-C", root, "fetch", "-q", "origin", "main"], capture_output=True)
        unknown = []
        items = publishable(root, unknown=unknown)
        if dry_run:
            clone, have = None, set()
            if items:
                try:   # a real run skips a game RedEngineGames already lists (and records it as published), whatever its page says: look, as a real run would (this tool's own scratch clone is refreshed)
                    clone = games_checkout()
                    have = {p["slug"] for p in json.loads(rd(os.path.join(clone, GAMES_MANIFEST))).get("data_playables", [])}
                except (Exception, SystemExit):
                    clone, have = None, set()
            for r, files in items:
                kind, slug = r.get("kind", "2d"), r["slug"]
                why = merged_wiki(root, slug)[1]   # the cheap check; a real run also draws every picture
                if clone and (slug in have or os.path.exists(os.path.join(clone, "projects", slug))):
                    print(f"publish: would skip {kind} {slug}: projects/{slug} already exists in RedEngineGames (a real run records it as published)")
                elif why:
                    print(f"publish: would NOT publish {kind} {slug}: {why}" + ("" if clone else " (unless RedEngineGames already lists it: a real run skips such a game; it could not be asked)"))
                else:
                    print(f"publish: would publish {kind} {slug} ({len(files)} file(s)) and draw its screenshot")
            print("publish: (a real run also draws the screenshot of any listed game that has none)" if items else "publish: no new game (a real run still draws any missing screenshot)")
            return []
        clone = games_checkout()
        staged, skipped, failed = stage_games(root, clone, items)
        for r, why in failed:
            print(f"publish: {r['slug']}: not published, {why}")
        st = load_state()
        for r, why in skipped:   # published by hand already: record it so it is not asked about again
            for x in ledger_rows(st["runs"]):
                if same_run(x, r):
                    x["published"] = {"note": why}
            print(f"publish: {r['slug']}: {why}")
        save_state(st)
        record_publish_blocks(failed, unknown=unknown)
        if failed:
            print(f"publish: NOT published, merged and waiting: {', '.join(r['slug'] for r, _ in failed)} (the reason is in the ledger as publish_blocked, and in the feedback inbox: `scripts/feedback.py digest`)")
        made, missing = fill_thumbnails(root, clone)   # every listed game has a screenshot: the older ones that lack one get theirs in the same commit
        if missing:
            print(f"publish: no screenshot could be drawn for: {', '.join(missing)}")
        if not staged and not made:
            print("publish: nothing to publish")
            return []
        check_release_manifest(clone)
        names = ", ".join(r.get("title") or r["slug"] for r in staged)
        sha = push_games(clone, (f"Publish {len(staged)} Idea Forge game{'s' if len(staged) != 1 else ''}: {names}" if staged else f"Add screenshots for {len(made)} game{'s' if len(made) != 1 else ''}") +
                                "\n\nForged, built and merged into RedEngine by scripts/idea_forge.py; RedEngineGames builds each one's installer and lists it once its release is made." +
                                (f"\n\nScreenshots drawn by the engine for: {', '.join(made)}." if made else "") + "\n\nCo-Authored-By: Claude <noreply@anthropic.com>")
        st = load_state()
        for r in staged:
            for x in ledger_rows(st["runs"]):
                if same_run(x, r):
                    x["published"] = {"commit": sha, "date": datetime.date.today().isoformat()}
                    x.pop("publish_blocked", None)
        save_state(st)
        print(f"publish: published {', '.join(r['slug'] for r in staged) or 'nothing new'}" + (f", screenshots for {', '.join(made)}" if made else "") + f" to {GAMES_REPO} ({sha})")
        return [r["slug"] for r in staged]
    except (Exception, SystemExit) as e:   # a failed publish (`git()` exits on failure) is retried by the next run; it must not fail the night or lose the feedback
        print(f"publish: FAILED, will retry next run: {e}")
        return []


def cmd_publish(a):
    with Lock():
        if not a.dry_run and not publish_enabled():
            sys.exit("publishing is off (`config --publish on` or unset IDEA_FORGE_PUBLISH)")
        publish_games(dry_run=a.dry_run)


def cmd_config(a):
    st = load_state()
    if a.auto_merge:
        st.setdefault("config", {})["auto_merge"] = a.auto_merge == "on"
        save_state(st)
    if a.publish:
        st.setdefault("config", {})["publish"] = a.publish == "on"
        save_state(st)
    cfg = {"auto_merge": auto_merge_enabled(), "publish": publish_enabled()}
    print(json.dumps(cfg))


def improve_brief(groups, today, cli):
    return f"""# Idea Forge engine-improvement run ({today})

You are an AI engineer on RedEngine, a data-driven game engine. In the last days AI agents built games on it and filed feedback about what slowed them down or was missing. Your job now is to fix ONE of those problems in the
engine so the next agent loses less time. This is the loop that makes the engine better every night: a fix that does not remove the friction is worth nothing, and a fix that is not tested will not be trusted.

{backlog_text(groups[:6])}
## Do this

1. Choose ONE issue: the most valuable one you can fix completely and safely in this session. A fix can be code, a clearer error message that names the fix, a line in `describe`/the reference, a recipe, a template: whatever
   removes the friction (prefer making the problem disappear over documenting it). If an issue needs a design decision a human should make, do not build it: choose another, or if none is safe make no change and say why.
2. Follow this repository's own workflow (`AGENTS.md`): `scripts/dev start "<the fix>"`, `scripts/dev red context <words>` for the files and tests involved, `scripts/dev iterate` while you edit. Keep the change small.
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
        p = proc_supervisor.run_capture(cmd, cwd=wt, timeout=3600)   # a gate that hangs is stopped with everything it started, and raises TimeoutExpired
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
    ended = stream_agent(agent_argv(prompt, a), wt, env, session, a.timeout_min * 60, a.quiet)
    run["agent_status"], run["agent_exit"] = ended["status"], ended["returncode"]
    jdump(run, os.path.join(run_dir, "run.json"))
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


def refresh_checkout(root=None):
    """Bring the checkout this tool runs from up to date with `origin/main` before the night, so the tool and the engine that run are the ones that were merged yesterday.
    Only a dedicated checkout is touched: clean, on a branch whose upstream is origin/main, fast-forward only. Anything else is left alone and said so. Returns a one-line report."""
    root = root or ROOT

    def g(*args):
        p = subprocess.run(["git", "-C", root] + list(args), capture_output=True, text=True)
        return p.returncode, (p.stdout + p.stderr).strip()

    if g("status", "--porcelain")[1]:
        return "not updated: the checkout has uncommitted changes"
    rc, branch = g("rev-parse", "--abbrev-ref", "HEAD")
    if rc or branch == "HEAD":
        return "not updated: detached HEAD"
    rc, upstream = g("rev-parse", "--abbrev-ref", "@{upstream}")
    if rc or upstream != "origin/main":
        return f"not updated: `{branch}` does not track origin/main (a dedicated checkout should: `git branch -u origin/main`)"
    rc, out = g("fetch", "-q", "origin", "main")
    if rc:
        return "not updated: fetch failed: " + out[:120]
    rc, out = g("merge", "--ff-only", "-q", "origin/main")
    return "updated to origin/main" if not rc else "not updated: cannot fast-forward: " + out[:120]


REEXEC_ENV = "IDEA_FORGE_REEXEC"


def checkout_head(root=None):
    p = subprocess.run(["git", "-C", root or ROOT, "rev-parse", "HEAD"], capture_output=True, text=True)
    return p.stdout.strip() if p.returncode == 0 else ""


def restart_if_refreshed(before, argv, root=None):
    """`refresh_checkout` changes this file under the running process, which has already loaded yesterday's copy. When it moved the checkout, start the same command again so
    tonight's briefs and publishing run the code that was just merged. Once only: the restarted process carries REEXEC_ENV and never restarts. Returns when there is nothing to do."""
    after = checkout_head(root)
    if not before or not after or before == after or os.environ.get(REEXEC_ENV):
        return
    print(f"checkout: moved {before[:12]} -> {after[:12]}: restarting so the night runs the new code")
    sys.stdout.flush()
    sys.stderr.flush()
    os.environ[REEXEC_ENV] = after
    os.execv(sys.executable, [sys.executable, os.path.join(HERE, "idea_forge.py")] + list(argv))


def free_gb():
    """GiB free where the night's builds go (the home directory's disk). The one place the tool reads the machine's free space: tests replace this function, so a result never depends on the disk of the machine that runs them."""
    return shutil.disk_usage(os.path.expanduser("~")).free / 2**30


def safe_stuck_lines():
    """`stuck_lines()` for the night's last lines, which can never stop a night: a ledger it cannot make sense of costs one warning line, not the night's record."""
    try:
        return stuck_lines()
    except Exception as e:
        print(f"warning: could not list what is stuck ({type(e).__name__}: {e}); the night goes on")
        return []


def cmd_nightly(a):
    """The whole night, under one lock and strictly one agent at a time: today's games (feedback always delivered), then one engine improvement from the backlog they fed."""
    date = a.date or datetime.date.today().isoformat()
    order = a.order.split(",") if a.order else None
    if order and sorted(order) != sorted(KINDS):
        sys.exit("--order must name each kind once, e.g. 3d,2d")
    if not a.dry_run and not shutil.which(a.claude) and not os.path.isfile(a.claude):
        sys.exit(f"`{a.claude}` not found: install Claude Code, or pass --claude PATH")
    free = free_gb()
    if not a.dry_run and free < a.min_free_gb:
        print(f"{date}: skipped: only {free:.0f} GB free (< {a.min_free_gb} GB): each run needs a seeded build and its own build output")
        st = load_state()
        st.setdefault("nights", {})[date] = {"skipped": f"{free:.0f} GB free", "at": round(time.time())}
        save_state(st)
        sys.exit(0)
    publish_ok = not a.no_publish and publish_enabled()
    with Lock():
        if not a.dry_run:
            if a.no_merge:
                os.environ["IDEA_FORGE_AUTO_MERGE"] = "0"
            settle(0)   # yesterday's PRs whose checks have finished: merged before today's worktrees are cut from main
            before = checkout_head()
            print("checkout: " + refresh_checkout())
            restart_if_refreshed(before, a.argv)
            if publish_ok:
                publish_games()   # what merged since the last run, and anything an earlier publish could not push
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
        result = ("off", "")
        if a.improve:
            ns = argparse.Namespace(**vars(a))
            ns.budget, ns.timeout_min, ns.key = a.improve_budget, a.improve_timeout_min, None
            result = do_improve(ns)
        merged, published, held = [], [], []
        if not a.dry_run:
            settled = settle(a.settle_timeout_min)
            merged = [pr["url"] for pr, verdict, _ in settled if verdict == "merged"]
            held = [(pr["url"], detail) for pr, verdict, detail in settled if verdict == "held"]
            published = publish_games() if publish_ok else []   # tonight's games, merged just now
            print("feedback note for the nightly maintenance job: " + export_feedback_note())
        stuck = [] if a.dry_run else safe_stuck_lines()
        st = load_state()
        review = [] if a.dry_run else [(k, v.get("detail", "")) for k, v in st.get("days", {}).get(date, {}).get("slots", {}).items() if v.get("status") == "review"]
        st.setdefault("nights", {})[date] = {"games_failed": failed, "improve": result[0], "detail": str(result[1])[:300], "merged": merged, "published": published, "at": round(time.time())}
        if held:
            st["nights"][date]["held"] = [url for url, _ in held]
        if stuck:
            st["nights"][date]["stuck"] = len(stuck)
        save_state(st)
        print(f"{date}: nightly done: games failed {failed}; improvement {result[0]}; merged {len(merged)} PR(s)" + (f"; published {len(published)} game(s)" if not a.dry_run else "") +
              (f"; HELD {len(held)} PR(s)" if held else "") + (f"; {len(review)} game(s) NOT shipped" if review else "") + (f"; {len(stuck)} stuck (below)" if stuck else ""))
        for url, why in held:
            print(f"held: {url}: {why}")
        for k, detail in review:
            print(f"stuck: the {k} game of {date} was built but NOT shipped: {detail}")
        for line in stuck:
            print(line)
        sys.exit(0)


def cron_lines(times, budget, model=None, improve_budget=None, command="nightly", improve=False):
    """One cron line per time of day. cron has no PATH worth the name, so the current one is baked in (it must reach claude, gh, git, python3 and cargo)."""
    home = state_dir()
    path = os.environ.get("PATH", "")
    cargo = os.path.join(os.path.expanduser("~"), ".cargo", "bin")
    if os.path.isdir(cargo) and cargo not in path.split(os.pathsep):
        path = path + os.pathsep + cargo
    tail = nightly_args(budget, model, improve_budget, improve) if command == "nightly" else f"daily --next --budget {budget:g}" + (f" --model {shlex.quote(model)}" if model else "")
    cmd = (f"cd {shlex.quote(ROOT)} && PATH={shlex.quote(path)} IDEA_FORGE_HOME={shlex.quote(home)} PYTHONUNBUFFERED=1 {shlex.quote(sys.executable)} "
           f"{shlex.quote(os.path.join(HERE, 'idea_forge.py'))} {tail} >> {shlex.quote(os.path.join(home, 'nightly.log'))} 2>&1")
    lines = []
    for t in times:
        m = re.fullmatch(r"(\d{1,2}):(\d{2})", t.strip())
        if not m or int(m.group(1)) > 23 or int(m.group(2)) > 59:
            sys.exit(f"bad time `{t}`: use HH:MM, 24-hour")
        lines.append(f"{int(m.group(2))} {int(m.group(1))} * * * {cmd}  {CRON_MARK}")
    return lines


SYSTEMD_UNIT = "idea-forge"


def systemd_dir():
    return os.path.join(os.path.expanduser("~"), ".config", "systemd", "user")


def have_systemd_user():
    if not shutil.which("systemctl"):
        return False
    return subprocess.run(["systemctl", "--user", "show-environment"], capture_output=True, text=True).returncode == 0


def nightly_args(budget, model=None, improve_budget=None, improve=False):
    """The arguments `nightly` is scheduled with (shared by both backends)."""
    args = f"nightly --budget {budget:g}"
    if improve:
        args += f" --improve --improve-budget {improve_budget or IMPROVE_BUDGET_USD:g}"
    if model:
        args += f" --model {shlex.quote(model)}"
    return args


def systemd_units(times, budget, model=None, improve_budget=None, improve=False):
    """(service text, timer text): a oneshot run at low CPU and IO priority, like the owner's other timers, with the PATH baked in (a timer's environment has no cargo, claude or gh)."""
    path = os.environ.get("PATH", "")
    cargo = os.path.join(os.path.expanduser("~"), ".cargo", "bin")
    if os.path.isdir(cargo) and cargo not in path.split(os.pathsep):
        path = path + os.pathsep + cargo
    for t in times:
        m = re.fullmatch(r"(\d{1,2}):(\d{2})", t.strip())
        if not m or int(m.group(1)) > 23 or int(m.group(2)) > 59:
            sys.exit(f"bad time `{t}`: use HH:MM, 24-hour")
    service = f"""[Unit]
Description=Idea Forge: two games a day one at a time, and their feedback to the engine

[Service]
Type=oneshot
Nice=10
IOSchedulingClass=best-effort
IOSchedulingPriority=7
TimeoutStartSec=10h
WorkingDirectory={ROOT}
Environment=PATH={path}
Environment=IDEA_FORGE_HOME={state_dir()}
Environment=PYTHONUNBUFFERED=1
ExecStart={sys.executable} {os.path.join(HERE, 'idea_forge.py')} {nightly_args(budget, model, improve_budget, improve)}
"""
    timer = "[Unit]\nDescription=Run Idea Forge at " + ", ".join(times) + "\n\n[Timer]\n" + "".join(f"OnCalendar=*-*-* {int(t.split(':')[0]):02d}:{t.split(':')[1]}:00\n" for t in times) + "Persistent=false\n\n[Install]\nWantedBy=timers.target\n"
    return service, timer


def read_crontab():
    p = subprocess.run(["crontab", "-l"], capture_output=True, text=True)
    return p.stdout.splitlines() if p.returncode == 0 else []


def write_crontab(lines):
    p = subprocess.run(["crontab", "-"], input="\n".join(lines) + "\n", capture_output=True, text=True)
    if p.returncode:
        sys.exit("crontab failed: " + p.stderr.strip())


def cmd_schedule(a):
    backend = a.backend if a.backend != "auto" else ("systemd" if have_systemd_user() else "cron")
    per = (f"games one at a time (up to ${a.budget:g} each), then their feedback is written for the nightly maintenance job" + (f" and one engine improvement (up to ${a.improve_budget:g})" if a.improve else "")
           if a.command == "nightly" else f"one game (up to ${a.budget:g}) per run")
    if backend == "systemd":
        times = a.times.split(",")
        service, timer = systemd_units(times, a.budget, a.model, a.improve_budget, a.improve)
        d = systemd_dir()
        installed = os.path.exists(os.path.join(d, SYSTEMD_UNIT + ".timer"))
        if a.uninstall:
            subprocess.run(["systemctl", "--user", "disable", "--now", SYSTEMD_UNIT + ".timer"], capture_output=True, text=True)
            for ext in (".timer", ".service"):
                try:
                    os.remove(os.path.join(d, SYSTEMD_UNIT + ext))
                except OSError:
                    pass
            subprocess.run(["systemctl", "--user", "daemon-reload"], capture_output=True, text=True)
            print(f"removed the {SYSTEMD_UNIT} timer" if installed else "nothing to remove")
            return
        if not a.install:
            print(f"# {os.path.join(d, SYSTEMD_UNIT + '.service')}\n{service}\n# {os.path.join(d, SYSTEMD_UNIT + '.timer')}\n{timer}")
            print(f"(not installed: add --install to enable this systemd user timer. Each run: {per}; merges the tool's own PRs only after CI passes; --uninstall removes it)")
            if installed:
                print("installed now: yes")
            return
        os.makedirs(d, exist_ok=True)
        wr(os.path.join(d, SYSTEMD_UNIT + ".service"), service)
        wr(os.path.join(d, SYSTEMD_UNIT + ".timer"), timer)
        for cmd in (["daemon-reload"], ["enable", "--now", SYSTEMD_UNIT + ".timer"]):
            p = subprocess.run(["systemctl", "--user"] + cmd, capture_output=True, text=True)
            if p.returncode:
                sys.exit(f"systemctl --user {' '.join(cmd)} failed: {p.stderr.strip()}")
        print(f"installed {SYSTEMD_UNIT}.timer ({a.times}); logs: journalctl --user -u {SYSTEMD_UNIT}")
        return
    mine = [l for l in read_crontab() if CRON_MARK in l] if shutil.which("crontab") else []
    if a.uninstall:
        write_crontab([l for l in read_crontab() if CRON_MARK not in l])
        print(f"removed {len(mine)} idea-forge cron line(s)")
        return
    lines = cron_lines(a.times.split(","), a.budget, a.model, a.improve_budget, a.command, a.improve)
    if not a.install:
        print("\n".join(lines))
        print(f"\n(not installed: add --install to put these in your crontab. Each run: {per}; merges the tool's own PRs only after CI passes; --uninstall removes them)")
        if mine:
            print(f"\ninstalled now: {len(mine)} line(s)")
        return
    if not shutil.which("crontab"):
        sys.exit("no `crontab` on this machine: use the lines above in your scheduler of choice")
    write_crontab([l for l in read_crontab() if CRON_MARK not in l] + lines)
    print(f"installed {len(lines)} cron line(s); log: {os.path.join(state_dir(), 'nightly.log')}")


# ---- ledger and digest -------------------------------------------------------------------------------------------------------------------------------

def cmd_ledger(a):
    runs = ledger_rows(load_state()["runs"])
    if not runs:
        print("no runs yet: `idea_forge.py run`")
    for r in runs:
        code = r.get("code") if isinstance(r.get("code"), int) else 0
        print(f"{r.get('date', '?')}  {r.get('kind', '2d')}  {code:010d}  {row_slug(r) or '?':<28} {r.get('status', '?'):<10} {r.get('minutes', '-'):>5} min  notes {r.get('notes', '-')}  feedback {'ok' if r.get('feedback_ok') else 'no'}  {r.get('pr', '')} {r.get('merge', '')}{' published' if r.get('published') else ''}" +
              (f"  NOT PUBLISHED since {r['publish_blocked'].get('since', '?')}: {r['publish_blocked'].get('reason', '')}" if isinstance(r.get("publish_blocked"), dict) and not r.get("published") else "") +
              (f"  feedback PR {r['feedback_pr']} {r.get('feedback_merge', '')}".rstrip() if r.get("feedback_pr") else ""))


def collect_findings(directory, skipped=None):
    """The findings of every feedback file in `directory`; a file with a missing or malformed findings block is skipped, and named in `skipped` (a list of (name, why)) when one is given."""
    rows = []
    for name in sorted(os.listdir(directory)) if os.path.isdir(directory) else []:
        if not name.endswith(".md") or name.startswith("DIGEST"):
            continue
        try:
            for f in read_findings(rd(os.path.join(directory, name))):
                rows.append(dict(f, run=name[:-3]))
        except ValueError as e:
            if skipped is not None:
                skipped.append((name, str(e)))
    return rows


def skipped_line(skipped):
    """One line naming the feedback files the backlog could not read ('' when there are none): a malformed note is skipped, never invisible."""
    if not skipped:
        return ""
    return f"SKIPPED {len(skipped)} feedback file(s) with a missing or malformed findings block, so none of their findings are ranked: " + "; ".join(f"{n} ({w})" for n, w in skipped) + "."


def kind_of(run_name):
    """`<date>-3d-<slug>` is a 3D run; `<date>-2d-<slug>` and the files from before kinds existed are 2D."""
    return "3d" if re.match(r"\d{4}-\d{2}-\d{2}-3d-", run_name) else "2d"


def digest_text(rows, skipped=None):
    """Findings from every run grouped by area, ranked by (severity, minutes, how many runs hit it): the engine backlog this loop produces."""
    if skipped:
        return digest_text(rows) + "\n" + skipped_line(skipped) + "\n"
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
    skipped = []
    text = digest_text(collect_findings(directory, skipped), skipped)
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
    s.add_argument("--timeout-min", type=float, default=150)
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

    s = sub.add_parser("wiki", help="(the agent) the game's wiki page: --init a template, --preview draws its pictures to look at, --check validates")
    g = s.add_mutually_exclusive_group(required=True)
    g.add_argument("--init", action="store_true")
    g.add_argument("--preview", action="store_true")
    g.add_argument("--check", action="store_true")
    s.add_argument("--run")
    s.set_defaults(f=cmd_wiki)

    s = sub.add_parser("ship", help="commit the game, its wiki page and its feedback, push, open a PR (refused, with the reason, when the page is missing or fails `wiki --check`)")
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
    s.add_argument("--timeout-min", type=float, default=150)
    s.add_argument("--claude", default="claude")
    s.add_argument("--date", help="YYYY-MM-DD (default today)")
    s.add_argument("--order", help="force today's order, e.g. 3d,2d (default: shuffled once per day)")
    s.add_argument("--dry-run", action="store_true", help="print today's order and what would run, run nothing")
    s.add_argument("--quiet", action="store_true")
    s.set_defaults(f=cmd_daily, no_story=False, dry_run=False, workdir=None)

    def agent_opts(s, budget_default):
        s.add_argument("--budget", type=float, default=budget_default, help="USD cap per agent session")
        s.add_argument("--model")
        s.add_argument("--timeout-min", type=float, default=150)
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
    s.add_argument("--improve-timeout-min", type=float, default=120)
    s.add_argument("--no-ship", action="store_true")
    s.add_argument("--keep", action="store_true")
    s.add_argument("--date")
    s.add_argument("--order", help="force today's game order, e.g. 3d,2d (default: shuffled once per day)")
    s.add_argument("--skip-games", action="store_true")
    s.add_argument("--improve", action="store_true", help="also run this tool's own engine-improvement agent (off by default: the owner's nightly maintenance job reads the exported feedback instead)")
    s.add_argument("--min-free-gb", type=int, default=20, help="skip the night below this much free disk")
    s.add_argument("--settle-timeout-min", type=int, default=120, help="how long to wait at the end for tonight's PRs' checks, then merge those that passed (the next run settles the rest)")
    s.add_argument("--no-merge", action="store_true", help="open the PRs but do not merge any, this run")
    s.add_argument("--no-publish", action="store_true", help="do not put merged games into RedEngineGames, this run")
    s.set_defaults(f=cmd_nightly, no_story=False, workdir=None)

    s = sub.add_parser("settle", help="merge the tool's own PRs whose CI has passed")
    s.add_argument("--wait-min", type=int, default=0, help="keep waiting up to this long for running checks")
    s.set_defaults(f=cmd_settle)

    s = sub.add_parser("publish", help="put the merged games that are not in RedEngineGames yet there (a commit to its main; its workflow then builds and lists them)")
    s.add_argument("--dry-run", action="store_true", help="say what would be published, change nothing")
    s.set_defaults(f=cmd_publish)

    s = sub.add_parser("config", help="show or change settings")
    s.add_argument("--auto-merge", choices=("on", "off"), help="merge the tool's PRs once CI passes (default on)")
    s.add_argument("--publish", choices=("on", "off"), help="put merged games into RedEngineGames (default on)")
    s.set_defaults(f=cmd_config)

    s = sub.add_parser("schedule")
    s.add_argument("--times", default="06:00", help="HH:MM,HH:MM local times of day (default 06:00)")
    s.add_argument("--budget", type=float, default=DAILY_BUDGET_USD, help="USD cap per game")
    s.add_argument("--improve-budget", type=float, default=IMPROVE_BUDGET_USD, help="USD cap for the engine-improvement step")
    s.add_argument("--command", choices=("nightly", "daily"), default="nightly", help="nightly = both games one at a time (default); daily = one game per run (--next); cron backend only")
    s.add_argument("--backend", choices=("auto", "systemd", "cron"), default="auto", help="auto = a systemd user timer when `systemctl --user` works (as the owner's other jobs), else cron")
    s.add_argument("--improve", action="store_true", help="also run this tool's own engine-improvement agent each night (off by default)")
    s.add_argument("--model")
    g = s.add_mutually_exclusive_group()
    g.add_argument("--install", action="store_true")
    g.add_argument("--uninstall", action="store_true")
    s.set_defaults(f=cmd_schedule)

    s = sub.add_parser("backlog")
    s.add_argument("--json", action="store_true")
    s.add_argument("--export", action="store_true", help=f"write the open backlog to ../{EXPORT_NAME} (the nightly maintenance job reads it)")
    s.add_argument("--status", default="open,recurring,in-progress", help="comma-separated: open, recurring, in-progress, addressed, rejected")
    s.set_defaults(f=cmd_backlog)

    sub.add_parser("ledger").set_defaults(f=cmd_ledger)
    s = sub.add_parser("digest")
    s.add_argument("--write", action="store_true")
    s.set_defaults(f=cmd_digest)

    a = ap.parse_args(argv)
    a.argv = list(sys.argv[1:] if argv is None else argv)   # the command as given, for `restart_if_refreshed`
    a.f(a)


if __name__ == "__main__":
    main()
