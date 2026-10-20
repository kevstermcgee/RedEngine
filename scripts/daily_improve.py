#!/usr/bin/env python3
"""The daily improver: once a day an AI session reads all the feedback (scripts/feedback.py digest), fixes what it can verify in the engine, the
RedEngine Games app and its website, and every change lands as a pull request that merges itself once its hosted CI passes.

  run [--force] [--dry-run] [--no-merge]   the day's run (what the timer starts; --dry-run prints the brief and changes nothing)
  prompt                                    the brief the session would get now, with today's inbox (read it before trusting it)
  settle [--wait-min N]                     merge earlier daily pull requests whose checks passed, close stale failed ones, list the rest
  install                                   make this machine's `redengine-nightly` (its systemd timer, 23:00) run this script from origin/main

How a day goes (docs/DAILY_IMPROVER.md):
  1. Settle earlier days' pull requests (`daily/<date>` branches): merge the ones whose checks passed, close a failed one after three days.
  2. Read the inbox since the last run. The brief is `scripts/daily_improve.md` with the inbox and the open daily pull requests filled in.
  3. One Claude Code session (bypass permissions, a dollar cap, a deadline it cannot miss, no push/merge/GitHub-writing tools) works in two
     checkouts on `daily/<date>`: this engine checkout, and a RedEngineGames worktree beside it. Uncommitted work is thrown away.
  4. For each repository with commits: refuse paths the improver must not touch (CI, its own rules, versions and dependencies, maintained
     games, release lists), run the local checks, push, open a pull request with the session's report, and (merging on, local checks green,
     nothing refused) wait for hosted CI and merge it when every check passed. Anything else stays open for a maintainer, with the reason.

Settings: `~/.config/redengine-nightly/env` (the same file as before): NIGHTLY_MAX_USD, NIGHTLY_CLAUDE_TIMEOUT, NIGHTLY_CI_TIMEOUT,
NIGHTLY_MIN_FREE_GB, NIGHTLY_PUSH_MAIN (1 = merge green pull requests, 0 = leave them for a maintainer), NIGHTLY_MODEL, NIGHTLY_WAIT_MIN.
Reports: ~/nightly-reports/<date>.md (latest.md), the session's transcript beside it.
"""
from __future__ import annotations

import argparse
import datetime
import json
import os
import re
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, HERE)
import feedback  # noqa: E402  (the inbox)
import idea_forge  # noqa: E402  (agent streaming, the checks verdict)

HOME = os.path.expanduser("~")
CONF = os.path.join(HOME, ".config", "redengine-nightly", "env")
STATE = os.path.join(HOME, ".local", "state", "redengine-nightly")
REPORTS = os.path.join(HOME, "nightly-reports")
GAMES_CLONE = os.path.join(HOME, "workspace", "RedEngineGames")
GAMES_WT = os.path.join(HOME, "workspace", "RedEngineGames-nightly")
ENGINE_TARGET = "/data/build/nightly"
LAUNCHER_TARGET = "/data/build/nightly-launcher"
REPOS = {"engine": "kevstermcgee/RedEngine", "games": "kevstermcgee/RedEngineGames"}
BRIEF = os.path.join(HERE, "daily_improve.md")
STALE_DAYS = 3
DEFAULTS = {"NIGHTLY_MAX_USD": "25", "NIGHTLY_CLAUDE_TIMEOUT": "3h", "NIGHTLY_CI_TIMEOUT": "100m", "NIGHTLY_MIN_FREE_GB": "20", "NIGHTLY_PUSH_MAIN": "1",
            "NIGHTLY_MODEL": "", "NIGHTLY_WAIT_MIN": "90"}
# The session may not use these; the wrapper does what they would (push, pull requests, merges) under its own rules.
DISALLOWED = ["Bash(git push:*)", "Bash(sudo:*)", "Bash(systemctl:*)", "Bash(pkill:*)", "Bash(killall:*)", "Bash(gh issue comment:*)", "Bash(gh issue close:*)",
              "Bash(gh issue create:*)", "Bash(gh issue edit:*)", "Bash(gh pr create:*)", "Bash(gh pr merge:*)", "Bash(gh pr close:*)", "Bash(gh pr comment:*)",
              "Bash(gh pr edit:*)", "Bash(gh pr review:*)", "Bash(gh pr ready:*)", "Bash(gh pr reopen:*)", "Bash(gh release:*)", "Bash(gh repo:*)",
              "Bash(gh api -X:*)", "Bash(gh api --method:*)", "Bash(gh workflow:*)"]
# What a daily pull request may never change, whatever its CI says (it then waits for a maintainer).
NEVER = {
    "engine": (".github/", "scripts/daily_improve.py", "scripts/daily_improve.md", "Cargo.toml", "Cargo.lock", "docs/player-reviews/"),
    "games": (".github/", "launcher/Cargo.toml", "launcher/Cargo.lock", ".release-games.json", ".games-catalog.json", "games/", "projects/", "prototypes/",
              "tests/", "demos/"),
}


def say(*parts):
    print(f"[{datetime.datetime.now():%H:%M:%S}]", *parts, flush=True)


# ---- settings ---------------------------------------------------------------------------------------------------------------------------------------


def load_config(path=CONF, env=None):
    """The settings: the env file's `KEY=value` lines (comments and quotes allowed), then the process environment, over the defaults."""
    cfg = dict(DEFAULTS)
    try:
        with open(path, encoding="utf-8") as f:
            for line in f:
                line = line.split("#", 1)[0].strip()
                if "=" in line:
                    k, v = line.split("=", 1)
                    cfg[k.strip()] = v.strip().strip('"').strip("'")
    except OSError:
        pass
    for k in DEFAULTS:
        if (env or os.environ).get(k):
            cfg[k] = (env or os.environ)[k]
    return cfg


def seconds(text):
    """`3h`, `100m`, `90s` or a number of seconds."""
    m = re.fullmatch(r"\s*(\d+(?:\.\d+)?)\s*([hms]?)\s*", str(text))
    if not m:
        raise ValueError(f"not a duration: {text!r}")
    return float(m.group(1)) * {"h": 3600, "m": 60, "s": 1, "": 1}[m.group(2)]


# ---- git and GitHub ---------------------------------------------------------------------------------------------------------------------------------


def git(wt, *args, check=True):
    p = subprocess.run(["git", "-C", wt, *args], capture_output=True, text=True)
    if check and p.returncode:
        raise RuntimeError(f"git {' '.join(args[:3])} in {wt}: {(p.stderr or p.stdout).strip()}")
    return p.stdout.strip()


def gh(*args, check=True, cwd=None):
    p = subprocess.run(["gh", *args], capture_output=True, text=True, cwd=cwd or STATE)
    if check and p.returncode:
        raise RuntimeError(f"gh {' '.join(args[:3])}: {(p.stderr or p.stdout).strip()}")
    return p.stdout


def free_gb(path=HOME):
    return shutil.disk_usage(path).free / 1e9


def open_daily_prs(repo):
    """The open pull requests from `daily/` branches, oldest first."""
    try:
        prs = json.loads(gh("pr", "list", "-R", repo, "--state", "open", "--limit", "50", "--json",
                            "number,title,headRefName,isDraft,createdAt,mergeable,statusCheckRollup,files,url"))
    except (RuntimeError, ValueError):
        return []
    return sorted((p for p in prs if p.get("headRefName", "").startswith("daily/")), key=lambda p: p["createdAt"])


# ---- what a daily pull request may change ------------------------------------------------------------------------------------------------------------


def maintained_paths(manifest):
    """Every path of a game that has a maintainer (its source, a 2D game's generator folder, its page): the daily improver never changes those."""
    out = []
    sources = {e.get("destination"): e.get("source", "") for e in manifest.get("collections", {}).get("games", []) if isinstance(e, dict)}
    for p in manifest.get("playables", []):
        if isinstance(p, dict) and p.get("maintainer"):
            src = sources.get(p["slug"], "")
            out += [x for x in (src, src[: -len(".game2d.json")] + "/" if src.endswith(".game2d.json") else "", p.get("page", "")) if x]
    return out


def refused(kind, paths, manifest=None):
    """The changed paths a daily pull request may not touch, with why."""
    bad = []
    never = NEVER[kind]
    games = maintained_paths(manifest or {}) if kind == "engine" else []
    for path in paths:
        hit = next((n for n in never if path == n or path.startswith(n) or (n.endswith(".toml") and path.endswith("/" + n)) or (n == "Cargo.lock" and path.endswith("/Cargo.lock"))), None)
        if hit:
            bad.append(f"`{path}` (the daily improver never changes {hit})")
            continue
        g = next((x for x in games if path == x.rstrip("/") or path.startswith(x.rstrip("/") + "/")), None)
        if g:
            bad.append(f"`{path}` (a maintained game: only its maintainer changes it)")
    return bad


# ---- the decision about an open daily pull request ---------------------------------------------------------------------------------------------------


def decide(pr, kind, merge_on, today, manifest=None):
    """What to do with an open daily pull request: (`merge` | `close` | `wait` | `hold`, why)."""
    verdict = idea_forge.checks_verdict(pr.get("statusCheckRollup") or [])
    bad = refused(kind, [f.get("path", "") for f in pr.get("files") or []], manifest)
    age = (datetime.date.fromisoformat(today) - datetime.date.fromisoformat(pr["createdAt"][:10])).days
    if bad:
        return "hold", "needs a maintainer: it changes " + "; ".join(bad[:3])
    if pr.get("isDraft"):
        return ("close", f"draft for {age} days") if age >= STALE_DAYS else ("hold", "a draft: its local checks failed")
    if verdict == "failed":
        return ("close", f"its checks failed and it has waited {age} days") if age >= STALE_DAYS else ("hold", "its checks failed")
    if verdict == "pending" and not (pr.get("statusCheckRollup") or []) and age >= 1:
        return "hold", "no hosted check ran on it (nothing it changed has one): a maintainer merges it"
    if verdict == "pending":
        return "wait", "its checks are still running"
    if pr.get("mergeable") == "CONFLICTING":
        return ("close", "it conflicts with main") if age >= STALE_DAYS else ("hold", "it conflicts with main")
    return ("merge", "every check passed") if merge_on else ("hold", "every check passed; merging is off (NIGHTLY_PUSH_MAIN=0): a maintainer merges")


def act(pr, kind, action, why):
    repo = REPOS[kind]
    if action == "merge":
        gh("pr", "merge", str(pr["number"]), "-R", repo, "--merge", "--delete-branch")
        return f"merged {repo}#{pr['number']} ({why})"
    if action == "close":
        gh("pr", "close", str(pr["number"]), "-R", repo, "--delete-branch", "--comment", f"Closed by the daily improver: {why}. The work is in this branch's history if anyone wants it.")
        return f"closed {repo}#{pr['number']} ({why})"
    return f"{repo}#{pr['number']} left open: {why}"


def manifest_on_main(wt):
    try:
        return json.loads(git(wt, "show", "origin/main:games-publish.json"))
    except (RuntimeError, ValueError):
        return {}


def settle(cfg, today, wait_min=0, only=None):
    """Act on every open daily pull request; with `wait_min`, wait that long for running checks first. Returns one line per pull request."""
    merge_on = cfg["NIGHTLY_PUSH_MAIN"] == "1"
    manifest = manifest_on_main(ROOT)
    deadline = time.monotonic() + wait_min * 60
    lines = []
    for kind in ("engine", "games"):
        for pr in open_daily_prs(REPOS[kind]):
            if only and pr["number"] not in only.get(kind, ()):
                continue
            while True:
                action, why = decide(pr, kind, merge_on, today, manifest)
                if action != "wait" or time.monotonic() >= deadline:
                    break
                time.sleep(60)
                pr = next((p for p in open_daily_prs(REPOS[kind]) if p["number"] == pr["number"]), pr)
            try:
                lines.append(act(pr, kind, action, why))
            except RuntimeError as e:
                lines.append(f"{REPOS[kind]}#{pr['number']}: could not {action} it ({e})")
    return lines


# ---- the brief -------------------------------------------------------------------------------------------------------------------------------------


def open_prs_text(prs_by_kind):
    rows = []
    for kind, prs in prs_by_kind.items():
        for p in prs:
            verdict = idea_forge.checks_verdict(p.get("statusCheckRollup") or [])
            rows.append(f"- {REPOS[kind]}#{p['number']} ({p['headRefName']}, checks {verdict}{', draft' if p.get('isDraft') else ''}): {p['title']}")
    if not rows:
        return "No earlier daily pull request is open."
    return ("Earlier daily pull requests still open (do not redo their work; if one failed because of its own change and the fix is small, "
            "redo that item correctly tonight and say so):\n" + "\n".join(rows))


def oldest_keys(backlog, n=2):
    """The open keys reported first: tonight's to check against main (a fix nobody recorded leaves its key open forever)."""
    open_ = [g for g in backlog if g["status"] in ("open", "recurring", "partial") and g.get("runs")]
    open_.sort(key=lambda g: (min(r[:10] for r in g["runs"]), g["key"]))
    lines = [f"`{g['key']}` (first reported {min(r[:10] for r in g['runs'])}, {g['status']}): {feedback.clean(g['title'], 120)}" for g in open_[:n]]
    return "; ".join(lines) or "(no open key)"


def brief(inbox, prs_by_kind, *, engine_wt, games_wt, branch, report, scratch, date, template=BRIEF):
    with open(template, encoding="utf-8") as f:
        text = f.read()
    return text.format(maintainers=", ".join(f"@{m}" for m in inbox["maintainers"]), engine_wt=engine_wt, games_wt=games_wt, branch=branch,
                       report=report, scratch=scratch, since=inbox["since"], date=date, digest=feedback.digest_text(inbox, limit=25).strip(),
                       open_prs=open_prs_text(prs_by_kind), oldest=oldest_keys(inbox["backlog"]))


# ---- the checkouts -----------------------------------------------------------------------------------------------------------------------------------


def prepare(engine_wt, games_wt, branch):
    """Both checkouts on `branch` from origin/main, clean."""
    git(engine_wt, "fetch", "-q", "origin")
    git(engine_wt, "reset", "-q", "--hard")
    git(engine_wt, "clean", "-qfd")
    git(engine_wt, "checkout", "-q", "-B", branch, "origin/main")
    git(GAMES_CLONE, "fetch", "-q", "origin")
    if not os.path.isdir(games_wt):
        git(GAMES_CLONE, "worktree", "add", "-q", "--detach", games_wt, "origin/main")
    git(games_wt, "reset", "-q", "--hard")
    git(games_wt, "clean", "-qfd")
    git(games_wt, "checkout", "-q", "-B", branch, "origin/main")


def changed_paths(wt):
    out = git(wt, "diff", "--name-only", "origin/main...HEAD")
    return [p for p in out.splitlines() if p]


def local_checks(kind, wt, paths, timeout, log):
    """Run the repository's own checks for what changed; (passed, what ran)."""
    env = dict(os.environ)
    if kind == "engine":
        env["CARGO_TARGET_DIR"] = ENGINE_TARGET
        steps = [["build-lock", "scripts/dev", "affected"]]
    else:
        env["CARGO_TARGET_DIR"] = LAUNCHER_TARGET
        steps = []
        if any(p.startswith("site/") for p in paths):
            steps.append(["python3", "-m", "unittest", "discover", "-s", "site"])
        if any(p.startswith("distribution/") for p in paths):
            steps.append(["python3", "-m", "unittest", "discover", "-s", "distribution"])
        if any(p.startswith("launcher/") for p in paths):
            steps.append(["build-lock", "cargo", "test", "--manifest-path", "launcher/Cargo.toml", "-p", "redengine-games-core"])
    with open(log, "a", encoding="utf-8") as out:
        for argv in steps:
            out.write(f"\n$ {' '.join(argv)}\n")
            out.flush()
            try:
                code = subprocess.run(argv, cwd=wt, env=env, stdout=out, stderr=subprocess.STDOUT, timeout=timeout).returncode
            except subprocess.TimeoutExpired:
                return False, f"`{' '.join(argv)}` timed out"
            except OSError as e:
                return False, f"`{' '.join(argv)}` could not start ({e})"
            if code:
                return False, f"`{' '.join(argv)}` failed (exit {code}; {log})"
    return True, ", ".join(f"`{' '.join(a)}`" for a in steps) or "nothing to run for these paths"


def report_title(report_path, date):
    try:
        with open(report_path, encoding="utf-8") as f:
            first = next((ln.strip() for ln in f if ln.strip()), "")
    except OSError:
        first = ""
    title = first.lstrip("#").strip()[:70]
    return f"Daily improver {date}: {title}" if title else f"Daily improver {date}"


def land(kind, wt, branch, report, date, cfg, manifest, log):
    """Push the day's commits in `wt` and open their pull request; merge it when allowed and hosted CI passes. Returns the outcome line."""
    stray = git(wt, "status", "--porcelain")
    if stray:
        git(wt, "reset", "-q", "--hard")
        git(wt, "clean", "-qfd")
    n = int(git(wt, "rev-list", "--count", "origin/main..HEAD") or 0)
    note = f" (threw away {len(stray.splitlines())} uncommitted file(s))" if stray else ""
    if n == 0:
        return f"{REPOS[kind]}: no commits{note}."
    paths = changed_paths(wt)
    bad = refused(kind, paths, manifest)
    ok, ran = local_checks(kind, wt, paths, seconds(cfg["NIGHTLY_CI_TIMEOUT"]), log) if not bad else (False, "not run: refused paths")
    git(wt, "push", "-q", "--force", "origin", f"HEAD:refs/heads/{branch}")
    body = ""
    try:
        with open(report, encoding="utf-8") as f:
            body = f.read()
    except OSError:
        body = "(the session wrote no report)"
    status = ("Local checks passed: " + ran) if ok else ("**Needs a maintainer.** " + ("It changes " + "; ".join(bad) if bad else "Local checks failed: " + ran))
    body += (f"\n\n---\n{status}.\n\nOpened by the daily improver (`scripts/daily_improve.py`, docs/DAILY_IMPROVER.md). "
             + ("It merges itself once every hosted check passes; close it to stop that." if ok and cfg["NIGHTLY_PUSH_MAIN"] == "1" else "It will not merge itself.")
             + "\n\n🤖 Generated with [Claude Code](https://claude.com/claude-code)\n")
    args = ["pr", "create", "-R", REPOS[kind], "--base", "main", "--head", branch, "--title", report_title(report, date), "--body", body]
    if not ok:
        args.append("--draft")
    url = gh(*args).strip().splitlines()[-1]
    number = int(url.rstrip("/").rsplit("/", 1)[-1])
    if not ok or cfg["NIGHTLY_PUSH_MAIN"] != "1":
        return f"{REPOS[kind]}: {n} commit(s){note}; opened {url} ({'needs a maintainer' if not ok else 'merging is off'})."
    lines = settle(cfg, date, wait_min=float(cfg["NIGHTLY_WAIT_MIN"]), only={kind: (number,)})
    return f"{REPOS[kind]}: {n} commit(s){note}; opened {url}; " + ("; ".join(lines) or "left open") + "."


# ---- run ---------------------------------------------------------------------------------------------------------------------------------------------


class Day:
    def __init__(self, date):
        self.date = date
        os.makedirs(REPORTS, exist_ok=True)
        os.makedirs(STATE, exist_ok=True)
        self.report = os.path.join(REPORTS, f"{date}.md")
        self.log = os.path.join(REPORTS, f"{date}.log")
        self.scratch = os.path.join(STATE, "scratch", date)

    def finish(self, *lines):
        with open(self.report, "a", encoding="utf-8") as f:
            f.write("\n## Wrapper result\n" + "".join(f"- {ln}\n" for ln in lines) + f"- Log: {self.log}\n")
        latest = os.path.join(REPORTS, "latest.md")
        if os.path.lexists(latest):
            os.remove(latest)
        os.symlink(self.report, latest)
        for ln in lines:
            say(ln)


def last_success():
    try:
        with open(os.path.join(STATE, "last-success"), encoding="utf-8") as f:
            return f.read().strip()[:10]
    except OSError:
        return (datetime.date.today() - datetime.timedelta(days=2)).isoformat()


def cmd_run(a):
    cfg = load_config()
    date = datetime.date.today().isoformat()
    day = Day(date)
    import fcntl  # the improver runs on the Linux box; the module is imported (and tested) on Windows too
    lock = open(os.path.join(STATE, "run.lock"), "w")
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        say("another daily run is active")
        return 0
    if free_gb() < float(cfg["NIGHTLY_MIN_FREE_GB"]):
        with open(day.report, "w", encoding="utf-8") as f:
            f.write(f"# Daily improver {date}: skipped (low disk)\n")
        day.finish(f"Skipped: {free_gb():.0f} GB free (< {cfg['NIGHTLY_MIN_FREE_GB']}).")
        return 0
    if a.no_merge:
        cfg["NIGHTLY_PUSH_MAIN"] = "0"
    settled = [] if a.dry_run else settle(cfg, date)
    since = last_success()
    inbox = feedback.collect(since)
    fresh = feedback.new_count(inbox)
    open_keys = sum(1 for g in inbox["backlog"] if g["status"] in ("open", "recurring", "partial"))
    if not a.force and fresh == 0 and open_keys == 0:
        with open(day.report, "w", encoding="utf-8") as f:
            f.write(f"# Daily improver {date}: nothing to do\n\nNo new feedback since {since} and no open key in the backlog.\n")
        day.finish(*settled, "Nothing to do.")
        return 0
    branch = f"daily/{date}"
    prs = {kind: open_daily_prs(REPOS[kind]) for kind in REPOS}
    text = brief(inbox, prs, engine_wt=ROOT, games_wt=GAMES_WT, branch=branch, report=day.report, scratch=day.scratch, date=date)
    if a.dry_run:
        print(text)
        return 0
    prepare(ROOT, GAMES_WT, branch)
    os.makedirs(day.scratch, exist_ok=True)
    if os.path.exists(day.report):
        os.remove(day.report)
    say(f"inbox: {fresh} new since {since}, {open_keys} open key(s); starting the session (cap ${cfg['NIGHTLY_MAX_USD']}, {cfg['NIGHTLY_CLAUDE_TIMEOUT']})")
    argv = ["claude", "-p", text, "--permission-mode", "bypassPermissions", "--max-budget-usd", cfg["NIGHTLY_MAX_USD"], "--no-session-persistence",
            "--output-format", "stream-json", "--verbose", "--add-dir", GAMES_WT, "--add-dir", REPORTS, "--add-dir", day.scratch,
            "--disallowedTools", *DISALLOWED]
    if cfg["NIGHTLY_MODEL"]:
        argv[3:3] = ["--model", cfg["NIGHTLY_MODEL"]]
    env = dict(os.environ, CARGO_TARGET_DIR=ENGINE_TARGET)
    outcome = idea_forge.stream_agent(argv, ROOT, env, os.path.join(REPORTS, f"{date}.claude.jsonl"), seconds(cfg["NIGHTLY_CLAUDE_TIMEOUT"]), quiet=False)
    say(f"session {outcome['status']} after {outcome.get('elapsed_s', 0):.0f} s")
    if not os.path.exists(day.report):
        with open(day.report, "w", encoding="utf-8") as f:
            f.write(f"# Daily improver {date}: no report\n\nThe session ({outcome['status']}) wrote no report; its transcript is {date}.claude.jsonl.\n")
    with open(os.path.join(STATE, "last-success"), "w", encoding="utf-8") as f:
        f.write(datetime.datetime.now().astimezone().isoformat())   # the feedback is handled (or reported) once, never retried in a loop
    manifest = manifest_on_main(ROOT)
    results = []
    for kind, wt in (("engine", ROOT), ("games", GAMES_WT)):
        try:
            results.append(land(kind, wt, branch, day.report, date, cfg, manifest, day.log))
        except (RuntimeError, ValueError) as e:
            results.append(f"{REPOS[kind]}: could not land ({e}); the commits are on {branch} in {wt}.")
    day.finish(*settled, f"Session: {outcome['status']}.", *results)
    return 0


def cmd_prompt(a):
    date = datetime.date.today().isoformat()
    inbox = feedback.collect(last_success())
    prs = {kind: open_daily_prs(REPOS[kind]) for kind in REPOS}
    print(brief(inbox, prs, engine_wt=ROOT, games_wt=GAMES_WT, branch=f"daily/{date}", report=os.path.join(REPORTS, f"{date}.md"),
                scratch=os.path.join(STATE, "scratch", date), date=date))
    return 0


def cmd_settle(a):
    os.makedirs(STATE, exist_ok=True)
    for line in settle(load_config(), datetime.date.today().isoformat(), wait_min=a.wait_min) or ["no daily pull request is open"]:
        print(line)
    return 0


SHIM = """#!/usr/bin/env bash
# redengine-nightly: RedEngine's daily improver (scripts/daily_improve.py on origin/main; docs/DAILY_IMPROVER.md). Written by
# `daily_improve.py install`; the previous script is beside it as {backup}.
set -uo pipefail
export PATH="$HOME/.local/bin:$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin"
REPO="$HOME/workspace/RedEngine"; WT="$HOME/workspace/RedEngine-nightly"
git -C "$REPO" fetch -q origin || {{ echo "git fetch failed; trying again tomorrow"; exit 0; }}
[ -d "$WT" ] || git -C "$REPO" worktree add -q --detach "$WT" origin/main
git -C "$WT" reset -q --hard && git -C "$WT" clean -qfd && git -C "$WT" checkout -q --detach origin/main
exec python3 "$WT/scripts/daily_improve.py" run "$@"
"""


def cmd_install(a):
    path = os.path.join(HOME, ".local", "bin", "redengine-nightly")
    backup = f"{path}.bak-{datetime.date.today():%Y%m%d}"
    if os.path.exists(path) and not os.path.exists(backup):
        shutil.copy2(path, backup)
    with open(path, "w", encoding="utf-8") as f:
        f.write(SHIM.format(backup=os.path.basename(backup)))
    os.chmod(path, 0o755)
    print(f"wrote {path} (previous: {backup}); the redengine-nightly timer now runs the daily improver from origin/main.")
    print("Check: systemctl --user list-timers redengine-nightly.timer; a dry run: python3 scripts/daily_improve.py run --dry-run")
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("run")
    s.add_argument("--force", action="store_true", help="run even with nothing new and nothing open")
    s.add_argument("--dry-run", action="store_true", help="print the brief; change nothing")
    s.add_argument("--no-merge", action="store_true", help="open pull requests but merge nothing (this run)")
    s.set_defaults(f=cmd_run)
    s = sub.add_parser("prompt")
    s.set_defaults(f=cmd_prompt)
    s = sub.add_parser("settle")
    s.add_argument("--wait-min", type=float, default=0)
    s.set_defaults(f=cmd_settle)
    s = sub.add_parser("install")
    s.set_defaults(f=cmd_install)
    a = ap.parse_args(argv)
    return a.f(a)


if __name__ == "__main__":
    sys.exit(main())
