#!/usr/bin/env python3
"""The published games and their family tree, and forking one: take a game as it was released, make it yours, publish it as a game of its own that
remembers where it came from.

  list [--json]                                   every game on the site: slug, kind, newest version, maintainer, what it was forked from
  tree [SLUG]                                     the family tree (every game, or SLUG's ancestors and descendants) with each version
  fork SOURCE[@vN] NEW-SLUG [--name NAME] [--maintainer LOGIN] [--games-repo DIR] [--dry-run]
                                                  copy SOURCE as version N shipped it (default: the newest) into this repository as NEW-SLUG

A fork is a game like any other (docs/GAMES_PUBLISHING.md "Forking a game"): its scenes in `examples/<new>/` (3D) or `examples/2d/<new>.game2d.json`
(2D, with its generator when the original had one), its page in `pages/<new>/` and its entry in `games-publish.json` with
`"forked_from": {"slug": SOURCE, "version": N}` and `"maintainer"`: whoever forked it. The files come from RedEngineGames at the commit that version
N was released from (the release's `repo_commit`), so the fork starts from exactly what players played. Its page is the original's with the
fields that must change marked `TODO(fork)`: `publish_games.py check` refuses the fork until they are written. Then change the game, check it,
and send it in one pull request (CONTRIBUTING.md); its first release records where it came from, and the site and the app show its family.

A game that plays with its own client (`binary`: engine code) or was uploaded ready-made (no source) cannot be forked here.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "games-publish.json"
CATALOG_URL = "https://kevstermcgee.github.io/RedEngineGames/catalog.json"
GAMES_REMOTE = "https://github.com/kevstermcgee/RedEngineGames.git"
SLUG = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
LOGIN = re.compile(r"^[A-Za-z0-9](?:[A-Za-z0-9-]{0,38})$")
TODO = "TODO(fork)"
TEXT = (".json", ".md", ".txt", ".js", ".py", ".wgsl", ".csv")
# A project's notes about its own development (RedEngineGames projects/): the original's history, not the fork's.
ORIGINALS_OWN = {"CLAUDE.md", "STATUS.md", "ENGINE_FEEDBACK.md", "AGENTS.md"}


class ForkError(RuntimeError):
    """The fork cannot be made; the message says why and what to do instead."""


# ---- the catalog and the family tree ------------------------------------------------------------------------------------------------------------


def load_catalog(source: str = CATALOG_URL) -> list[dict]:
    """The site's catalog.json games (a URL or a file)."""
    if os.path.isfile(source):
        text = Path(source).read_text(encoding="utf-8")
    else:
        with urllib.request.urlopen(urllib.request.Request(source, headers={"User-Agent": "RedEngine-games"}), timeout=60) as r:
            text = r.read().decode("utf-8")
    return json.loads(text)["games"]


def parent_of(game: dict) -> dict | None:
    """{slug, version} this game was forked from, or None: the catalog's `forked_from`, else the first release's record of it."""
    f = game.get("forked_from")
    if not f:
        first = (game.get("versions") or [{}])[-1]
        f = first.get("forked_from")
    return f if isinstance(f, dict) and f.get("slug") else None


def children(games: list[dict]) -> dict[str, list[dict]]:
    out: dict[str, list[dict]] = {}
    for g in games:
        p = parent_of(g)
        if p:
            out.setdefault(p["slug"], []).append(g)
    return out


def tree_lines(games: list[dict], slug: str | None = None) -> list[str]:
    by_slug = {g["slug"]: g for g in games}
    kids = children(games)

    def label(g):
        vs = g.get("versions") or []
        newest = f"v{vs[0]['version']}" if vs else "unreleased"
        who = f" by @{g['maintainer']}" if g.get("maintainer") else ""
        p = parent_of(g)
        from_ = f" (from {p['slug']} v{p.get('version', '?')})" if p else ""
        return f"{g['name']} `{g['slug']}` {newest}{who}{from_}"

    def walk(g, depth, out, seen):
        if g["slug"] in seen:
            return
        seen.add(g["slug"])
        out.append("  " * depth + ("└ " if depth else "") + label(g))
        for c in sorted(kids.get(g["slug"], []), key=lambda c: c["slug"]):
            walk(c, depth + 1, out, seen)

    out: list[str] = []
    if slug:
        if slug not in by_slug:
            raise ForkError(f"no game `{slug}` on the site (`games.py list`)")
        chain, g = [], by_slug[slug]
        while parent_of(g) and parent_of(g)["slug"] in by_slug and len(chain) < 50:
            g = by_slug[parent_of(g)["slug"]]
            chain.append(g)
        walk(chain[-1] if chain else by_slug[slug], 0, out, set())
        return out
    roots = [g for g in games if not parent_of(g) or parent_of(g)["slug"] not in by_slug]
    for g in sorted(roots, key=lambda g: g["slug"]):
        if g["slug"] in kids or parent_of(g):
            walk(g, 0, out, set())
    alone = sum(1 for g in roots if g["slug"] not in kids and not parent_of(g))
    out.append(f"({alone} more game(s) with no forks yet: `games.py list`)")
    return out


# ---- RedEngineGames at a commit ------------------------------------------------------------------------------------------------------------------


def git(repo: Path, *args: str, check: bool = True) -> str:
    p = subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True, encoding="utf-8", errors="replace")
    if check and p.returncode:
        raise ForkError(f"git {' '.join(args[:2])}: {(p.stderr or p.stdout).strip()}")
    return p.stdout


def git_bytes(repo: Path, spec: str) -> bytes:
    p = subprocess.run(["git", "-C", str(repo), "show", spec], capture_output=True)
    if p.returncode:
        raise ForkError(f"git show {spec}: {p.stderr.decode(errors='replace').strip()}")
    return p.stdout


def has_commit(repo: Path, commit: str) -> bool:
    return bool(git(repo, "rev-parse", "--verify", "-q", f"{commit}^{{commit}}", check=False).strip())


def games_repo_at(commit: str, given: str | None) -> Path:
    """A RedEngineGames clone that has `commit`: DIR, ../RedEngineGames, or a partial clone in a temporary folder (blobs fetched as needed)."""
    c = Path(given) if given else ROOT.parent / "RedEngineGames"
    if (c / ".git").exists():
        if not has_commit(c, commit):
            git(c, "fetch", "-q", "origin", commit, check=False)
        if has_commit(c, commit):
            return c
    elif given:
        raise ForkError(f"--games-repo {given} is not a git checkout")
    d = Path(tempfile.mkdtemp(prefix="redenginegames-"))
    git(d, "init", "-q")
    git(d, "remote", "add", "origin", GAMES_REMOTE)
    git(d, "fetch", "-q", "--filter=blob:none", "--depth=1", "origin", commit)
    return d


def playable_at(repo: Path, commit: str, slug: str) -> dict:
    """The playable as RedEngineGames held it at `commit`: RedEngine's (its catalog) or the repository's own data-driven one."""
    for name, key in ((".games-catalog.json", "playables"), (".release-games.json", "data_playables")):
        try:
            data = json.loads(git(repo, "show", f"{commit}:{name}"))
        except (ForkError, ValueError):
            continue
        for p in data.get(key, []):
            if p.get("slug") == slug:
                return p
    raise ForkError(f"`{slug}` is not a playable in RedEngineGames at {commit[:12]}")


def files_at(repo: Path, commit: str, paths: list[str]) -> dict[str, bytes]:
    out = {}
    for path in paths:
        for f in git(repo, "ls-tree", "-r", "--name-only", commit, "--", path).splitlines():
            out[f] = git_bytes(repo, f"{commit}:{f}")
    if not out:
        raise ForkError(f"nothing at {', '.join(paths)} in RedEngineGames {commit[:12]}")
    return out


def page_at(repo: Path, commit: str, slug: str) -> dict[str, bytes]:
    """The game's page (wiki.json, its pictures, the thumbnail) at `commit`, or as the site has it now when it was written after that release."""
    paths = [f"site/wiki/{slug}", f"site/thumbs/{slug}.png"]
    try:
        return files_at(repo, commit, paths)
    except ForkError:
        git(repo, "fetch", "-q", "--filter=blob:none", "--depth=1", "origin", "main", check=False)
        return files_at(repo, "FETCH_HEAD", paths)


# ---- the fork ------------------------------------------------------------------------------------------------------------------------------------


def pick_version(game: dict, wanted: int | None) -> dict:
    versions = game.get("versions") or []
    if not versions:
        raise ForkError(f"`{game['slug']}` has no released version to fork")
    if wanted is None:
        return versions[0]
    for v in versions:
        if v.get("version") == wanted:
            return v
    raise ForkError(f"`{game['slug']}` has no v{wanted} (it has {', '.join('v' + str(v['version']) for v in versions)})")


def rename_text(data: bytes, old_dir: str, new_dir: str, old_slug: str, new_slug: str, name: str) -> bytes:
    """Paths into the old game's folder become paths into the new one; a 2D game's file name too."""
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        return data
    text = text.replace(f"{old_dir}/", f"{new_dir}/").replace(f"{old_slug}.game2d.json", f"{new_slug}.game2d.json")
    return text.encode("utf-8")


def entry_of(playable: dict) -> str:
    """The game's main file: its `entry`, or (RedEngineGames' own playables have none) the content file its launch arguments open."""
    if playable.get("entry"):
        return playable["entry"]
    opened = [a[len("content/"):] for a in playable.get("arguments", []) if a.startswith("content/")]
    if not opened:
        raise ForkError(f"`{playable.get('slug')}` names no entry and opens no content file")
    return opened[-1]


def requote(text: str, renames: list[tuple[str, str]], quotes: tuple[str, ...]) -> str:
    """`text` with every exactly quoted OLD (a whole string literal, in any of `quotes`) replaced by NEW: a name used inside a longer string is kept."""
    for old, new in renames:
        for q in quotes:
            text = text.replace(f"{q}{old}{q}", f"{q}{new}{q}")
    return text


def plan_fork(source_game: dict, version: dict, playable: dict, files: dict[str, bytes], page: dict[str, bytes], new_slug: str, name: str,
              maintainer: str, generator: dict[str, bytes] | None = None) -> tuple[dict[str, bytes], dict, dict]:
    """(files to write {repo path: bytes}, the `games` collection entry, the playable) for the fork. Pure: tested without git or network."""
    old = source_game["slug"]
    kind = playable.get("kind", "3d")
    if playable.get("binary"):
        raise ForkError(f"`{old}` plays with its own client (`binary`: {playable['binary']}), which is engine code: copy games/{playable['binary']} in a "
                        "pull request the other maintainer reviews, not with this tool")
    if len(playable.get("files", [])) != 1:
        raise ForkError(f"`{old}` ships {len(playable.get('files', []))} folders ({playable.get('files')}); a fork copies one")
    old_dir = playable["files"][0].rstrip("/")
    new_dir = f"games/{new_slug}"
    entry_rel = entry_of(playable)[len(old_dir) + 1:]
    out: dict[str, bytes] = {}
    if kind == "2d":
        scenes = [p for p in files if p.endswith(".game2d.json")]
        if len(scenes) != 1 or len(files) != 1:
            raise ForkError(f"`{old}` is a 2D game of {len(files)} file(s); a 2D fork is one `*.game2d.json`")
        text = files[scenes[0]].decode("utf-8")
        game = json.loads(text)
        # The same exact-string renames in the game and in its generator (the game is the generator's output): the id (saves are filed under it, so
        # the fork keeps its own) and the title. A generator that wrote the original byte for byte then writes the fork byte for byte.
        renames = [(v, n) for v, n in ((game.get("id"), new_slug), (game.get("title"), name)) if isinstance(v, str) and v and v != n]
        text = requote(text, renames, ('"',))
        if json.loads(text).get("id", new_slug) != new_slug:
            raise ForkError("internal: the fork's id did not change")
        source = f"examples/2d/{new_slug}.game2d.json"
        out[source] = text.encode("utf-8")
        entry = f"{new_dir}/{new_slug}.game2d.json"
        stem = Path(scenes[0]).name[: -len(".game2d.json")]
        for path, data in (generator or {}).items():
            rel = path.split(f"examples/2d/{stem}/", 1)[-1]
            if path.endswith(TEXT):
                gen = rename_text(data, old_dir, new_dir, stem, new_slug, name).decode("utf-8").replace(f"examples/2d/{stem}/", f"examples/2d/{new_slug}/")
                data = requote(gen, renames, ('"', "'")).encode("utf-8")
            out[f"examples/2d/{new_slug}/{rel}"] = data
    else:
        source = f"examples/{new_slug}"
        for path, data in files.items():
            rel = path[len(old_dir) + 1:]
            if rel in ORIGINALS_OWN or rel.startswith("scripts/"):   # scripts/red*: shims that ran a sibling engine checkout; the fork lives in the engine
                continue
            if rel == "game.json":
                project = json.loads(data.decode("utf-8"))
                project["id"], project["name"] = new_slug, new_slug   # saves are filed under the id: the fork keeps its own
                if isinstance(project.get("engine"), dict) and "path" in project["engine"]:
                    project["engine"]["path"] = "../.."               # examples/<new>/ inside the engine's own checkout
                data = (json.dumps(project, indent=2, ensure_ascii=False) + "\n").encode("utf-8")
            data = rename_text(data, old_dir, new_dir, old, new_slug, name) if path.endswith(TEXT) else data
            if rel == "README.md":   # its first paragraph is the game's description on the site
                data = f"{TODO}: what {name} is, in one paragraph (forked from {source_game['name']} v{version['version']}).\n\n".encode("utf-8") + data
            out[f"{source}/{rel}"] = data
        entry = f"{new_dir}/{entry_rel}"
    args = []
    for a in playable.get("arguments", []):
        if a.startswith(f"content/{old_dir}/"):
            a = f"content/{entry}" if kind == "2d" else f"content/{new_dir}/{a[len('content/' + old_dir) + 1:]}"
        args.append(a)
    out.update(fork_page(page, old, source_game["name"], version["version"], new_slug, name))
    collection = {"source": source, "destination": new_slug}
    new_playable = {"slug": new_slug, "name": name}
    if kind == "2d":
        new_playable["kind"] = "2d"
    new_playable.update({"entry": entry, "files": [new_dir], "arguments": args, "page": f"pages/{new_slug}", "maintainer": maintainer,
                         "forked_from": {"slug": old, "version": version["version"]}})
    return out, collection, new_playable


def fork_page(page: dict[str, bytes], old: str, old_name: str, version: int, new: str, name: str) -> dict[str, bytes]:
    """The original's page as the fork's starting point: pictures and fields kept, the ones that must change marked TODO(fork)."""
    out = {}
    wiki = None
    for path, data in page.items():
        base = Path(path).name
        if base == "wiki.json":
            wiki = json.loads(data.decode("utf-8"))
        elif path.endswith(f"thumbs/{old}.png") or base == "thumb.png":
            out[f"pages/{new}/thumb.png"] = data
        else:
            out[f"pages/{new}/{base}"] = data
    if wiki is None:
        raise ForkError(f"`{old}` has no page to start the fork's page from")
    wiki["tagline"] = f"{TODO}: one sentence on what {name} is. (Forked from {old_name} v{version}: {wiki.get('tagline', '')})"
    wiki["overview"] = f"{TODO}: what you changed and why it plays differently. {old_name} v{version}, as forked: " + str(wiki.get("overview", ""))
    for k in ("why_made", "why_made_note", "prompt_unavailable", "prompt_source"):
        wiki.pop(k, None)
    wiki["prompt"] = f"{TODO}: the prompt(s) you gave to make {name} from {old_name} v{version}, exactly as written."
    out[f"pages/{new}/wiki.json"] = (json.dumps(wiki, indent=1, ensure_ascii=False) + "\n").encode("utf-8")
    return out


def add_to_manifest(text: str, collection: dict, playable: dict) -> str:
    """games-publish.json with the fork's `games` entry and playable appended, edited as text so the diff is only the new lines (the file is
    hand-formatted). The result must parse to exactly the old manifest plus the two entries."""
    before = json.loads(text)
    games_end = re.search(r'("games": \[\n(?:      \{[^\n]*\},?\n)+)(    \])', text)
    if not games_end:
        raise ForkError("games-publish.json has an unexpected shape: no one-line-per-entry `games` collection")
    block = games_end.group(1)
    line = "      " + json.dumps(collection, ensure_ascii=False)
    text = text[: games_end.start(1)] + block.rstrip("\n").rstrip(",") + ",\n" + line + "\n" + text[games_end.end(1):]
    tail = "\n    }\n  ]\n}"
    if not text.rstrip().endswith(tail.strip()) or not text.endswith("\n"):
        raise ForkError("games-publish.json has an unexpected shape: it must end with the `playables` list")
    fields = []
    for k, v in playable.items():
        fields.append(f'      "{k}": ' + (json.dumps(v, ensure_ascii=False, separators=(", ", ": ")) if k != "forked_from" else json.dumps(v, ensure_ascii=False)))
    entry = "    {\n" + ",\n".join(fields) + "\n    }"
    cut = text.rstrip().rfind("\n  ]")
    text = text[:cut] + ",\n" + entry + text[cut:]
    after = json.loads(text)
    expect = json.loads(json.dumps(before))
    expect["collections"]["games"].append(collection)
    expect["playables"].append(playable)
    if after != expect:
        raise ForkError("internal: editing games-publish.json changed more than the fork's two entries")
    return text


def generator_of(source_game: dict, version: dict, playable: dict) -> dict[str, bytes]:
    """A 2D game's generator as RedEngine held it when the version was released (`examples/2d/<name>/build.js|build.py`), or {}."""
    rev = version.get("engine_revision")
    if not rev or playable.get("kind") != "2d":
        return {}
    try:
        manifest = json.loads(git(ROOT, "show", f"{rev}:games-publish.json"))
    except (ForkError, ValueError):
        return {}
    src = next((e["source"] for e in manifest.get("collections", {}).get("games", []) if e.get("destination") == source_game["slug"]), "")
    if not src.endswith(".game2d.json"):
        return {}
    folder = src[: -len(".game2d.json")]
    names = git(ROOT, "ls-tree", "-r", "--name-only", rev, "--", folder + "/", check=False).splitlines()
    if not any(Path(f).name in ("build.js", "build.py") for f in names):
        return {}
    return {f: git_bytes(ROOT, f"{rev}:{f}") for f in names}   # the generator and what it imports beside it


def whoami() -> str:
    try:
        out = subprocess.run(["gh", "api", "user", "--jq", ".login"], capture_output=True, text=True, timeout=20).stdout.strip()
    except (OSError, subprocess.TimeoutExpired):
        out = ""
    return out if LOGIN.match(out) else ""


def cmd_fork(a) -> int:
    source, _, ver = a.source.partition("@")
    wanted = int(ver.lstrip("v")) if ver else None
    if not SLUG.match(a.new_slug):
        raise ForkError(f"`{a.new_slug}`: a slug is lowercase words joined by hyphens, like my-snake")
    games = load_catalog(a.catalog)
    by_slug = {g["slug"]: g for g in games}
    if source not in by_slug:
        raise ForkError(f"no game `{source}` on the site (`games.py list`)")
    manifest_text = MANIFEST.read_text(encoding="utf-8")
    manifest = json.loads(manifest_text)
    taken = set(by_slug) | {p["slug"] for p in manifest["playables"]} | {e["destination"] for e in manifest["collections"]["games"]}
    if a.new_slug in taken or (ROOT / "pages" / a.new_slug).exists() or (ROOT / "examples" / a.new_slug).exists():
        raise ForkError(f"`{a.new_slug}` is taken: pick another slug")
    g = by_slug[source]
    if g.get("uploaded"):
        raise ForkError(f"`{source}` was uploaded ready-made: there is no source to fork (fork it in the app instead)")
    maintainer = a.maintainer or whoami()
    if not LOGIN.match(maintainer or ""):
        raise ForkError("say whose fork it is: --maintainer <your GitHub login> (gh is not signed in here)")
    version = pick_version(g, wanted)
    commit = version.get("repo_commit")
    if not commit:
        raise ForkError(f"`{source}` v{version['version']} does not record the commit it was released from")
    repo = games_repo_at(commit, a.games_repo)
    playable = playable_at(repo, commit, source)
    if playable.get("binary"):
        plan_fork(g, version, playable, {}, {}, a.new_slug, a.new_slug, maintainer)   # raises: the reason, in one place
    files = files_at(repo, commit, playable["files"])
    page = page_at(repo, commit, source)
    name = a.name or a.new_slug.replace("-", " ").title()
    out, collection, new_playable = plan_fork(g, version, playable, files, page, a.new_slug, name, maintainer, generator_of(g, version, playable))
    print(f"fork {g['name']} v{version['version']} (RedEngineGames {commit[:12]}) -> {name} `{a.new_slug}`, maintained by @{maintainer}")
    for path in sorted(out):
        print(f"  {'would write' if a.dry_run else 'write'} {path}")
    print("  games-publish.json: a `games` entry and the playable, with \"forked_from\": " + json.dumps(new_playable["forked_from"]))
    if a.dry_run:
        return 0
    for path, data in out.items():
        target = ROOT / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
    MANIFEST.write_text(add_to_manifest(manifest_text, collection, new_playable), encoding="utf-8", newline="\n")
    gen = [p for p in out if p.startswith(f"examples/2d/{a.new_slug}/")]
    print(f"""
Next (CONTRIBUTING.md):
  1. make it yours: change {collection['source']}{' through its generator (' + ', '.join(gen) + ': run it, then `python3 scripts/game_generators.py check`)' if gen else ''}
  2. check it: scripts/dev red validate {collection['source']} && scripts/dev red verify {collection['source']}  (Windows without the CLI: CI runs them)
  3. write the page: every {TODO} in pages/{a.new_slug}/wiki.json, and new pictures of what changed (`publish_games.py check` refuses the TODOs)
  4. commit, push, open one pull request; when CI passes, the merge publishes {a.new_slug}-v1, and the site and the app show it under {g['name']}""")
    return 0


def cmd_list(a) -> int:
    games = load_catalog(a.catalog)
    if a.json:
        print(json.dumps([{"slug": g["slug"], "name": g["name"], "kind": g.get("presentation", "3d"), "version": (g.get("versions") or [{}])[0].get("version"),
                           "maintainer": g.get("maintainer", ""), "forked_from": parent_of(g), "uploaded": bool(g.get("uploaded"))} for g in games], indent=1))
        return 0
    for g in sorted(games, key=lambda g: g["slug"]):
        vs = g.get("versions") or []
        p = parent_of(g)
        print(f"{g['slug']:28} {g.get('presentation', '3d'):3} {('v' + str(vs[0]['version'])) if vs else '-':5} "
              f"{('@' + g['maintainer']) if g.get('maintainer') else '':16} {('from ' + p['slug'] + ' v' + str(p.get('version'))) if p else ''}"
              f"{' (uploaded)' if g.get('uploaded') else ''}")
    return 0


def cmd_tree(a) -> int:
    print("\n".join(tree_lines(load_catalog(a.catalog), a.slug)))
    return 0


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--catalog", default=CATALOG_URL, help="the site's catalog.json (a URL or a file)")
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("list")
    s.add_argument("--json", action="store_true")
    s.set_defaults(f=cmd_list)
    s = sub.add_parser("tree")
    s.add_argument("slug", nargs="?")
    s.set_defaults(f=cmd_tree)
    s = sub.add_parser("fork")
    s.add_argument("source", help="SLUG or SLUG@vN (default: the newest version)")
    s.add_argument("new_slug")
    s.add_argument("--name", help="the fork's name on the site (default: from the slug)")
    s.add_argument("--maintainer", help="your GitHub login (default: gh's)")
    s.add_argument("--games-repo", help="a RedEngineGames clone (default ../RedEngineGames, else a temporary partial clone)")
    s.add_argument("--dry-run", action="store_true")
    s.set_defaults(f=cmd_fork)
    a = ap.parse_args(argv)
    try:
        return a.f(a)
    except ForkError as e:
        print(f"games.py {a.cmd}: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
