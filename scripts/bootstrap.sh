#!/bin/sh
# RedEngine on a fresh Linux machine: downloads the prebuilt binaries of a release, verifies their checksum, installs them under ~/.local and runs `red_engine2 doctor`.
# No Rust, no compiler, no clone.
#
#   curl -fsSL https://raw.githubusercontent.com/kevstermcgee/RedEngine/main/scripts/bootstrap.sh | sh
#   sh scripts/bootstrap.sh [--tag v0.3.0] [--prefix DIR] [--headless] [--no-doctor] [--latest]
#
# Run from inside a checkout (sh scripts/bootstrap.sh), it installs the release that was built from exactly THIS checkout's sources (scripts/prebuilt.py: a source fingerprint, a
# verified checksum) into the checkout's target directory, where `scripts/dev` and `scripts/red` find it: no compile. If no release matches (none published, the sources differ, an
# uncommitted edit, no network) it installs nothing, says why and exits 3: build instead (`scripts/dev red describe --brief`, about 7 minutes cold on four cores, run it in the
# background). `--latest`, or `--tag`, keeps the old behaviour: the newest release into ~/.local, which may differ from the checkout.
#
# --headless installs the build without rendering or audio (lint, reach, walk, verify, sim, plan, the server: it needs nothing but libc). Without the flag you get the full set
# (the CLI with `frame`/`tour`, the client, server and bots) when libasound is present, and the headless one, with a note, when it is not.
# Environment: RED_REPO (owner/name), RED_RELEASE_BASE (where releases are downloaded from: BASE/<tag>/<file>), RED_PREFIX.
set -eu

repo="${RED_REPO:-kevstermcgee/RedEngine}"
base="${RED_RELEASE_BASE:-https://github.com/$repo/releases/download}"
prefix="${RED_PREFIX:-$HOME/.local}"
tag="${RED_VERSION:-}"
headless=0
doctor=1
latest=0

while [ $# -gt 0 ]; do
  case "$1" in
    --tag) tag="$2"; shift 2 ;;
    --prefix) prefix="$2"; shift 2 ;;
    --headless) headless=1; shift ;;
    --no-doctor) doctor=0; shift ;;
    --latest) latest=1; shift ;;
    -h|--help) sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "bootstrap: unknown option $1 (try --help)" >&2; exit 2 ;;
  esac
done

here="$(cd "$(dirname "$0")" 2>/dev/null && pwd || true)"
if [ "$latest" = 0 ] && [ -z "$tag" ] && [ -n "$here" ] && [ -f "$here/prebuilt.py" ] && [ -f "$here/../Cargo.toml" ] && command -v python3 >/dev/null 2>&1; then
  hl=""; [ "$headless" = 1 ] && hl="--headless"
  if exe="$(python3 "$here/prebuilt.py" ensure --refresh $hl)"; then
    echo "bootstrap: installed for this checkout: $exe"
    echo "bootstrap: next:  scripts/dev red describe --brief    (it uses this binary: no compile)"
    exit 0
  fi
  echo "bootstrap: nothing was installed. Build instead:  scripts/dev red describe --brief   (about 7 minutes cold on four cores: run it in the background)" >&2
  echo "           or install the newest release anyway, which may differ from this checkout:  sh scripts/bootstrap.sh --latest" >&2
  exit 3
fi

[ "$(uname -s)" = "Linux" ] && [ "$(uname -m)" = "x86_64" ] || {
  echo "bootstrap: prebuilt binaries exist for Linux x86_64 only (this is $(uname -s) $(uname -m))." >&2
  echo "           On Windows play the games from the RedEngineGames launcher; elsewhere build from source: git clone https://github.com/$repo && scripts/dev doctor" >&2
  exit 1
}
for tool in curl tar sha256sum; do
  command -v "$tool" >/dev/null 2>&1 || { echo "bootstrap: needs $tool" >&2; exit 1; }
done

if [ -z "$tag" ]; then
  tag="$(curl -fsSL "https://api.github.com/repos/$repo/releases/latest" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)"
  [ -n "$tag" ] || { echo "bootstrap: could not find the latest release of $repo (none published yet? pass --tag)" >&2; exit 1; }
fi

if [ "$headless" = 0 ] && ! { ldconfig -p 2>/dev/null | grep -q 'libasound\.so\.2' || ls /usr/lib/*/libasound.so.2 /usr/lib64/libasound.so.2 /usr/lib/libasound.so.2 >/dev/null 2>&1; }; then
  echo "bootstrap: libasound.so.2 is not installed, so the full set (it plays sound) would not start; installing the headless build." >&2
  echo "           For rendering and the client: sudo apt-get install libasound2 (then run this again)." >&2
  headless=1
fi
name="red-engine-$tag-linux-x86_64"
[ "$headless" = 1 ] && name="$name-headless"

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
echo "bootstrap: RedEngine $tag ($name)"
curl -fsSL -o "$tmp/$name.tar.gz" "$base/$tag/$name.tar.gz" || { echo "bootstrap: could not download $base/$tag/$name.tar.gz" >&2; exit 1; }
curl -fsSL -o "$tmp/SHA256SUMS" "$base/$tag/SHA256SUMS" || { echo "bootstrap: could not download the checksums" >&2; exit 1; }
want="$(grep " $name.tar.gz\$" "$tmp/SHA256SUMS" | cut -d' ' -f1)"
have="$(sha256sum "$tmp/$name.tar.gz" | cut -d' ' -f1)"
[ -n "$want" ] && [ "$want" = "$have" ] || { echo "bootstrap: checksum mismatch for $name.tar.gz (expected ${want:-none}, got $have): refusing to install" >&2; exit 1; }

dest="$prefix/share/red_engine2/$tag"
rm -rf "$dest"; mkdir -p "$dest" "$prefix/bin"
tar -C "$tmp" -xzf "$tmp/$name.tar.gz"
cp -R "$tmp/$name/." "$dest/"
for bin in "$dest"/*; do
  [ -f "$bin" ] && [ -x "$bin" ] && ln -sf "$bin" "$prefix/bin/$(basename "$bin")"
done
echo "bootstrap: installed to $dest (linked into $prefix/bin)"
case ":$PATH:" in *":$prefix/bin:"*) ;; *) echo "bootstrap: add it to your PATH:  export PATH=\"$prefix/bin:\$PATH\"" ;; esac
if [ "$doctor" = 1 ]; then
  echo "bootstrap: what this machine can do:"
  "$prefix/bin/red_engine2" doctor || true
fi
echo "bootstrap: next:  red_engine2 describe --brief    red_engine2 recipe    red_engine2 new-game ../mygame"
