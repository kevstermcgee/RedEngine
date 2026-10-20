#!/usr/bin/env python3
"""A contributor's game, checked for a one-tap merge: when a pull request only changes games its author maintains and CI passed, the owner is told it is
safe to merge (a comment that mentions them, so their phone shows it) and merges it with one tap; the publish then runs as for any merge.

  python3 scripts/contributed_games.py check PR                       what the checker says about a pull request (read only; needs `gh`)
  python3 scripts/contributed_games.py review PR                      the same, written as one comment on the pull request (what the workflow runs)
  python3 scripts/contributed_games.py review --head OWNER:BRANCH     the open pull request from that branch (a fork's pull request, after its CI run)

`.github/workflows/contributed-games.yml` runs `review` when CI passes on a pull request. It reads the pull request through the API, as data, and never runs
its code; the rules below come from `main`. It never merges. A pull request is safe to merge when ALL of these hold:

- its author is listed in `.github/contributors.json` (only the owner edits it);
- CI passed on its newest commit, it is open, not a draft, targets `main` and merges without conflicts;
- every file it changes is content (JSON, Markdown, PNG, JPEG; at most 5 MB, no symlinks) inside a game its author maintains (the game's published source,
  a 2D game's folder beside it and `pages/<slug>/`), or `games-publish.json`; a JavaScript generator of the author's own (`build.js`) counts too, since
  nothing here runs it (a Python one does not: CI runs those);
- its change to `games-publish.json` touches only that author's games: a game is theirs when its playable says `"maintainer": "<their GitHub login>"` and no
  one else maintains it on `main`. A new game is theirs from the pull request that adds it, if its slug, source and page are new everywhere (here and in
  RedEngineGames). Their playable stays a plain one: played by `re2` (3D, its source `examples/<name>` or `examples/<name>.json`) or `re2d` (2D,
  `examples/2d/<name>.game2d.json`), no `binary` (a game's own client is engine code), files `games/<slug>`, launch arguments only `--host`/`--no-host` and
  its own files, and a `page` (`pages/<slug>`: the site refuses a released game without one).

Otherwise the comment says what needs the owner's eyes, and why. One comment per commit. ADR 2026-10-20-a-game-and-its-page-are-sent-in-one-pull-request.
"""
from __future__ import annotations

import base64
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTRIBUTORS = ".github/contributors.json"
MANIFEST = "games-publish.json"
CONTENT = (".json", ".md", ".png", ".jpg", ".jpeg")
# A game's own generator in JavaScript (`examples/2d/<name>/build.js`): its author runs it; nothing here does (CI and scripts/game_generators.py run Python
# generators, so a `.py` file always needs the owner's eyes).
AUTHOR_TOOLS = (".js",)
MAX_BYTES = 5 * 1024 * 1024
FLAGS = {"--host", "--no-host"}
PLAYABLE_KEYS = {"slug", "name", "kind", "entry", "files", "arguments", "page", "maintainer", "forked_from"}
SOURCE_3D = re.compile(r"examples/[a-z0-9][a-z0-9_-]*(\.json)?")
SOURCE_2D = re.compile(r"examples/2d/[a-z0-9][a-z0-9_-]*\.game2d\.json")
NOT_A_GAME_FOLDER = {"examples/2d", "examples/3d", "examples/external"}
MARK = "<!-- contributed-games:"


def by_slug(manifest: dict) -> dict:
    return {p.get("slug"): p for p in manifest.get("playables", []) if isinstance(p, dict)}


def games_entries(manifest: dict) -> list:
    return list(manifest.get("collections", {}).get("games", []))


def under(path: str, root: str) -> bool:
    return path == root or path.startswith(root.rstrip("/") + "/")


def verdict(author: str, contributors: list, main: dict, base: dict, head: dict, changed: list, tree: dict, taken: set, exists_on_main) -> tuple[list[str], list[str]]:
    """(problems, the games of the author it changes). No problems means it is safe to merge.

    main / base / head: games-publish.json on `main`, at the pull request's merge base and at its head. changed: [(status, path, previous path or None)].
    tree: {path: (mode, size)} at the head. taken: slugs RedEngineGames already uses. exists_on_main(path): whether `main` has that file or folder."""
    if author not in contributors:
        return [f"@{author} is not in {CONTRIBUTORS}"], []
    problems: list[str] = []
    on_main, at_base, at_head = by_slug(main), by_slug(base), by_slug(head)

    # Whose games: theirs on main and at the merge base, or new everywhere.
    owned = []
    for slug, p in at_head.items():
        if p.get("maintainer") != author:
            continue
        others = [m for m in (on_main.get(slug), at_base.get(slug)) if m is not None and m.get("maintainer") != author]
        if others:
            problems.append(f"`{slug}` is maintained by {others[0].get('maintainer') or 'the owner'}, so only they can change it")
        else:
            owned.append(slug)
    for slug, p in {**at_base, **on_main}.items():
        if p.get("maintainer") == author and slug not in at_head:
            problems.append(f"removing `{slug}` from the site needs the owner")
    new = [s for s in owned if s not in on_main and s not in at_base]

    # games-publish.json: nothing but their games changes.
    def rest(m):
        return {k: (v if k != "collections" else {c: e for c, e in v.items() if c != "games"}) for k, v in m.items() if k != "playables"}

    if rest(base) != rest(head):
        problems.append(f"{MANIFEST}: changes more than your games' entries (only `collections.games` and `playables` may change)")
    if [p for p in base.get("playables", []) if p.get("slug") not in owned] != [p for p in head.get("playables", []) if p.get("slug") not in owned]:
        problems.append(f"{MANIFEST}: changes a playable you do not maintain (yours say \"maintainer\": \"{author}\")")
    if [e for e in games_entries(base) if e.get("destination") not in owned] != [e for e in games_entries(head) if e.get("destination") not in owned]:
        problems.append(f"{MANIFEST}: changes a `games` entry of a game you do not maintain")

    # Each of their games stays a plain one.
    paths, roots = [], {}
    for slug in owned:
        p = at_head[slug]
        kind = p.get("kind", "3d")
        entries = [e for e in games_entries(head) if e.get("destination") == slug]
        source = entries[0].get("source", "") if len(entries) == 1 else ""
        if len(entries) != 1:
            problems.append(f"`{slug}`: needs exactly one `games` entry with \"destination\": \"{slug}\"")
        elif not ((SOURCE_2D if kind == "2d" else SOURCE_3D).fullmatch(source) and source not in NOT_A_GAME_FOLDER):
            problems.append(f"`{slug}`: its source `{source}` must be " + ("`examples/2d/<name>.game2d.json`" if kind == "2d" else "`examples/<name>` or `examples/<name>.json`"))
        old = [e.get("source") for e in games_entries(base) + games_entries(main) if e.get("destination") == slug]
        if slug in new and source and exists_on_main(source):
            problems.append(f"`{slug}`: its source `{source}` already exists on main: a new game needs a new file or folder")
        elif old and any(o != source for o in old):
            problems.append(f"`{slug}`: moving its source needs the owner")
        if slug in new and (slug in taken or exists_on_main(f"pages/{slug}")):
            problems.append(f"`{slug}`: the name is already used (here or on RedEngineGames): pick another slug")
        extra = sorted(set(p) - PLAYABLE_KEYS)
        if extra:
            problems.append(f"`{slug}`: " + ("a game with its own client (`binary`) is engine code: the owner reviews it" if "binary" in extra else f"unexpected keys {extra}"))
        if kind not in ("2d", "3d"):
            problems.append(f"`{slug}`: kind must be \"3d\" or \"2d\"")
        if p.get("files") != [f"games/{slug}"] or not str(p.get("entry", "")).startswith(f"games/{slug}/"):
            problems.append(f"`{slug}`: needs \"files\": [\"games/{slug}\"] and an entry under games/{slug}/")
        for arg in p.get("arguments", []):
            if not (arg in FLAGS or (isinstance(arg, str) and arg.startswith(f"content/games/{slug}/") and ".." not in arg)):
                problems.append(f"`{slug}`: launch argument `{arg}` is not allowed (only --host, --no-host and the game's own files)")
        if p.get("page") != f"pages/{slug}":
            problems.append(f"`{slug}`: needs its page, \"page\": \"pages/{slug}\" (CONTRIBUTING.md)")
        folder = source[: -len(".game2d.json")] if kind == "2d" and source.endswith(".game2d.json") else ""   # a 2D game's generator folder beside it
        roots[slug] = [x for x in (source, *old, folder, f"pages/{slug}") if x]
        paths += roots[slug]

    # The games it changes: their entries in the manifest, or files inside them.
    entry = lambda m, s: ([e for e in games_entries(m) if e.get("destination") == s], by_slug(m).get(s))
    touched = [s for s in owned if entry(base, s) != entry(head, s) or any(under(q, r) for _, path, prev in changed for q in (path, prev) if q for r in roots[s])]

    # Every changed file is content inside their games.
    if not changed:
        problems.append("the pull request changes nothing")
    for status, path, previous in changed:
        for q in (path, previous):
            if not q or q == MANIFEST:
                continue
            if not any(under(q, root) for root in paths):
                problems.append(f"`{q}` is outside the games you maintain")
            elif not (q.lower().endswith(CONTENT) or (q.lower().endswith(AUTHOR_TOOLS) and not q.startswith("pages/"))):
                problems.append(f"`{q}`: only JSON, Markdown, PNG and JPEG files (and a JavaScript generator of your own) are safe without a look")
        if status != "removed" and path in tree:
            mode, size = tree[path]
            if mode == "120000":
                problems.append(f"`{path}` is a symlink")
            elif size > MAX_BYTES:
                problems.append(f"`{path}` is {size / 1048576:.1f} MB (at most {MAX_BYTES // 1048576} MB)")
    return list(dict.fromkeys(problems)), touched


# ---- GitHub (read only, apart from the one comment) ---------------------------------------------------------------------------------------------------


def gh(*args: str, check: bool = True) -> str:
    r = subprocess.run(["gh", *args], capture_output=True, text=True)
    if check and r.returncode:
        raise SystemExit(f"gh {' '.join(args[:3])}: {(r.stderr or r.stdout).strip()}")
    return r.stdout


def api(path: str):
    return json.loads(gh("api", path) or "null")


def paged(path: str) -> list:
    return [json.loads(line) for line in gh("api", "--paginate", path, "--jq", ".[]|@json").splitlines()]


def manifest_at(repo: str, ref: str) -> dict:
    return json.loads(base64.b64decode(api(f"repos/{repo}/contents/{MANIFEST}?ref={ref}")["content"]))


def contributors() -> list:
    return json.loads((ROOT / CONTRIBUTORS).read_text(encoding="utf-8")).get("contributors", [])


def games_repo_taken(games_repo: str) -> set:
    """Every slug RedEngineGames uses: its catalog's playables, its own data-driven games, and every folder under games/, projects/ and site/wiki/."""
    taken = set()
    for folder in ("games", "projects", "site/wiki"):
        r = gh("api", f"repos/{games_repo}/contents/{folder}", check=False)
        if r.strip().startswith("["):
            taken |= {e["name"] for e in json.loads(r) if e.get("type") == "dir"}
    for name, key in ((".games-catalog.json", "playables"), (".release-games.json", "data_playables")):
        r = gh("api", f"repos/{games_repo}/contents/{name}", check=False)
        if r.strip().startswith("{"):
            taken |= {p.get("slug") for p in json.loads(base64.b64decode(json.loads(r)["content"])).get(key, [])}
    return taken


def ci_passed(repo: str, sha: str) -> tuple[bool, str]:
    """Whether the newest CI run (.github/workflows/ci.yml, on the pull request) for this exact commit succeeded."""
    runs = api(f"repos/{repo}/actions/runs?head_sha={sha}&event=pull_request&per_page=50").get("workflow_runs", [])
    runs = sorted((r for r in runs if r.get("path") == ".github/workflows/ci.yml"), key=lambda r: r["run_number"], reverse=True)
    if not runs:
        return False, "CI has not run on its newest commit"
    if runs[0]["status"] != "completed":
        return False, "CI is still running"
    if runs[0]["conclusion"] != "success":
        return False, f"CI {runs[0]['conclusion']}: {runs[0]['html_url']}"
    return True, ""


def evaluate(repo: str, number: int) -> tuple[dict, list[str], list[str], list[str]]:
    """(the pull request, why it is not ready yet (CI, state), why it needs the owner's eyes (the rules), the author's games it changes)."""
    pr = api(f"repos/{repo}/pulls/{number}")
    author, sha = pr["user"]["login"], pr["head"]["sha"]
    if author not in contributors():
        return pr, [], [f"@{author} is not in {CONTRIBUTORS}"], []
    waiting = []
    if pr["state"] != "open" or pr.get("draft"):
        waiting.append("it is not an open, ready pull request")
    if pr["base"]["ref"] != "main":
        waiting.append("it does not target main")
    if pr.get("mergeable") is False:
        waiting.append("it has merge conflicts with main: update the branch")
    ok, why = ci_passed(repo, sha)
    if not ok:
        waiting.append(why)
    head_repo = pr["head"]["repo"]["full_name"] if pr["head"].get("repo") else repo
    merge_base = api(f"repos/{repo}/compare/{pr['base']['sha']}...{sha}")["merge_base_commit"]["sha"]
    changed = [(f["status"], f["filename"], f.get("previous_filename")) for f in paged(f"repos/{repo}/pulls/{number}/files?per_page=100")]
    tree = {e["path"]: (e["mode"], e.get("size", 0)) for e in api(f"repos/{head_repo}/git/trees/{sha}?recursive=1").get("tree", []) if e["type"] == "blob"}
    main = json.loads((ROOT / MANIFEST).read_text(encoding="utf-8"))
    rules, owned = verdict(author, contributors(), main, manifest_at(repo, merge_base), manifest_at(head_repo, sha), changed, tree,
                           games_repo_taken(main["target_repository"]), lambda p: (ROOT / p).exists())
    return pr, waiting, rules, owned


def message(pr: dict, owner: str, waiting: list[str], rules: list[str], owned: list[str]) -> str:
    sha = pr["head"]["sha"]
    games = ", ".join(f"`{s}`" for s in owned) or "no game"
    if not waiting and not rules:
        return "\n".join([
            f"{MARK}{sha} -->",
            f"@{owner} **Safe to merge** ({games}): it only changes games @{pr['user']['login']} maintains, every file is content, the site page is there, "
            f"and CI passed on {sha[:7]}. Merging publishes it to RedEngineGames by itself (the site and the app list it about 15 minutes later).",
        ])
    return "\n".join([f"{MARK}{sha} -->", f"@{owner} **Needs your eyes** ({games}) before merging:", "", *[f"- {p}" for p in waiting + rules], "",
                      "A pull request that only changes games its author maintains is marked safe to merge once CI passes: see CONTRIBUTING.md."])


def review(repo: str, number: int) -> int:
    pr, waiting, rules, owned = evaluate(repo, number)
    if pr["user"]["login"] not in contributors():
        print(f"#{number}: not a listed contributor's pull request")
        return 0
    if pr["state"] != "open" or any(w in ("CI is still running", "CI has not run on its newest commit") for w in waiting):
        print(f"#{number}: not ready to review yet: {'; '.join(waiting)}")
        return 0
    mark = f"{MARK}{pr['head']['sha']} -->"
    if any(mark in (c.get("body") or "") for c in paged(f"repos/{repo}/issues/{number}/comments?per_page=100")):
        print(f"#{number}: this commit is reviewed already")
        return 0
    owner = repo.split("/")[0]
    body = message(pr, owner, waiting, rules, owned)
    gh("api", f"repos/{repo}/issues/{number}/comments", "-f", f"body={body}")
    print(body)
    return 0


def main() -> int:
    args = sys.argv[1:]
    repo = os.environ.get("GITHUB_REPOSITORY") or json.loads(gh("repo", "view", "--json", "nameWithOwner"))["nameWithOwner"]
    if len(args) == 3 and args[:2] == ["review", "--head"]:
        found = api(f"repos/{repo}/pulls?state=open&head={args[2]}")
        if not found:
            print(f"no open pull request from {args[2]}")
            return 0
        return review(repo, found[0]["number"])
    if len(args) == 2 and args[0] in ("check", "review") and args[1].isdigit():
        if args[0] == "review":
            return review(repo, int(args[1]))
        pr, waiting, rules, owned = evaluate(repo, int(args[1]))
        state = "safe to merge" if not (waiting or rules) else ("needs the owner's eyes" if rules else "not ready yet")
        print(f"#{args[1]} by @{pr['user']['login']}: {state}" + (f" (games: {', '.join(owned)})" if owned else ""))
        for p in waiting + rules:
            print(f"  - {p}")
        return 0 if not (waiting or rules) else 1
    print(__doc__)
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
