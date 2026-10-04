#!/usr/bin/env bash
# Unpacks every archive in <dist> into an empty directory and runs the CLI the way a fresh machine would: no repository, no Rust.
set -euo pipefail
dist="$(cd "${1:?dist directory}" && pwd)"
(cd "$dist" && sha256sum -c SHA256SUMS)
work="$(mktemp -d)"; trap 'rm -rf "$work"' EXIT
for archive in "$dist"/*.tar.gz; do
  d="$work/$(basename "$archive" .tar.gz)"; mkdir -p "$d"; tar -C "$d" -xzf "$archive"
  bin="$(echo "$d"/*/red_engine2)"
  empty="$(mktemp -d)"
  echo "== $(basename "$archive")"
  (cd "$empty"
   "$bin" describe --brief | head -3
   "$bin" doctor || true
   "$bin" recipe coin_run --new coin.json
   "$bin" lint coin.json
   "$bin" verify coin.json --no-views
   case "$archive" in *headless*) ;; *) "$bin" frame coin.json coin.png; test -s coin.png ;; esac)
done
echo "release smoke test passed"
