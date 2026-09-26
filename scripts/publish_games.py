#!/usr/bin/env python3
"""Export curated engine-made content into the companion Games repository."""

from __future__ import annotations

import argparse
import hashlib
import json
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
        "files": sorted(catalog_files, key=lambda item: item["path"]),
    }
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


def publish(root: Path, output: Path, revision: str) -> dict:
    manifest = load_manifest(root)
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="games-publish-", dir=output.parent) as temp:
        staging = Path(temp)
        catalog = export_tree(root, staging, manifest, revision)
        for category in CATEGORIES:
            target = output / category
            if target.exists():
                if target.is_symlink() or not target.is_dir():
                    raise PublishError(f"managed output is not a directory: {target}")
                shutil.rmtree(target)
            shutil.move(str(staging / category), str(target))
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

