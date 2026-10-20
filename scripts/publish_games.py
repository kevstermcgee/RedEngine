#!/usr/bin/env python3
"""Export curated engine-made content into the companion Games repository.

  check [--games-repo DIR]   export into a throwaway folder: the manifest is sound and every page meets DIR's page rules (DIR/site/wiki.py)
  export --output DIR        write what the manifest publishes into the Games checkout DIR (and remove what an earlier export published and this one does not)
  notes --since REV          "<slug>: <commit subject>" for every playable whose published sources changed since REV (the sync commit's body: the release's "What changed")
  play-check [--engine CMD]  every published game the engine's client plays still plays: `lint` finds no errors, the bots' `nav` graph can be travelled, the scene's own `checks` pass

A playable's `page` (docs/GAMES_PUBLISHING.md) is the folder of its page on the download site: `wiki.json`, the pictures it names and `thumb.png`, published to
`site/wiki/<slug>/` and `site/thumbs/<slug>.png` so a game and its page arrive in one pull request.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path, PurePosixPath


CATEGORIES = ("games", "prototypes", "tests", "demos")
MANIFEST_NAME = "games-publish.json"
CATALOG_NAME = ".games-catalog.json"
# A page's files: the page, its pictures (site/wiki.py takes PNG and JPEG) and the thumbnail.
PAGE_FILE = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]*\.(?i:png|jpe?g)|wiki\.json")
PNG = b"\x89PNG\r\n\x1a\n"


class PublishError(RuntimeError):
    """A manifest or export is unsafe or invalid."""


def safe_relative(value: object, field: str, *, allow_dot: bool = False) -> Path:
    if not isinstance(value, str) or not value.strip():
        raise PublishError(f"{field} must be a non-empty string")
    normalized = value.replace("\\", "/")
    path = PurePosixPath(normalized)
    if path.is_absolute() or ".." in path.parts:
        raise PublishError(f"{field} must stay inside the repository: {value!r}")
    if not allow_dot and normalized in (".", "./"):
        raise PublishError(f"{field} cannot be the repository root")
    return Path(*path.parts)


def load_manifest(root: Path) -> dict:
    path = root / MANIFEST_NAME
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise PublishError(f"cannot read {MANIFEST_NAME}: {error}") from error
    if data.get("version") != 1:
        raise PublishError("games-publish.json must have version 1")
    if not isinstance(data.get("target_repository"), str):
        raise PublishError("target_repository must be a string")
    collections = data.get("collections")
    if not isinstance(collections, dict):
        raise PublishError("collections must be an object")
    unknown = sorted(set(collections) - set(CATEGORIES))
    if unknown:
        raise PublishError(f"unknown collections: {', '.join(unknown)}")
    for category in CATEGORIES:
        if not isinstance(collections.get(category), list) or not collections[category]:
            raise PublishError(f"collections.{category} must be a non-empty array")
    playables = data.get("playables")
    if not isinstance(playables, list) or not playables:
        raise PublishError("playables must be a non-empty array")
    slugs = set()
    for index, playable in enumerate(playables):
        if not isinstance(playable, dict):
            raise PublishError(f"playables[{index}] must be an object")
        slug = playable.get("slug")
        if not isinstance(slug, str) or not re.fullmatch(r"[a-z0-9]+(?:-[a-z0-9]+)*", slug):
            raise PublishError(f"playables[{index}].slug must be lowercase kebab-case")
        if slug in slugs:
            raise PublishError(f"duplicate playable slug: {slug}")
        slugs.add(slug)
        if not isinstance(playable.get("name"), str) or not playable["name"].strip():
            raise PublishError(f"playables[{index}].name must be a non-empty string")
        safe_relative(playable.get("entry"), f"playables[{index}].entry")
        files = playable.get("files")
        if not isinstance(files, list) or not files:
            raise PublishError(f"playables[{index}].files must be a non-empty array")
        for file_index, value in enumerate(files):
            safe_relative(value, f"playables[{index}].files[{file_index}]")
        if playable.get("kind", "3d") not in ("3d", "2d"):
            raise PublishError(f"playables[{index}].kind must be \"3d\" (the default, played by RedEngine.exe) or \"2d\" (a *.game2d.json played by re2d.exe)")
        binary = playable.get("binary")
        if binary is not None and (not isinstance(binary, str) or not re.fullmatch(r"[a-z][a-z0-9_-]{0,39}", binary)):
            raise PublishError(
                f"playables[{index}].binary must name the executable that plays it, like \"killchain\" (a package binary of the engine's workspace; "
                "the default is the client its kind needs: re2, or re2d for 2d)"
            )
        arguments = playable.get("arguments")
        if not isinstance(arguments, list) or any(
            not isinstance(value, str) or "\n" in value or "\r" in value for value in arguments
        ):
            raise PublishError(f"playables[{index}].arguments must be an array of single-line strings")
        if "page" in playable:
            safe_relative(playable["page"], f"playables[{index}].page")
    return data


def files_under(source: Path):
    if source.is_symlink():
        raise PublishError(f"symlinks are not published: {source}")
    if source.is_file():
        yield source, Path(source.name)
        return
    if not source.is_dir():
        raise PublishError(f"source does not exist: {source}")
    for candidate in sorted(source.rglob("*")):
        if candidate.is_symlink():
            raise PublishError(f"symlinks are not published: {candidate}")
        if candidate.is_file():
            yield candidate, candidate.relative_to(source)


def page_files(root: Path, playable: dict):
    """(file, published path) for every file of a playable's `page`: `wiki.json` and its pictures to `site/wiki/<slug>/`, `thumb.png` to `site/thumbs/<slug>.png`."""
    slug, rel = playable["slug"], playable["page"]
    folder = root / safe_relative(rel, f"{slug}.page")
    if folder.is_symlink() or not folder.is_dir():
        raise PublishError(f"{slug}: page {rel} is not a folder")
    if not (folder / "wiki.json").is_file():
        raise PublishError(f"{slug}: page {rel} has no wiki.json")
    for item in sorted(folder.iterdir()):
        if item.is_symlink() or not item.is_file() or not PAGE_FILE.fullmatch(item.name):
            raise PublishError(f"{slug}: page {rel} may hold only wiki.json, PNG and JPEG pictures and thumb.png, not {item.name}")
        if item.name == "thumb.png":
            if item.read_bytes()[:8] != PNG:
                raise PublishError(f"{slug}: {rel}/thumb.png is not a PNG")
            yield item, f"site/thumbs/{slug}.png"
        else:
            yield item, f"site/wiki/{slug}/{item.name}"


def is_managed(path: str) -> bool:
    """A path an export may write, and so delete once it no longer publishes it: under a collection, or a page (`site/wiki/<slug>/<file>`, `site/thumbs/<slug>.png`)."""
    parts = PurePosixPath(path).parts
    if not parts or ".." in parts or PurePosixPath(path).is_absolute():
        return False
    if parts[0] in CATEGORIES:
        return len(parts) > 1
    return parts[:2] == ("site", "wiki") and len(parts) == 4 or parts[:2] == ("site", "thumbs") and len(parts) == 3


def export_tree(root: Path, staging: Path, manifest: dict, revision: str) -> dict:
    emitted: dict[str, Path] = {}
    catalog_files = []
    counts = {category: 0 for category in CATEGORIES}

    for category in CATEGORIES:
        (staging / category).mkdir(parents=True, exist_ok=True)
        for index, entry in enumerate(manifest["collections"][category]):
            if not isinstance(entry, dict):
                raise PublishError(f"collections.{category}[{index}] must be an object")
            source_rel = safe_relative(entry.get("source"), f"{category}[{index}].source")
            destination_rel = safe_relative(
                entry.get("destination", "."),
                f"{category}[{index}].destination",
                allow_dot=True,
            )
            source = (root / source_rel).resolve()
            try:
                source.relative_to(root.resolve())
            except ValueError as error:
                raise PublishError(f"source escapes repository: {source_rel}") from error

            is_file = source.is_file()
            for candidate, nested in files_under(source):
                suffix = Path(candidate.name) if is_file and destination_rel == Path(".") else nested
                target_rel = Path(category) / destination_rel / suffix
                target_key = target_rel.as_posix()
                if target_key in emitted:
                    raise PublishError(
                        f"two entries publish {target_key}: {emitted[target_key]} and {candidate}"
                    )
                emitted[target_key] = candidate
                target = staging / target_rel
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(candidate, target, follow_symlinks=False)
                digest = hashlib.sha256(target.read_bytes()).hexdigest()
                catalog_files.append({"path": target_key, "sha256": digest})
                counts[category] += 1

    pages = 0
    for playable in manifest["playables"]:
        if "page" not in playable:
            continue
        pages += 1
        for candidate, target_key in page_files(root, playable):
            emitted[target_key] = candidate
            target = staging / target_key
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(candidate, target, follow_symlinks=False)
            catalog_files.append({"path": target_key, "sha256": hashlib.sha256(target.read_bytes()).hexdigest()})

    catalog = {
        "schema_version": 1,
        "source_repository": manifest["source_repository"],
        "source_revision": revision,
        "target_repository": manifest["target_repository"],
        "collections": counts,
        "pages": pages,
        "playables": manifest["playables"],
        "files": sorted(catalog_files, key=lambda item: item["path"]),
    }
    published_paths = set(emitted)
    for playable in catalog["playables"]:
        if playable["entry"] not in published_paths:
            raise PublishError(f"playable entry is not published: {playable['entry']}")
        for value in playable["files"]:
            prefix = value.rstrip("/") + "/"
            if value not in published_paths and not any(path.startswith(prefix) for path in published_paths):
                raise PublishError(f"playable file is not published: {value}")
    (staging / CATALOG_NAME).write_text(
        json.dumps(catalog, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return catalog


def git_revision(root: Path) -> str:
    result = subprocess.run(
        ["git", "-C", str(root), "rev-parse", "HEAD"],
        check=False,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip() if result.returncode == 0 else "working-tree"


def previously_published(output: Path) -> set[str]:
    """The paths the last export wrote, from the catalog it left in `output` (empty when there is none or it cannot be read).

    Only these may ever be deleted: anything else under the managed folders belongs to someone else (hand-added games, for instance) and is left alone.
    """
    try:
        data = json.loads((output / CATALOG_NAME).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return set()
    paths: set[str] = set()
    for item in data.get("files", []) if isinstance(data, dict) else []:
        value = item.get("path") if isinstance(item, dict) else None
        if isinstance(value, str) and is_managed(value.replace("\\", "/")):
            paths.add("/".join(PurePosixPath(value.replace("\\", "/")).parts))
    return paths


def prune_empty_dirs(start: Path, stop: Path) -> None:
    """Removes `start` and its parents while they are empty directories, never going above or removing `stop`."""
    current = start
    while current != stop and stop in current.parents and current.is_dir() and not any(current.iterdir()):
        current.rmdir()
        current = current.parent


def page_problems(staging: Path, catalog: dict, games_repo: Path) -> list[str]:
    """What the Games repository's own page rules (`site/wiki.py`, the rules its site build enforces) say about every exported page; a page the
    rules reject would stop the download site from building, so it is refused before it is published."""
    import importlib.util

    spec = importlib.util.spec_from_file_location("games_site_wiki", games_repo / "site" / "wiki.py")
    wiki = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(wiki)
    try:
        meta = json.loads((games_repo / "site" / "games-meta.json").read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        meta = {}
    try:
        pending = set(json.loads((games_repo / "site" / "wiki-pending.json").read_text(encoding="utf-8")).get("pending", []))
    except (OSError, json.JSONDecodeError, AttributeError):
        pending = set()
    out = []
    for playable in catalog["playables"]:
        slug = playable["slug"]
        if "page" not in playable and not (games_repo / "site" / "wiki" / slug / "wiki.json").is_file() and slug not in pending:
            out.append(f"{slug}: has no page (the site build fails for a released game without one): add a `page` folder to its playable (docs/GAMES_PUBLISHING.md)")
        if "page" in playable:
            folder = staging / "site" / "wiki" / slug
            data = json.loads((folder / "wiki.json").read_text(encoding="utf-8"))
            author = meta.get(slug, {}).get("author", "human") if isinstance(meta.get(slug), dict) else "human"
            out += [f"{slug} ({playable['page']}/wiki.json): {m}" for m in wiki.problems(data, author, folder)]
    return out


def checked_pages(staging: Path, catalog: dict, games_repo: Path | None) -> None:
    if games_repo is None:
        return
    if not (games_repo / "site" / "wiki.py").is_file():
        raise PublishError(f"{games_repo} has no site/wiki.py: it is not a RedEngineGames checkout")
    problems = page_problems(staging, catalog, games_repo)
    if problems:
        raise PublishError("pages the download site would refuse (rules: RedEngineGames site/wiki.py):\n  " + "\n  ".join(problems))


def publish(root: Path, output: Path, revision: str) -> dict:
    """Writes what the manifest publishes into `output` and nothing else is touched.

    Files the export produces are written (replacing their old versions). Files a *previous* export published (they are in the catalog it left) that this one no longer
    produces are removed, and the folders that leaves empty. Every other file under `games/`, `prototypes/`, `tests/`, `demos/`, `site/wiki/` and `site/thumbs/` is someone
    else's and is never deleted: the old behaviour (delete each whole folder, then copy) wiped the hand-added games in `RedEngineGames/games` on the next push to the engine.
    Pages are checked against `output`'s own page rules first, when it has them.
    """
    manifest = load_manifest(root)
    output.mkdir(parents=True, exist_ok=True)
    for managed in (*CATEGORIES, "site/wiki", "site/thumbs"):
        target = output / managed
        if target.exists() and (target.is_symlink() or not target.is_dir()):
            raise PublishError(f"managed output is not a directory: {target}")
    with tempfile.TemporaryDirectory(prefix="games-publish-", dir=output.parent) as temp:
        staging = Path(temp)
        catalog = export_tree(root, staging, manifest, revision)
        checked_pages(staging, catalog, output if (output / "site" / "wiki.py").is_file() else None)
        new_paths = {item["path"] for item in catalog["files"]}
        for relative in sorted(previously_published(output) - new_paths):
            path = output / relative
            if path.is_symlink() or path.is_file():
                path.unlink()
                parts = PurePosixPath(relative).parts
                prune_empty_dirs(path.parent, output.joinpath(*parts[:2]) if parts[0] == "site" else output / parts[0])
        for relative in sorted(new_paths):
            target = output / relative
            if target.is_symlink() or (target.exists() and not target.is_file()):
                raise PublishError(f"managed output is not a file: {target}")
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(staging / relative, target)
        shutil.copy2(staging / CATALOG_NAME, output / CATALOG_NAME)
    return catalog


def check(root: Path, revision: str, games_repo: Path | None = None) -> dict:
    manifest = load_manifest(root)
    with tempfile.TemporaryDirectory(prefix="games-publish-check-") as temp:
        catalog = export_tree(root, Path(temp), manifest, revision)
        checked_pages(Path(temp), catalog, games_repo)
        return catalog


def sources_of(root: Path, manifest: dict, published: str) -> list[str]:
    """The engine paths a published path comes from (a published folder may gather several entries; a file comes from one)."""
    out = []
    for category in CATEGORIES:
        for entry in manifest["collections"][category]:
            source = safe_relative(entry.get("source"), "source").as_posix()
            destination = safe_relative(entry.get("destination", "."), "destination", allow_dot=True).as_posix()
            base = category if destination == "." else f"{category}/{destination}"
            if published == base or base.startswith(published + "/"):
                out.append(source)
            elif published.startswith(base + "/"):
                out.append(source if (root / source).is_file() else source + published[len(base):])
    return out


def notes(root: Path, since: str) -> list[str]:
    """`<slug>: <subject>` for every commit since `since` (merges left out) that changed what a playable publishes: the "What changed" of its next release.
    Its page is left out (a page is not part of the game: it never makes a new version). Nothing when `since` is not a commit of this repository."""
    manifest = load_manifest(root)
    known = subprocess.run(["git", "-C", str(root), "cat-file", "-e", f"{since}^{{commit}}"], capture_output=True).returncode == 0
    if not known:
        return []
    lines = []
    for playable in manifest["playables"]:
        sources = sorted({s for f in playable["files"] for s in sources_of(root, manifest, f)})
        if not sources:
            continue
        log = subprocess.run(
            ["git", "-C", str(root), "log", "--no-merges", "--reverse", "--format=%s", f"{since}..HEAD", "--", *sources],
            capture_output=True, text=True, check=True,
        ).stdout
        seen = []
        for subject in log.splitlines():
            subject = subject.strip()
            if subject and subject not in seen:
                seen.append(subject)
        lines += [f"{playable['slug']}: {subject}" for subject in seen]
    return lines


def play_check(root: Path, engine: list[str]) -> int:
    """Every game of the `games` collection that the engine's own client plays (a 3D scene, no `binary`): `lint` finds no errors, the `nav` graph its bots
    route along can be travelled when it has one, and its own `checks` pass when it has them. These are the checks a game's author may have no toolchain to run."""
    manifest = load_manifest(root)
    failed = 0
    for playable in manifest["playables"]:
        if not playable["entry"].startswith("games/") or playable.get("kind", "3d") != "3d" or playable.get("binary"):
            continue
        scenes = sources_of(root, manifest, playable["entry"])
        if len(scenes) != 1:
            raise PublishError(f"{playable['slug']}: cannot tell which engine file {playable['entry']} is published from")
        scene = scenes[0]
        data = json.loads((root / scene).read_text(encoding="utf-8"))
        commands = ["lint"] + (["nav"] if "nav" in data else []) + (["verify"] if "checks" in data else [])
        results = []
        for command in commands:
            run = subprocess.run([*engine, command, scene], cwd=root, capture_output=True, text=True)
            results.append((command, run))
        bad = [(c, r) for c, r in results if r.returncode != 0]
        print(f"{'FAIL' if bad else 'ok  '} {playable['slug']}: {' '.join(c for c, _ in results)}  ({scene})", flush=True)
        for command, run in bad:
            failed += 1
            text = (run.stdout + run.stderr).strip().splitlines()
            print(f"     `red_engine2 {command} {scene}` failed:")
            print("\n".join("       " + line for line in text[-25:]), flush=True)
    return 1 if failed else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("command", choices=("check", "export", "notes", "play-check"))
    parser.add_argument("--output", type=Path, help="Games repository checkout (export only)")
    parser.add_argument("--revision", help="source Git revision recorded in the catalog")
    parser.add_argument("--games-repo", type=Path, help="a RedEngineGames checkout whose page rules (site/wiki.py) every page must meet (check only)")
    parser.add_argument("--since", help="the engine commit the last sync published (notes only)")
    parser.add_argument("--engine", default="red_engine2", help="the CLI that checks the games, as a command line (play-check only)")
    args = parser.parse_args()

    root = Path(__file__).resolve().parents[1]
    revision = args.revision or git_revision(root)
    try:
        if args.command == "notes":
            for line in notes(root, args.since or ""):
                print(line)
            return 0
        if args.command == "play-check":
            return play_check(root, shlex.split(args.engine))
        if args.command == "check":
            if args.output:
                raise PublishError("--output is only valid with export")
            catalog = check(root, revision, args.games_repo)
        else:
            if not args.output:
                raise PublishError("export requires --output")
            catalog = publish(root, args.output.resolve(), revision)
    except PublishError as error:
        print(f"publish-games: {error}", file=sys.stderr)
        return 2

    total = sum(catalog["collections"].values())
    print(
        json.dumps(
            {"ok": True, "files": total, "collections": catalog["collections"], "pages": catalog["pages"]},
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
