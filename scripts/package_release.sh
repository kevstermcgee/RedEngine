#!/usr/bin/env bash
# Packages the Linux binaries of an already-built tree into dist/: the full set and the headless CLI, each with a checksum.
#   scripts/package_release.sh <version> <out-dir> [target-dir]        (target-dir defaults to ./target; the headless CLI is expected in <target-dir>/headless)
set -euo pipefail
version="${1:?version}"; out="${2:?output directory}"; target="${3:-target}"
mkdir -p "$out"
stage="$(mktemp -d)"; trap 'rm -rf "$stage"' EXIT
readme() {
  cat > "$1/README.txt" <<TXT
RedEngine $version (Linux x86_64)

  red_engine2   the self-describing CLI: start with  ./red_engine2 describe --brief   and   ./red_engine2 doctor
$2
Put this folder on your PATH (or run scripts/bootstrap.sh, which does it for you) and see https://github.com/kevstermcgee/RedEngine
TXT
}
full="red-engine-$version-linux-x86_64"
mkdir -p "$stage/$full"
for b in red_engine2 re2 red_server red_bot red_relay; do cp "$target/release/$b" "$stage/$full/"; done
readme "$stage/$full" "  re2           the game client (needs libasound2 and a window system)
  red_server    the dedicated server;  red_bot  bots;  red_relay  the relay"
tar -C "$stage" -czf "$out/$full.tar.gz" "$full"
head="red-engine-$version-linux-x86_64-headless"
mkdir -p "$stage/$head"
cp "$target/headless/release/red_engine2" "$stage/$head/"
readme "$stage/$head" "  (headless build: lint, reach, walk, verify --no-views, sim, plan, patch ... no rendering or audio; needs only libc)"
tar -C "$stage" -czf "$out/$head.tar.gz" "$head"
(cd "$out" && sha256sum ./*.tar.gz | sed 's# \./# #' > SHA256SUMS)
ls -l "$out"
