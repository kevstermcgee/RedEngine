#!/usr/bin/env python3
"""Export curated engine-made content into the companion Games repository."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path, PurePosixPath


CATEGORIES = ("games", "prototypes", "tests", "demos")
MANIFEST_NAME = "games-publish.json"
CATALOG_NAME = ".games-catalog.json"


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
        arguments = playable.get("arguments")
        if not isinstance(arguments, list) or any(
            not isinstance(value, str) or "\n" in value or "\r" in value for value in arguments
        ):
            raise PublishError(f"playables[{index}].arguments must be an array of single-line strings")
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

    catalog = {
        "schema_version": 1,
        "source_repository": manifest["source_repository"],
        "source_revision": revision,
        "target_repository": manifest["target_repository"],
        "collections": counts,
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
        if not isinstance(value, str):
            continue
        normalized = value.replace("\\", "/")
        parts = PurePosixPath(normalized).parts
        if parts and parts[0] in CATEGORIES and len(parts) > 1 and ".." not in parts and not PurePosixPath(normalized).is_absolute():
            paths.add("/".join(parts))
    return paths


def prune_empty_dirs(start: Path, stop: Path) -> None:
    """Removes `start` and its parents while they are empty directories, never going above or removing `stop`."""
    current = start
    while current != stop and stop in current.parents and current.is_dir() and not any(current.iterdir()):
        current.rmdir()
        current = current.parent


def publish(root: Path, output: Path, revision: str) -> dict:
    """Writes what the manifest publishes into `output` and nothing else is touched.

    Files the export produces are written (replacing their old versions). Files a *previous* export published (they are in the catalog it left) that this one no longer
    produces are removed, and the folders that leaves empty. Every other file under `games/`, `prototypes/`, `tests/` and `demos/` is someone else's and is never
    deleted: the old behaviour (delete each whole folder, then copy) wiped the hand-added games in `RedEngineGames/games` on the next push to the engine.
    """
    manifest = load_manifest(root)
    output.mkdir(parents=True, exist_ok=True)
    for category in CATEGORIES:
        target = output / category
        if target.exists() and (target.is_symlink() or not target.is_dir()):
            raise PublishError(f"managed output is not a directory: {target}")
    with tempfile.TemporaryDirectory(prefix="games-publish-", dir=output.parent) as temp:
        staging = Path(temp)
        catalog = export_tree(root, staging, manifest, revision)
        new_paths = {item["path"] for item in catalog["files"]}
        for relative in sorted(previously_published(output) - new_paths):
            path = output / relative
            if path.is_symlink() or path.is_file():
                path.unlink()
                prune_empty_dirs(path.parent, output / PurePosixPath(relative).parts[0])
        for relative in sorted(new_paths):
            target = output / relative
            if target.is_symlink() or (target.exists() and not target.is_file()):
                raise PublishError(f"managed output is not a file: {target}")
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(staging / relative, target)
        shutil.copy2(staging / CATALOG_NAME, output / CATALOG_NAME)
    return catalog


def check(root: Path, revision: str) -> dict:
    manifest = load_manifest(root)
    with tempfile.TemporaryDirectory(prefix="games-publish-check-") as temp:
        return export_tree(root, Path(temp), manifest, revision)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("check", "export"))
    parser.add_argument("--output", type=Path, help="Games repository checkout (export only)")
    parser.add_argument("--revision", help="source Git revision recorded in the catalog")
    args = parser.parse_args()

    root = Path(__file__).resolve().parents[1]
    revision = args.revision or git_revision(root)
    try:
        if args.command == "check":
            if args.output:
                raise PublishError("--output is only valid with export")
            catalog = check(root, revision)
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
            {"ok": True, "files": total, "collections": catalog["collections"]},
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
