#!/usr/bin/env python3
"""The daily improver: once a day an AI session reads all the feedback (scripts/feedback.py digest), fixes what it can verify in the engine, the
RedEngine Games app and its website, and every change lands as a pull request that merges itself once its hosted CI passes.

  run [--force] [--dry-run] [--no-merge]   the day's run (what the timer starts; --dry-run prints the brief and changes nothing)
  prompt                                    the brief the session would get now, with today's inbox (read it before trusting it)
  settle [--wait-min N]                     merge green daily pull requests, retry failed checks, update branches behind main
  install                                   make this machine's `redengine-nightly` (its systemd timer, 23:00) run this script from origin/main

How a day goes (docs/DAILY_IMPROVER.md):
  1. Settle earlier days' pull requests (`daily/<date>` branches): merge the ones whose checks passed, retry failures, repair one stuck PR.
  2. Read the inbox since the last run. The brief is `scripts/daily_improve.md` with the inbox and the open daily pull requests filled in.
  3. One Claude Code session (bypass permissions, a dollar cap, a deadline it cannot miss, no push/merge/GitHub-writing tools) works in two
     checkouts on `daily/<date>`: this engine checkout, and a RedEngineGames worktree beside it. Uncommitted work is thrown away.
  4. For each repository with commits: refuse paths the improver must not touch (CI, its own rules, versions and dependencies, maintained
     games, release lists), run the local checks, push, open a pull request with the session's report, and (merging on,
     nothing refused) wait for hosted CI and merge it when every check passed. Local failures are advisory; refused paths remain drafts.

Settings: `~/.config/redengine-nightly/env` (the same file as before): NIGHTLY_MAX_USD, NIGHTLY_CLAUDE_TIMEOUT, NIGHTLY_CI_TIMEOUT,
NIGHTLY_MIN_FREE_GB, NIGHTLY_PUSH_MAIN (1 = merge green pull requests, 0 = leave them for a maintainer), NIGHTLY_MODEL, NIGHTLY_WAIT_MIN.
Reports: ~/nightly-reports/<date>.md (latest.md), the session's transcript beside it.
"""
from __future__ import annotations

import argparse
import contextlib
import datetime
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, HERE)
import feedback  # noqa: E402  (the inbox)
import idea_forge  # noqa: E402  (agent streaming, the checks verdict)
import proc_supervisor  # noqa: E402

HOME = os.path.expanduser("~")
CONF = os.path.join(HOME, ".config", "redengine-nightly", "env")
STATE = os.path.join(HOME, ".local", "state", "redengine-nightly")
REPORTS = os.path.join(HOME, "nightly-reports")
GAMES_CLONE = os.path.join(HOME, "workspace", "RedEngineGames")
GAMES_WT = os.path.join(HOME, "workspace", "RedEngineGames-nightly")
NIGHTLY_WT = os.path.join(HOME, "workspace", "RedEngine-nightly")   # the checkout `install`'s shim runs this from (it is reset every run)
ENGINE_TARGET = "/data/build/nightly"
LAUNCHER_TARGET = "/data/build/nightly-launcher"
REPOS = {"engine": "kevstermcgee/RedEngine", "games": "kevstermcgee/RedEngineGames"}
BRIEF = os.path.join(HERE, "daily_improve.md")
DEFAULTS = {"NIGHTLY_MAX_USD": "25", "NIGHTLY_CLAUDE_TIMEOUT": "3h", "NIGHTLY_CI_TIMEOUT": "100m", "NIGHTLY_MIN_FREE_GB": "20", "NIGHTLY_PUSH_MAIN": "1",
            "NIGHTLY_MODEL": "", "NIGHTLY_WAIT_MIN": "90", "NIGHTLY_REPAIR_MAX_USD": "5", "NIGHTLY_REPAIR_TIMEOUT": "30m"}
# The session may not use these; the wrapper does what they would (push, pull requests, merges) under its own rules.
DISALLOWED = ["Bash(git push:*)", "Bash(sudo:*)", "Bash(systemctl:*)", "Bash(pkill:*)", "Bash(killall:*)", "Bash(gh issue comment:*)", "Bash(gh issue close:*)",
              "Bash(gh issue create:*)", "Bash(gh issue edit:*)", "Bash(gh pr create:*)", "Bash(gh pr merge:*)", "Bash(gh pr close:*)", "Bash(gh pr comment:*)",
              "Bash(gh pr edit:*)", "Bash(gh pr review:*)", "Bash(gh pr ready:*)", "Bash(gh pr reopen:*)", "Bash(gh release:*)", "Bash(gh repo:*)",
              "Bash(gh api -X:*)", "Bash(gh api --method:*)", "Bash(gh workflow:*)"]
# What a daily pull request may never change, whatever its CI says (it then waits for a maintainer).
NEVER = {
    "engine": (".github/", "scripts/daily_improve.py", "scripts/daily_improve.md", "scripts/test_daily_improve.py", "Cargo.toml", "Cargo.lock", "docs/player-reviews/"),
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
    p = subprocess.run(["git", "-C", wt, *args], capture_output=True, text=True, timeout=120)
    if check and p.returncode:
        raise RuntimeError(f"git {' '.join(args[:3])} in {wt}: {(p.stderr or p.stdout).strip()}")
    return p.stdout.strip()


def gh(*args, check=True, cwd=None):
    p = subprocess.run(["gh", *args], capture_output=True, text=True, cwd=cwd or (STATE if os.path.isdir(STATE) else HOME), timeout=120)
    if check and p.returncode:
        raise RuntimeError(f"gh {' '.join(args[:3])}: {(p.stderr or p.stdout).strip()}")
    return p.stdout


def free_gb(path=HOME):
    return shutil.disk_usage(path).free / 1e9


def open_daily_prs(repo):
    """Only this owner's daily branches targeting main, with complete file lists. API failures are never an empty inbox."""
    prs = json.loads(gh("pr", "list", "-R", repo, "--state", "open", "--limit", "100", "--json",
                        "number,title,body,headRefName,headRefOid,baseRefName,isCrossRepository,author,isDraft,createdAt,mergeable,mergeStateStatus,statusCheckRollup,url,changedFiles"))
    out = []
    for pr in prs:
        if (not re.fullmatch(r"daily/\d{4}-\d{2}-\d{2}(?:-[a-zA-Z0-9-]+)?", pr.get("headRefName", ""))
                or pr.get("baseRefName") != "main" or pr.get("isCrossRepository")
                or (pr.get("author") or {}).get("login") != repo.split("/")[0]):
            continue
        # gh 2.45 (the box's packaged version) has --paginate but no --slurp. JSON per line also preserves escaped filenames.
        files = [json.loads(line) for line in gh("api", "--paginate", f"repos/{repo}/pulls/{pr['number']}/files?per_page=100",
                 "--jq", ".[] | [.filename, .previous_filename] | @json").splitlines() if line]
        if len(files) != pr["changedFiles"]:
            raise RuntimeError(f"incomplete file list for {repo}#{pr['number']}; refusing to act")
        pr["files"] = [{"path": path} for names in files for path in names if path]
        out.append(pr)
    return sorted(out, key=lambda p: p["createdAt"])


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


def local_failure_draft(pr):
    """Only the old runner's local-failure drafts, not a new ready PR that a maintainer later drafts."""
    body = pr.get("body", "")
    return pr.get("isDraft") and all(s in body for s in ("Opened by the daily improver", "Local checks failed:", "It will not merge itself."))


def decide(pr, kind, merge_on, today, manifest=None):
    """What to do with an open daily pull request: (`merge` | `update` | `wait` | `hold`, why)."""
    verdict = idea_forge.checks_verdict(pr.get("statusCheckRollup") or [])
    bad = refused(kind, [f.get("path", "") for f in pr.get("files") or []], manifest)
    age = (datetime.date.fromisoformat(today) - datetime.date.fromisoformat(pr["createdAt"][:10])).days
    if bad:
        return "hold", "needs a maintainer: it changes " + "; ".join(bad[:3])
    # Older runners made local failures drafts. A green hosted run can recover these, but never an intentional draft.
    if pr.get("isDraft") and not local_failure_draft(pr):
        return "hold", "an intentional draft: left for its maintainer"
    if verdict == "failed":
        return "hold", "its checks failed; automatic recovery pending"
    if verdict == "pending" and not (pr.get("statusCheckRollup") or []) and age >= 1:
        return "hold", "no hosted check ran on it; waiting for CI"
    if verdict == "pending":
        return "wait", "its checks are still running"
    if pr.get("mergeable") == "CONFLICTING":
        return "hold", "it conflicts with main; automatic recovery will repair it"
    if pr.get("mergeable") != "MERGEABLE":
        return "wait", "GitHub is computing mergeability"
    if pr.get("mergeStateStatus") == "BEHIND":
        return "update", "main moved; update the branch and check the new commit"
    return ("merge", "every check passed") if merge_on else ("hold", "every check passed; merging is off (NIGHTLY_PUSH_MAIN=0): a maintainer merges")


def act(pr, kind, action, why):
    repo = REPOS[kind]
    if action == "merge":
        # Unprotected branches can report CLEAN even when main has moved. Check ancestry explicitly too.
        ancestry = gh("api", f"repos/{repo}/compare/{pr['headRefOid']}...main", "--jq", ".status").strip()
        if ancestry in ("ahead", "diverged"):
            return act(pr, kind, "update", "main moved")
        if ancestry not in ("behind", "identical"):
            raise RuntimeError("could not establish that the inspected head includes main")
        if pr.get("isDraft"):
            gh("pr", "ready", str(pr["number"]), "-R", repo)
        # Merge only the commit we inspected, even if another writer updates the PR in the meantime.
        gh("pr", "merge", str(pr["number"]), "-R", repo, "--merge", "--match-head-commit", pr["headRefOid"])
        return f"merged {repo}#{pr['number']} ({why})"
    if action == "update":
        gh("api", "--method", "PUT", f"repos/{repo}/pulls/{pr['number']}/update-branch", "-f", f"expected_head_sha={pr['headRefOid']}")
        return f"updated {repo}#{pr['number']}; waiting for checks on its new head"
    return f"{repo}#{pr['number']} left open: {why}"


def manifest_on_main(wt):
    # Missing ownership data must stop merges rather than silently allowing maintained games.
    manifest = json.loads(git(wt, "show", "origin/main:games-publish.json"))
    if not isinstance(manifest, dict) or not isinstance(manifest.get("playables"), list) or not isinstance(manifest.get("collections"), dict):
        raise RuntimeError("invalid ownership manifest; refusing to act")
    return manifest


def once(key, action, *, retry_error=False):
    """Persist a bounded recovery attempt before acting, so a restart cannot create an endless retry loop."""
    os.makedirs(os.path.join(STATE, "attempts"), exist_ok=True)
    path = os.path.join(STATE, "attempts", key)
    try:
        with open(path, "x", encoding="utf-8") as f:
            f.write(datetime.datetime.now().astimezone().isoformat())
    except FileExistsError:
        return False
    try:
        action()
    except (RuntimeError, subprocess.TimeoutExpired):
        if retry_error:
            os.unlink(path)  # A failed read-only dispatch can retry on the next tick; do not strand a PR on a transient API error.
        raise
    return True


@contextlib.contextmanager
def run_lock():
    """The shim takes this same lock BEFORE resetting either checkout; a direct command takes it here."""
    import fcntl
    os.makedirs(STATE, exist_ok=True)
    path = os.path.join(STATE, "run.lock")
    inherited = os.environ.get("RED_NIGHTLY_LOCK_FD")
    lock = None
    if inherited:
        try:
            fd = int(inherited)
            if os.fstat(fd).st_ino == os.stat(path).st_ino and os.fstat(fd).st_dev == os.stat(path).st_dev:
                lock = os.fdopen(os.dup(fd), "w")
        except (OSError, ValueError):
            pass
    with lock or open(path, "a") as handle:
        try:
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            yield False
            return
        yield True


def retry_checks(pr, kind):
    """One retry of failed PR runs (or Games' read-only fallback) per inspected head; never a stale head or publishing workflow."""
    repo = REPOS[kind]
    runs = json.loads(gh("api", f"repos/{repo}/actions/runs?head_sha={pr['headRefOid']}&per_page=100"))["workflow_runs"]
    failed = [r for r in runs if r["head_sha"] == pr["headRefOid"] and r.get("conclusion") in ("failure", "timed_out", "cancelled", "startup_failure")
              and (r.get("event") == "pull_request" or (kind == "games" and r.get("event") == "workflow_dispatch" and r.get("path") == ".github/workflows/checks.yml"))]
    if not failed:
        return False
    def retry():
        for run in failed:
            gh("api", "--method", "POST", f"repos/{repo}/actions/runs/{run['id']}/rerun-failed-jobs")
    return once(f"{kind}-{pr['number']}-{pr['headRefOid']}-ci", retry)


def dispatch_missing_checks(pr, kind):
    """Games' path filters omit ordinary docs. Dispatch its existing read-only validation, never a release workflow."""
    if kind != "games" or pr.get("statusCheckRollup"):
        return False
    paths = [f["path"] for f in pr.get("files") or []]
    if not paths or not all(p.endswith(".md") and not p.startswith(("launcher/", "site/", "distribution/")) and p != "docs/GAME_METADATA.md" for p in paths):
        return False  # Never substitute site validation for a missing launcher or other code workflow.
    created = datetime.datetime.fromisoformat(pr["createdAt"].replace("Z", "+00:00"))
    if (datetime.datetime.now(datetime.timezone.utc) - created).total_seconds() < 300:
        return False  # Allow the usual PR workflow to appear before dispatching a duplicate.
    return once(f"{kind}-{pr['number']}-{pr['headRefOid']}-dispatch", lambda: gh("workflow", "run", "checks.yml", "-R", REPOS[kind], "--ref", pr["headRefName"]), retry_error=True)


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
                if (merge_on and (not pr.get("isDraft") or local_failure_draft(pr))
                        and not refused(kind, [f["path"] for f in pr["files"]], manifest)):
                    try:
                        if dispatch_missing_checks(pr, kind):
                            lines.append(f"dispatched read-only hosted checks for {REPOS[kind]}#{pr['number']}")
                            action, why = "wait", "dispatched hosted checks; waiting for a real green result"
                    except (RuntimeError, subprocess.TimeoutExpired) as e:
                        lines.append(f"{REPOS[kind]}#{pr['number']}: could not dispatch checks ({e})")
                if action != "wait" or time.monotonic() >= deadline:
                    break
                time.sleep(min(60, max(0, deadline - time.monotonic())))
                pr = next((p for p in open_daily_prs(REPOS[kind]) if p["number"] == pr["number"]), None)
                if pr is None:  # A maintainer closed it while we waited: do not act on an old snapshot.
                    break
            if pr is None:
                continue
            try:
                if not merge_on:
                    action, why = "hold", "merging is off"
                if (merge_on and action == "hold" and (not pr.get("isDraft") or local_failure_draft(pr))
                        and not refused(kind, [f["path"] for f in pr["files"]], manifest)):
                    if idea_forge.checks_verdict(pr.get("statusCheckRollup") or []) == "failed" and retry_checks(pr, kind):
                        lines.append(f"retried failed checks for {REPOS[kind]}#{pr['number']}; the settle timer will check again")
                        continue
                lines.append(act(pr, kind, action, why))
            except (RuntimeError, subprocess.TimeoutExpired) as e:
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
    out = git(wt, "diff", "--name-only", "--no-renames", "origin/main...HEAD")
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
    deadline = time.monotonic() + timeout
    with open(log, "ab") as out:
        for argv in steps:
            out.write(f"\n$ {' '.join(argv)}\n".encode())
            out.flush()
            try:
                result = proc_supervisor.supervise(argv, cwd=wt, env=env, log=out, timeout=max(0, deadline - time.monotonic()))
            except OSError as e:
                return False, f"`{' '.join(argv)}` could not start ({e})"
            if result["status"] != "completed":
                return False, f"`{' '.join(argv)}` {result['status']} ({log})"
    return True, ", ".join(f"`{' '.join(a)}`" for a in steps) or "nothing to run for these paths"


def report_title(report_path, date):
    try:
        with open(report_path, encoding="utf-8") as f:
            first = next((ln.strip() for ln in f if ln.strip()), "")
    except OSError:
        first = ""
    title = first.lstrip("#").strip()[:70]
    return f"Daily improver {date}: {title}" if title else f"Daily improver {date}"


def repair_one(cfg, date):
    """At most one bounded repair session per night and two per PR. The existing PR keeps its history and CI gate."""
    if cfg["NIGHTLY_PUSH_MAIN"] != "1":
        return []
    manifest = manifest_on_main(ROOT)
    for kind in REPOS:
        for pr in open_daily_prs(REPOS[kind]):
            if refused(kind, [f["path"] for f in pr["files"]], manifest):
                continue
            if pr.get("isDraft") and not local_failure_draft(pr):
                continue
            if pr.get("mergeable") != "CONFLICTING" and idea_forge.checks_verdict(pr.get("statusCheckRollup") or []) != "failed":
                continue
            for attempt in range(2):
                key = f"{kind}-{pr['number']}-repair-{attempt}"
                result = []
                if once(key, lambda: result.append(repair_pr(pr, kind, cfg, date, manifest))):
                    return result
    return []


def repair_pr(pr, kind, cfg, date, manifest):
    repo = REPOS[kind]
    clone = ROOT if kind == "engine" else GAMES_CLONE
    report = os.path.join(REPORTS, f"{date}-repair-{kind}-{pr['number']}.md")
    with tempfile.TemporaryDirectory(prefix="repair-", dir=STATE) as scratch:
        wt = os.path.join(scratch, "checkout")
        git(clone, "fetch", "-q", "origin", "main", pr["headRefName"])
        git(clone, "worktree", "add", "-q", "--detach", wt, pr["headRefOid"])
        try:
            git(wt, "merge", "--no-edit", "origin/main", check=False)
            text = (f"Repair the existing daily improver PR {repo}#{pr['number']} in {wt}. Nobody can answer questions. "
                    f"Read AGENTS.md and CLAUDE.md, and the PR and its failed workflow logs with gh. Main has been merged, or a merge with conflicts is in progress. "
                    f"Resolve conflicts preserving both changes; fix the actual CI failure with a minimal change and regression test. "
                    f"Use build-lock for builds. Run focused checks, commit all repairs, and write {report}. "
                    "Never weaken, skip or remove a check, change dependencies, CI workflows, daily_improve.py or daily_improve.md, "
                    "or edit a maintained game. Never push, merge a PR, change services, or write to GitHub. "
                    "If the safe fix cannot be completed within 25 minutes, leave the PR for the next attempt and explain why.")
            argv = ["claude", "-p", text, "--permission-mode", "bypassPermissions", "--max-budget-usd", cfg["NIGHTLY_REPAIR_MAX_USD"],
                    "--no-session-persistence", "--output-format", "stream-json", "--verbose", "--add-dir", REPORTS,
                    "--disallowedTools", *DISALLOWED]
            if cfg["NIGHTLY_MODEL"]:
                argv[3:3] = ["--model", cfg["NIGHTLY_MODEL"]]
            outcome = idea_forge.stream_agent(argv, wt, dict(os.environ, CARGO_TARGET_DIR=ENGINE_TARGET if kind == "engine" else LAUNCHER_TARGET),
                                              report + ".jsonl", seconds(cfg["NIGHTLY_REPAIR_TIMEOUT"]), quiet=False)
            if outcome["status"] != "completed" or git(wt, "status", "--porcelain"):
                return f"{repo}#{pr['number']}: repair incomplete ({outcome['status']}); existing PR preserved"
            if git(wt, "rev-parse", "HEAD") == pr["headRefOid"]:
                return f"{repo}#{pr['number']}: repair made no commits; existing PR preserved"
            for ancestor in (pr["headRefOid"], "origin/main"):
                # merge-base equality is checked rather than ignoring a nonzero ancestry check.
                if git(wt, "merge-base", ancestor, "HEAD") != git(wt, "rev-parse", ancestor):
                    raise RuntimeError("repair did not preserve the PR and main histories")
            bad = refused(kind, changed_paths(wt), manifest)
            if bad:
                return f"{repo}#{pr['number']}: repair refused ({'; '.join(bad)})"
            current = json.loads(gh("pr", "view", str(pr["number"]), "-R", repo, "--json", "state,headRefOid"))
            if current["state"] != "OPEN" or current["headRefOid"] != pr["headRefOid"]:
                return f"{repo}#{pr['number']}: changed during repair; left untouched"
            git(wt, "push", "-q", f"--force-with-lease=refs/heads/{pr['headRefName']}:{pr['headRefOid']}", "origin", f"HEAD:refs/heads/{pr['headRefName']}")
            return f"{repo}#{pr['number']}: repair pushed; the settle timer will merge only after hosted checks pass"
        finally:
            git(clone, "worktree", "remove", "--force", wt, check=False)


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
    git(wt, "push", "-q", "origin", f"HEAD:refs/heads/{branch}")
    body = ""
    try:
        with open(report, encoding="utf-8") as f:
            body = f.read()
    except OSError:
        body = "(the session wrote no report)"
    status = ("Local checks passed: " + ran) if ok else ("**Needs a maintainer.** It changes " + "; ".join(bad) if bad else "Local checks failed: " + ran + ". Hosted CI is authoritative")
    body += (f"\n\n---\n{status}.\n\nOpened by the daily improver (`scripts/daily_improve.py`, docs/DAILY_IMPROVER.md). "
             + ("It merges itself once every hosted check passes; close it to stop that." if not bad and cfg["NIGHTLY_PUSH_MAIN"] == "1" else "It will not merge itself.")
             + "\n\n🤖 Generated with [Claude Code](https://claude.com/claude-code)\n")
    args = ["pr", "create", "-R", REPOS[kind], "--base", "main", "--head", branch, "--title", report_title(report, date), "--body", body]
    if bad:
        args.append("--draft")
    url = gh(*args).strip().splitlines()[-1]
    number = int(url.rstrip("/").rsplit("/", 1)[-1])
    if bad or cfg["NIGHTLY_PUSH_MAIN"] != "1":
        return f"{REPOS[kind]}: {n} commit(s){note}; opened {url} ({'needs a maintainer' if bad else 'merging is off'})."
    lines = settle(cfg, date, only={kind: (number,)})
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
    if free_gb() < float(cfg["NIGHTLY_MIN_FREE_GB"]):
        with open(day.report, "w", encoding="utf-8") as f:
            f.write(f"# Daily improver {date}: skipped (low disk)\n")
        day.finish(f"Skipped: {free_gb():.0f} GB free (< {cfg['NIGHTLY_MIN_FREE_GB']}).")
        return 0
    if not a.dry_run and not a.here and os.path.realpath(ROOT) != os.path.realpath(NIGHTLY_WT):
        sys.exit(f"run resets the checkout it runs from to origin/main; it runs from {NIGHTLY_WT} (`install`, then the timer). "
                 f"--dry-run changes nothing; --here runs in {ROOT} anyway, throwing away its uncommitted work.")
    if a.no_merge:
        cfg["NIGHTLY_PUSH_MAIN"] = "0"
    settled = [] if a.dry_run else settle(cfg, date)
    if not a.dry_run:
        try:
            settled += repair_one(cfg, date)
        except (RuntimeError, ValueError, OSError, subprocess.TimeoutExpired) as e:
            settled.append(f"Automatic repair could not finish ({e}); existing PR preserved")
    since = last_success()
    inbox = feedback.collect(since)
    fresh = feedback.new_count(inbox)
    open_keys = sum(1 for g in inbox["backlog"] if g["status"] in ("open", "recurring", "partial"))
    if not a.force and fresh == 0 and open_keys == 0:
        with open(day.report, "w", encoding="utf-8") as f:
            f.write(f"# Daily improver {date}: nothing to do\n\nNo new feedback since {since} and no open key in the backlog.\n")
        day.finish(*settled, "Nothing to do.")
        return 0
    # A restart on the same date must not overwrite the previous run's commits or remote branch.
    branch = f"daily/{date}-{datetime.datetime.now():%H%M%S%f}"
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
    manifest = manifest_on_main(ROOT)
    results = []
    for kind, wt in (("engine", ROOT), ("games", GAMES_WT)):
        try:
            results.append(land(kind, wt, branch, day.report, date, cfg, manifest, day.log))
        except (RuntimeError, ValueError, OSError, subprocess.TimeoutExpired) as e:
            results.append(f"{REPOS[kind]}: could not land ({e}); the commits are on {branch} in {wt}.")
    if cfg["NIGHTLY_PUSH_MAIN"] == "1":
        try:
            # One shared hosted-CI wait budget for both repositories keeps the service below its outer deadline.
            results += settle(cfg, date, wait_min=float(cfg["NIGHTLY_WAIT_MIN"]))
        except (RuntimeError, ValueError, OSError, subprocess.TimeoutExpired) as e:
            results.append(f"Hosted settle deferred to the follow-up timer ({e})")
    day.finish(*settled, f"Session: {outcome['status']}.", *results)
    # Do not acknowledge feedback before the changes have been handed off successfully.
    if outcome["status"] == "completed" and not any("could not land" in line for line in results):
        with open(os.path.join(STATE, "last-success"), "w", encoding="utf-8") as f:
            f.write(datetime.datetime.now().astimezone().isoformat())
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
set -euo pipefail
export PATH="$HOME/.local/bin:$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin"
export GIT_TERMINAL_PROMPT=0 GH_PROMPT_DISABLED=1
REPO="$HOME/workspace/RedEngine"; WT="$HOME/workspace/RedEngine-nightly"
mkdir -p "$HOME/.local/state/redengine-nightly"
exec 9>>"$HOME/.local/state/redengine-nightly/run.lock"
flock -n 9 || {{ echo "another daily command is active"; exit 0; }}
export RED_NIGHTLY_LOCK_FD=9
timeout 120 git -C "$REPO" fetch -q origin
[ -d "$WT" ] || git -C "$REPO" worktree add -q --detach "$WT" origin/main
git -C "$WT" reset -q --hard
git -C "$WT" clean -qfd
git -C "$WT" checkout -q --detach origin/main
if [ "$#" -eq 0 ] || [[ "$1" == -* ]]; then set -- run "$@"; fi
exec python3 "$WT/scripts/daily_improve.py" "$@"
"""

SETTLE_SERVICE = """[Unit]
Description=Finish RedEngine nightly pull requests once hosted checks pass

[Service]
Type=oneshot
TimeoutStartSec=10m
ExecStart=%h/.local/bin/redengine-nightly settle
"""
SETTLE_TIMER = """[Unit]
Description=Recheck RedEngine nightly pull requests every 15 minutes

[Timer]
OnBootSec=5m
OnUnitInactiveSec=15m

[Install]
WantedBy=timers.target
"""


def cmd_install(a):
    path = os.path.join(HOME, ".local", "bin", "redengine-nightly")
    backup = f"{path}.bak-{datetime.date.today():%Y%m%d}"
    if os.path.exists(path) and not os.path.exists(backup):
        shutil.copy2(path, backup)
    with open(path, "w", encoding="utf-8") as f:
        f.write(SHIM.format(backup=os.path.basename(backup)))
    os.chmod(path, 0o755)
    units = os.path.join(HOME, ".config", "systemd", "user")
    for relative, text in (("redengine-nightly-settle.service", SETTLE_SERVICE), ("redengine-nightly-settle.timer", SETTLE_TIMER),
                           ("redengine-nightly.service.d/reliability.conf", "[Service]\nTimeoutStartSec=9h\n"),
                           ("redengine-nightly.timer.d/reliability.conf", "[Timer]\nPersistent=true\n")):
        dest = os.path.join(units, relative)
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        with open(dest, "w", encoding="utf-8") as f:
            f.write(text)
    subprocess.run(["systemctl", "--user", "daemon-reload"], check=True)
    subprocess.run(["systemctl", "--user", "enable", "--now", "redengine-nightly-settle.timer"], check=True)
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
    s.add_argument("--here", action="store_true", help="run in this checkout although it is not the nightly worktree (it is reset)")
    s.set_defaults(f=cmd_run)
    s = sub.add_parser("prompt")
    s.set_defaults(f=cmd_prompt)
    s = sub.add_parser("settle")
    s.add_argument("--wait-min", type=float, default=0)
    s.set_defaults(f=cmd_settle)
    s = sub.add_parser("install")
    s.set_defaults(f=cmd_install)
    a = ap.parse_args(argv)
    if a.cmd in ("run", "settle"):
        # --dry-run must neither reset worktrees nor write reports/state or acquire a lock.
        if a.cmd == "run" and a.dry_run:
            return cmd_prompt(a)
        with run_lock() as acquired:
            if not acquired:
                say("another daily command is active")
                return 0
            return a.f(a)
    return a.f(a)


if __name__ == "__main__":
    sys.exit(main())
