//! `red_engine2 new-game <dir>`: scaffold a game project that *uses* Red instead of forking it (see [`super::game`]).
//!
//! The scaffold is deliberately tiny and already green: a `game.json`, one blueprint and the map it builds, a
//! `CLAUDE.md` that teaches the loop in about thirty lines, a `STATUS.md` handoff, the `scripts/red` wrapper that
//! finds and builds the pinned engine (bash and PowerShell), and a CI workflow that runs `red check` headless.

use super::game::EngineRef;
use std::path::{Path, PathBuf};

/// What kind of game to start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    /// Rooms, doors and people on foot: a blueprint builds the map.
    #[default]
    Walk,
    /// A kart race: `race-track` builds the circuit (the eight animals come with the engine), with a lobby-and-rounds `match` block and bots.
    Race,
    /// A 2D game for the browser: one `NAME.game2d.json` (see `describe 2d`), already verified.
    TwoD,
}

/// The hosting script every project gets as `deploy/install.sh` (a per-user systemd service; `deploy/install.sh --info` says how friends connect).
const HOST_SH: &str = include_str!("../../deploy/game-host.sh");

const RACE_CLAUDE_MD: &str = r#"# {{NAME}}

A Red Engine 2 **kart race** project. **The engine is not in this repo**: `game.json` pins it (`engine`), and `scripts/red` fetches and builds that version on first use.
Never copy engine source here; if the engine needs a change, make it in the engine repo.

## First 60 seconds
```bash
scripts/red doctor               # which engine, is it built, is the toolchain there? (scripts\red.ps1 on Windows)
scripts/red status               # resume: facts + git + STATUS.md (what is done / in flight / next)
scripts/red describe --brief     # the engine's own ~1 KB manual; then `scripts/red search "<question>"`
```

## The loop
```bash
scripts/red race-track maps/main.json --half-width 130 --laps 4   # a whole raceable map from a few numbers (`race-track --help`); re-run to change the shape
scripts/red race-test maps/main.json                              # 8 bots race it headless: lap times per animal, exit 1 if one cannot finish
scripts/red check                                                  # lint + the karts audit + the map's own checks
scripts/red frame maps/main.json out/look.png --eye 60,110,150 --at 0,0,20   # LOOK at it (camera.far must exceed the distance)
scripts/red playtest maps/main.json --out out/playtest             # a scripted client plays it and takes pictures
```
- The eight animals (Duck, Bunny, Deer, Coyote, Hawk, Bear, Wolf, Beaver) are the engine's `karts` pack; the map instances them as `kart_<animal>`. Their stats are in
  `sim::kart` (`scripts/red search "kart driver stats"`). Terrain, pickups, bots, the lobby animal picker, the HUD and the gamepad mapping are engine features.
- `race-track` writes the whole map. To customise, edit the JSON's objects (or copy `race-track` output and script your changes, e.g. a Python generator like Great
  Outdoors'); keep `race.line` (the bots' racing line) in step with any new route, and re-run `race-test`: a bot that did not finish tells you where it stopped.
- SPEC.md "Races" has every `race` key (surfaces, item boxes, the grid, the line).

## Play and host
```bash
scripts/red play-local           # a race against bots on this machine (hosts one and joins it)
deploy/install.sh                # host it for friends on this Linux box (user systemd service, QUIC + join key); `--info` prints how they connect
scripts/red game publish ../RedEngineGames   # put it where friends can install it (then commit and push that repo yourself)
```

## Rules for whoever works here next
1. `scripts/red check` and `scripts/red race-test maps/main.json` before every commit and before you say "done".
2. Record progress: `scripts/red status --note "what changed" --section done|now|next|blocked|notes`. Do it at every checkpoint.
3. Keep this file short and true. Never commit `~/.config/<name>/` (the server identity and join key live there, outside the project).
"#;

const CLAUDE_MD: &str = r#"# {{NAME}}

A Red Engine 2 game project. **The engine is not in this repo**: `game.json` pins it (`engine`), and `scripts/red` fetches and
builds that version on first use. Never copy engine source here; if the engine needs a change, make it in the engine repo.

## First 60 seconds
```bash
scripts/red doctor               # which engine, is it built, is the toolchain there? (scripts\red.ps1 on Windows)
scripts/red status               # resume: facts + git + STATUS.md (what is done / in flight / next)
scripts/red describe --brief     # the engine's own ~1 KB manual; then `scripts/red search "<question>"`
```

## The loop
```bash
# 1. change WHAT THE MAP IS: blueprints/*.blueprint.json  (rooms, doors, spawns, fill; `scripts/red build --example` shows the format)
scripts/red build-all            # blueprints -> maps/*.json (walls, doors, lamps, zones, spawns, portals, checks)
scripts/red check                # blueprints build + equal their maps, every map passes its own `checks` (lint, reach, auto walks)
scripts/red plan maps/main.json  # LOOK at it (labelled top-down PNG); `tour` renders every room
```
- A failing walk names the object that blocked it (id, gap, passage width) and writes `out/verify/*_explain.png`.
  `scripts/red walk maps/main.json --auto --from X,Z --to X,Z` plans a route for you; never guess waypoints.
- **Game rules are data**: put `vars` / `rules` / `weapons` under the blueprint's `"scene"` block (`scripts/red describe rules`), and prove
  them with `checks.sim` scenarios (`scripts/red sim maps/main.json`). No Rust needed for most games.
- Maps under `maps/` are generated. Do not hand-edit them: `check` fails on drift. (If you must hand-edit, delete the blueprint.)
- **Assets grow locally first**: search core with `scripts/red catalog <need>`, then put specialized
  prefabs in `assets/gameplay.json` (already linked by `prefab_files`). Inspect both together with
  `scripts/red catalog --library assets/gameplay.json <need>`. Follow Reuse -> Modify -> Generate -> Import.

## Play locally (required for every game)
```bash
scripts/red play-local           # direct single-player: no server or network needed
```

## Multiplayer (optional in addition to local play)
```bash
scripts/red serve                # headless authoritative UDP server on the map in game.json (port 27015)
scripts/red play 127.0.0.1:27015   # the graphical client (run two)
```

## Rules for whoever works here next
1. `scripts/red check` before every commit and before you say "done".
2. Record progress: `scripts/red status --note "what changed" --section done|now|next|blocked|notes`. Do it at every checkpoint.
3. Keep this file short and true. Derived facts (binaries, counts) belong in `red_engine2 status`, not in prose.
"#;

const RED_SH: &str = r##"#!/usr/bin/env bash
# scripts/red: run the Red Engine version this project pins (game.json "engine"), building it on first use.
#   scripts/red doctor | check | build-all | info | play-local | serve | play [HOST:PORT] | status ... | <any red_engine2 command>
# Env: RED_ENGINE=/path/to/checkout (override), RED_UPDATE=1 (git fetch the pinned ref), RED_REBUILD=1 (force a build),
# RED_HEADLESS=1 (no graphics crates: CLI + server only, ideal for CI and containers), RED_PROFILE=debug|release (default debug).
set -eu
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
for f in "$HOME/.local/toolchain/env.sh" "$HOME/.cargo/env"; do [ -f "$f" ] && . "$f"; done
export CARGO_TERM_COLOR=never

json_get() { sed -n "s/.*\"$1\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\".*/\1/p" game.json | head -1; }

ENGINE="${RED_ENGINE:-}"
if [ -z "$ENGINE" ]; then
  P="$(json_get path)"
  case "$P" in
    "") ;;
    /*|[A-Za-z]:*) ENGINE="$(cd "$P" 2>/dev/null && pwd || true)" ;;
    *) ENGINE="$(cd "$ROOT/$P" 2>/dev/null && pwd || true)" ;;
  esac
fi
if [ -z "$ENGINE" ]; then
  GIT="$(json_get git)"; REF="$(json_get ref)"; REF="${REF:-master}"
  ENGINE="$ROOT/.red/engine"
  if [ ! -d "$ENGINE/.git" ]; then
    echo "red: cloning the engine ($GIT) into .red/engine ..." >&2
    git clone --quiet "$GIT" "$ENGINE"
    UPDATE=1
  fi
  if [ "${UPDATE:-${RED_UPDATE:-0}}" = "1" ]; then
    git -C "$ENGINE" fetch --quiet --tags origin
    git -C "$ENGINE" checkout --quiet --detach "origin/$REF" 2>/dev/null || git -C "$ENGINE" checkout --quiet --detach "$REF"
  fi
fi
[ -f "$ENGINE/Cargo.toml" ] || { echo "red: no engine at '$ENGINE' (fix game.json \"engine\" or set RED_ENGINE)" >&2; exit 2; }

PROFILE="${RED_PROFILE:-debug}"; PFLAG=""; [ "$PROFILE" = "release" ] && PFLAG="--release"
TARGET="${CARGO_TARGET_DIR:-$ENGINE/target}"
exe() { for c in "$TARGET/$PROFILE/$1" "$TARGET/$PROFILE/$1.exe"; do [ -f "$c" ] && { echo "$c"; return; }; done; echo "$TARGET/$PROFILE/$1"; }

REBUILD="${RED_REBUILD:-0}"
if [ "${1:-}" = "--rebuild" ]; then REBUILD=1; shift; fi
cmd="${1:-help}"; [ $# -gt 0 ] && shift
if [ "$cmd" = "help" ] || [ "$cmd" = "-h" ] || [ "$cmd" = "--help" ]; then sed -n '2,5p' "$0"; exit 0; fi
if [ "$cmd" = "doctor" ]; then
  echo "project  $ROOT"; echo "engine   $ENGINE ($(git -C "$ENGINE" rev-parse --short HEAD 2>/dev/null || echo 'not a git checkout'))"
  command -v cargo >/dev/null 2>&1 && echo "cargo    $(cargo --version)" || echo "cargo    NOT FOUND (install Rust: https://rustup.rs)"
  for b in red_engine2 red_server re2; do [ -f "$(exe $b)" ] && echo "built    $b" || echo "missing  $b (built on first use)"; done
  exit 0
fi
command -v cargo >/dev/null 2>&1 || { echo "red: cargo not found (install Rust: https://rustup.rs)" >&2; exit 127; }

if [ "${1:-}" = "--rebuild" ]; then REBUILD=1; shift; fi
MODE=default; FEATURES=""; [ "${RED_HEADLESS:-0}" = "1" ] && { MODE=headless; FEATURES="--no-default-features"; }
[ "$MODE" = "headless" ] && { [ "$cmd" = "play" ] || [ "$cmd" = "play-local" ]; } && { echo "red: $cmd needs graphics; unset RED_HEADLESS" >&2; exit 2; }
REQUIRED="red_engine2"; [ "$cmd" = "serve" ] && REQUIRED="$REQUIRED red_server"; { [ "$cmd" = "play" ] || [ "$cmd" = "play-local" ]; } && REQUIRED="$REQUIRED re2"
needs_build() {
  OUT="$(exe "$1")"; STAMP="$TARGET/$PROFILE/.red-wrapper-$1.mode"
  [ "$REBUILD" = "1" ] || [ ! -f "$OUT" ] && return 0
  [ -f "$STAMP" ] || return 0
  [ "$(cat "$STAMP")" != "$MODE" ] && return 0
  for f in "$ENGINE/Cargo.toml" "$ENGINE/Cargo.lock" "$ENGINE/build.rs"; do [ -f "$f" ] && [ "$f" -nt "$OUT" ] && return 0; done
  for d in "$ENGINE/src" "$ENGINE/assets"; do
    [ -d "$d" ] && [ -n "$(find "$d" -type f -newer "$OUT" -print -quit)" ] && return 0
  done
  return 1
}
BUILD=0; BINS=""
for b in $REQUIRED; do BINS="$BINS --bin $b"; needs_build "$b" && BUILD=1; done
if [ "$BUILD" = "1" ]; then
  echo "red: building required engine binaries ..." >&2
  cargo build --quiet $PFLAG $FEATURES --manifest-path "$ENGINE/Cargo.toml" $BINS
  for b in $REQUIRED; do printf '%s' "$MODE" > "$TARGET/$PROFILE/.red-wrapper-$b.mode"; done
fi

CLI="$(exe red_engine2)"
case "$cmd" in
  check|build-all|info|play-local|serve|play) exec "$CLI" game "$cmd" "$@" ;;
  *) exec "$CLI" "$cmd" "$@" ;;
esac
"##;

const RED_PS1: &str = r##"# scripts/red.ps1: the Windows-native twin of scripts/red (same commands and env vars).
#   powershell -File scripts\red.ps1 doctor | check | build-all | info | play-local | serve | play [HOST:PORT] | status ... | <any red_engine2 command>
param([Parameter(Position = 0)][string]$Cmd = 'help', [Parameter(ValueFromRemainingArguments = $true)][string[]]$Rest)
$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root
$env:CARGO_TERM_COLOR = 'never'
$cargoBin = Join-Path $HOME '.cargo\bin'
if ((Test-Path $cargoBin) -and (($env:PATH -split ';') -notcontains $cargoBin)) { $env:PATH = "$cargoBin;$env:PATH" }
$game = Get-Content game.json -Raw | ConvertFrom-Json

$Engine = $env:RED_ENGINE
if (-not $Engine -and $game.engine.path) { $p = if ([IO.Path]::IsPathRooted($game.engine.path)) { $game.engine.path } else { Join-Path $Root $game.engine.path }; if (Test-Path $p) { $Engine = (Resolve-Path $p).Path } }
if (-not $Engine) {
    $Engine = Join-Path $Root '.red\engine'
    $ref = if ($game.engine.ref) { $game.engine.ref } else { 'master' }
    $update = $env:RED_UPDATE -eq '1'
    if (-not (Test-Path (Join-Path $Engine '.git'))) { Write-Host "red: cloning the engine ($($game.engine.git)) into .red\engine ..."; git clone --quiet $game.engine.git $Engine; $update = $true }
    if ($update) { git -C $Engine fetch --quiet --tags origin; git -C $Engine checkout --quiet --detach "origin/$ref" 2>$null; if ($LASTEXITCODE -ne 0) { git -C $Engine checkout --quiet --detach $ref } }
}
if (-not (Test-Path (Join-Path $Engine 'Cargo.toml'))) { Write-Error "red: no engine at '$Engine' (fix game.json engine or set RED_ENGINE)"; exit 2 }
$Profile_ = if ($env:RED_PROFILE) { $env:RED_PROFILE } else { 'debug' }
$PFlag = if ($Profile_ -eq 'release') { @('--release') } else { @() }
$Target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Engine 'target' }
function Exe([string]$n) { Join-Path $Target "$Profile_\$n.exe" }

$rebuild = $env:RED_REBUILD -eq '1'
if ($Cmd -eq '--rebuild') {
    $rebuild = $true
    $Cmd = if ($Rest.Count -gt 0) { $Rest[0] } else { 'help' }
    $Rest = @($Rest | Select-Object -Skip 1)
}
if ($Cmd -in 'help', '-h', '--help') { Get-Content $PSCommandPath -TotalCount 2 | ForEach-Object { $_ -replace '^# ?', '' }; exit 0 }
if ($Cmd -eq 'doctor') {
    "project  $Root"; "engine   $Engine"
    if (Get-Command cargo -ErrorAction SilentlyContinue) { "cargo    $(cargo --version)" } else { 'cargo    NOT FOUND (install Rust: https://rustup.rs)' }
    foreach ($b in 'red_engine2', 'red_server', 're2') { if (Test-Path (Exe $b)) { "built    $b" } else { "missing  $b (built on first use)" } }
    exit 0
}
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { Write-Error 'red: cargo not found (install Rust: https://rustup.rs)'; exit 127 }
if ($Rest.Count -gt 0 -and $Rest[0] -eq '--rebuild') { $rebuild = $true; $Rest = @($Rest | Select-Object -Skip 1) }
$mode = if ($env:RED_HEADLESS -eq '1') { 'headless' } else { 'default' }
$features = if ($mode -eq 'headless') { @('--no-default-features') } else { @() }
if ($mode -eq 'headless' -and $Cmd -in 'play', 'play-local') { Write-Error "red: $Cmd needs graphics; unset RED_HEADLESS"; exit 2 }
$required = @('red_engine2'); if ($Cmd -eq 'serve') { $required += 'red_server' }; if ($Cmd -in 'play', 'play-local') { $required += 're2' }
function Needs-Build([string]$n) {
    $out = Exe $n; if ($rebuild -or -not (Test-Path $out)) { return $true }
    $stamp = Join-Path $Target "$Profile_\.red-wrapper-$n.mode"
    if (-not (Test-Path $stamp) -or (Get-Content $stamp -Raw) -ne $mode) { return $true }
    $inputs = @((Join-Path $Engine 'Cargo.toml'), (Join-Path $Engine 'Cargo.lock'), (Join-Path $Engine 'build.rs'))
    $inputs += Get-ChildItem (Join-Path $Engine 'src'), (Join-Path $Engine 'assets') -Recurse -File -ErrorAction SilentlyContinue | Select-Object -ExpandProperty FullName
    $built = (Get-Item $out).LastWriteTimeUtc
    return $null -ne ($inputs | Where-Object { (Test-Path $_) -and (Get-Item $_).LastWriteTimeUtc -gt $built } | Select-Object -First 1)
}
$build = $false; $bins = @()
foreach ($b in $required) { $bins += @('--bin', $b); if (Needs-Build $b) { $build = $true } }
if ($build) {
    Write-Host 'red: building required engine binaries ...'
    & cargo build --quiet @PFlag @features --manifest-path (Join-Path $Engine 'Cargo.toml') @bins
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    foreach ($b in $required) { Set-Content -NoNewline -Path (Join-Path $Target "$Profile_\.red-wrapper-$b.mode") -Value $mode }
}

$cli = Exe 'red_engine2'
switch ($Cmd) {
    { $_ -in 'check', 'build-all', 'info', 'play-local', 'serve', 'play' } { & $cli game $Cmd @Rest }
    default { & $cli $Cmd @Rest }
}
exit $LASTEXITCODE
"##;

const CI_YML: &str = r#"name: check
on: [push, pull_request]
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      # Headless engine build: CLI + server only, no GPU or windowing libraries needed.
      - run: RED_HEADLESS=1 bash scripts/red check
"#;

const LOCAL_ASSETS_README: &str = r#"# Game-local assets

`gameplay.json` is this game's incubator for specialized prefabs. Search the core first, prefer an
`extends` variant second, and create a new definition only when neither fits. The main blueprint
already includes this file through `prefab_files`, so built maps remain self-contained.

Discover local and core assets together:

```bash
scripts/red catalog --library assets/gameplay.json "what I need"
```

Use the structured `meta` fields shown by `scripts/red catalog --manifest`. If an asset proves useful
across games, propose it for the narrowest engine pack; do not copy the whole local library into core.
"#;

/// `chmod +x` (a scaffolded `scripts/red` that is not executable fails with "Permission denied" on the very command the `next:` hint tells you to run).
fn make_executable(p: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    let _ = p;
    Ok(())
}

fn write(dir: &Path, rel: &str, text: &str, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    if p.exists() {
        return Err(format!("{} already exists; new-game never overwrites (pick an empty directory)", p.display()));
    }
    std::fs::write(&p, text).map_err(|e| format!("{}: {e}", p.display()))?;
    out.push(p);
    Ok(())
}

/// The 2D starter: the smallest game that has every part a real one has (a player, things to collect, a HUD, a sound, an end with a restart, a saved best, a scripted
/// playthrough and a browser check). `{{ID}}` and `{{TITLE}}` are filled in.
pub const STARTER_2D: &str = r##"{
  "game2d": 1,
  "id": "{{ID}}",
  "title": "{{TITLE}}",
  "description": "Collect every gem before the clock runs out.",
  "capabilities": { "presentation": "2d", "platforms": ["web"], "networking": "offline", "input": ["keyboard", "mouse", "touch"], "persistence": ["progress"] },
  "view": { "width": 320, "height": 180, "background": "#16202e" },

  "sounds": { "ding": { "seconds": 0.2, "level": 0.5, "layers": [{ "sine": 880, "decay": 16 }, { "sine": 1320, "decay": 20, "delay": 0.05, "gain": 0.7 }] } },

  "vars": { "gems_left": 5, "timeleft": 20, "best": 0, "gems_total": 0 },
  "persist": ["best", "gems_total"],

  "prefabs": {
    "player": { "tag": "player", "shape": { "rect": [10, 10], "color": "#ffd166" }, "layer": 2, "clamp": true, "move": { "keys": { "mode": "topdown", "speed": 90 } } },
    "gem": { "tag": "gem", "shape": { "circle": 5, "color": "#5cf2ff" }, "emit": { "rate": 4, "life": [0.3, 0.6], "speed": [5, 12], "angle": [240, 300], "color": "#bff8ff" } }
  },

  "scene": [
    { "prefab": "player", "at": [160, 90], "id": "p" },
    { "prefab": "gem", "at": [40, 40] }, { "prefab": "gem", "at": [280, 40] }, { "prefab": "gem", "at": [40, 150] },
    { "prefab": "gem", "at": [280, 150] }, { "prefab": "gem", "at": [160, 30] }
  ],

  "ui": [
    { "text": "GEMS LEFT {count_gem}", "at": [6, 6], "color": "#5cf2ff" },
    { "text": "TIME {timeleft:2}", "at": [314, 6], "align": "right" },
    { "text": "BEST {best}", "at": [160, 6], "align": "center", "color": "#9fb3d9", "show": "best > 0" },
    { "text": "ALL TIME {gems_total}", "at": [314, 172], "align": "right", "color": "#9fb3d9" },
    { "text": "YOU WIN!", "at": [160, 70], "scale": 2, "align": "center", "color": "#8cff9b", "show": "ended == 1" },
    { "text": "TIME UP", "at": [160, 70], "scale": 2, "align": "center", "color": "#ff7a8c", "show": "ended == 2" },
    { "button": { "id": "again", "label": "PLAY AGAIN (ENTER)", "at": [100, 100], "size": [120, 14], "key": "Enter", "do": [{ "restart": true }] }, "show": "ended" }
  ],

  "rules": [
    { "id": "collect", "when": { "touch": ["player", "gem"] }, "do": [
      { "add": ["gems_left", -1] }, { "add": ["gems_total", 1] }, { "play": "ding" }, { "burst": { "at": "other", "n": 10, "color": "#5cf2ff", "speed": [20, 60], "life": [0.2, 0.5] } }, { "destroy": "other" }] },
    { "id": "clock", "when": { "every": 1 }, "do": [{ "add": ["timeleft", -1] }] },
    { "id": "win", "when": { "every": 0.05 }, "if": "count_gem == 0", "do": [{ "end": "win" }] },
    { "id": "lose", "when": { "every": 0.05 }, "if": "timeleft <= 0 && count_gem > 0", "do": [{ "end": "lose" }] },
    { "id": "best", "when": { "end": "win" }, "if": "timeleft > best", "do": [{ "set": ["best", "timeleft"] }] }
  ],

  "checks": {
    "scenarios": [
      { "name": "walking over every gem wins", "max_seconds": 30, "smoke": true,
        "script": [{ "approach": "gem", "seconds": 15 }],
        "expect": [{ "ended": "win" }, { "count": "gem", "eq": 0 }, { "sound": "ding", "min": 5 }, { "var": "best", "gt": 0 }, { "var": "gems_total", "gte": 5 }] },
      { "name": "standing still runs out the clock", "max_seconds": 40,
        "script": [{ "wait_until": { "ended": "lose" }, "timeout": 30 }],
        "expect": [{ "ended": "lose" }, { "count": "gem", "gt": 0 }, { "var": "best", "eq": 0 }] }
    ],
    "browser": [
      { "name": "taking a gem is remembered after a reload", "keys": ["ArrowUp"], "ms": 900, "changes": ["gems_total"], "persists": ["gems_total"] },
      { "name": "the arrow keys move the player", "keys": ["ArrowRight"], "ms": 400, "changes": ["p_x"] }
    ]
  }
}
"##;

const CLAUDE_MD_2D: &str = r#"# {{NAME}}

A Red Engine 2 **2D browser game**: the whole game is `{{ID}}.game2d.json`. **The engine is not in this repo**: `game.json` pins it, and `scripts/red` fetches and builds that version on first use.
Never copy engine source here.

## The loop
```bash
scripts/red describe web                         # the whole browser workflow on one page: 2d vs hybrid vs 3d, the loop, publishing, the evidence, the limits
scripts/red web status {{ID}}.game2d.json        # where this game stands and the exact next command (add --json for a program)
scripts/red describe 2d                          # the file format on one page (read it once; do not open any source)
scripts/red recipe                               # verified mechanics to copy (a key and a door, a countdown, checkpoints, a spawner, a whole collect-survive-escape game)
scripts/red validate {{ID}}.game2d.json          # well formed? every sprite/sound/tag/variable name resolves? (errors say the fix)
scripts/red sim {{ID}}.game2d.json [--every 5]   # the scripted playthroughs, with the variables every 5 s when a balance is off
scripts/red verify {{ID}}.game2d.json            # simulation + render + audio waveform; exit 1 on any failure
scripts/red frame {{ID}}.game2d.json out/look.png --t 8 --size 1280x720   # LOOK at it (the same renderer the browser uses)
scripts/red web verify {{ID}}.game2d.json        # build the WebAssembly package and run it in a real headless browser (`scripts/red web setup-browser` once)
scripts/red publish {{ID}}.game2d.json           # the pipeline to a site; it says exactly which stage failed, never invents a URL, and writes out/publish/{{ID}}/publication.json (the pieces of evidence, each on its own)
```
- Updating the game later: edit the JSON, run `scripts/red web status {{ID}}.game2d.json`, run the loop again. A passing run is never a claim that a person played it.
- Edit the JSON only. A scenario that asserts nothing is refused; write the playthrough and its `expect` first, then the rules.
- `validate`/`verify` prove the rules, the picture and the sound waveform. They do not prove it is fun or that it sounds right: play it (`scripts/red web serve out/web/{{ID}}`).
- Record progress: `scripts/red status --note "what changed" --section done|now|next`.
"#;

const CI_YML_2D: &str = r#"name: check
on: [push, pull_request]
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: wasm32-unknown-unknown
      - run: RED_HEADLESS=1 bash scripts/red verify {{ID}}.game2d.json
"#;

fn title_case(name: &str) -> String {
    name.split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| w.chars().next().map(|c| c.to_uppercase().collect::<String>() + &w[1..]).unwrap_or_default())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The id a game gets from a project name: lowercase, hyphens.
pub fn game_id(name: &str) -> String {
    name.to_lowercase().replace('_', "-")
}

fn scaffold_2d(dir: &Path, name: &str, engine: &EngineRef) -> Result<Vec<PathBuf>, String> {
    let id = game_id(name);
    let mut out = Vec::new();
    let engine_json = match (&engine.path, &engine.git) {
        (Some(p), _) => format!("{{ \"path\": \"{}\" }}", p.replace('\\', "/")),
        (None, g) => format!(
            "{{ \"git\": \"{}\", \"ref\": \"{}\" }}",
            g.clone().unwrap_or_else(|| "https://github.com/kevstermcgee/RedEngine.git".into()),
            engine.git_ref.clone().unwrap_or_else(|| "master".into())
        ),
    };
    write(
        dir,
        "game.json",
        &format!("{{\n  \"game\": 1,\n  \"name\": \"{name}\",\n  \"engine\": {engine_json},\n  \"blueprints\": [],\n  \"maps\": []\n}}\n"),
        &mut out,
    )?;
    let game = STARTER_2D.replace("{{ID}}", &id).replace("{{TITLE}}", &title_case(name));
    write(dir, &format!("{id}.game2d.json"), &game, &mut out)?;
    write(dir, "CLAUDE.md", &CLAUDE_MD_2D.replace("{{NAME}}", name).replace("{{ID}}", &id), &mut out)?;
    write(dir, "scripts/red", RED_SH, &mut out)?;
    make_executable(&dir.join("scripts/red"))?;
    write(dir, "scripts/red.ps1", RED_PS1, &mut out)?;
    write(dir, ".github/workflows/check.yml", &CI_YML_2D.replace("{{ID}}", &id), &mut out)?;
    write(dir, ".gitignore", ".red/\nout/\ntarget/\n", &mut out)?;
    write(dir, ".gitattributes", "* text=auto eol=lf\n*.png binary\n", &mut out)?;
    out.push(super::status::init(dir)?);
    Ok(out)
}

/// Creates a game project in `dir` (which may exist but must not contain any of the files). Returns the files written.
pub fn scaffold(dir: &Path, name: &str, engine: &EngineRef) -> Result<Vec<PathBuf>, String> {
    scaffold_kind(dir, name, engine, Kind::Walk)
}

/// [`scaffold`] for a chosen [`Kind`] of game.
pub fn scaffold_kind(dir: &Path, name: &str, engine: &EngineRef, kind: Kind) -> Result<Vec<PathBuf>, String> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(format!("--name '{name}' must be letters, digits, _ or - (it becomes the blueprint and map name)"));
    }
    if kind == Kind::TwoD {
        return scaffold_2d(dir, name, engine);
    }
    let mut out = Vec::new();
    let engine_json = match (&engine.path, &engine.git) {
        (Some(p), _) => format!("{{ \"path\": \"{}\" }}", p.replace('\\', "/")),
        (None, g) => format!(
            "{{ \"git\": \"{}\", \"ref\": \"{}\" }}",
            g.clone().unwrap_or_else(|| "https://github.com/kevstermcgee/RedEngine.git".into()),
            engine.git_ref.clone().unwrap_or_else(|| "master".into())
        ),
    };
    let race = kind == Kind::Race;
    let blueprints = if race { "[]" } else { "[\"blueprints/main.blueprint.json\"]" };
    let group = if race { "race" } else { "duel" };
    write(
        dir,
        "game.json",
        &format!(
            "{{\n  \"game\": 1,\n  \"name\": \"{name}\",\n  \"engine\": {engine_json},\n  \"capabilities\": {{ \"presentation\": \"3d\", \"platforms\": [\"windows\", \"linux\"], \"networking\": \"authoritative\", \"input\": [\"keyboard\", \"mouse\", \"gamepad\"] }},\n  \"blueprints\": {blueprints},\n  \"maps\": [\"maps/main.json\"],\n  \"server\": {{ \"map\": \"maps/main.json\", \"port\": 27015, \"spawn_group\": \"{group}\" }}\n}}\n"
        ),
        &mut out,
    )?;
    write(dir, "assets/gameplay.json", "[]\n", &mut out)?;
    write(dir, "assets/README.md", LOCAL_ASSETS_README, &mut out)?;
    if race {
        let scene = super::racetrack::build(&super::racetrack::TrackSpec::default())?;
        write(dir, "maps/main.json", &serde_json::to_string(&scene).map_err(|e| e.to_string())?, &mut out)?;
        write(dir, "CLAUDE.md", &RACE_CLAUDE_MD.replace("{{NAME}}", name), &mut out)?;
    } else {
        let mut blueprint: serde_json::Value = serde_json::from_str(&super::blueprint::example().replace("three_rooms", name))
            .map_err(|e| format!("internal starter blueprint is invalid: {e}"))?;
        blueprint["prefab_files"] = serde_json::json!(["../assets/gameplay.json"]);
        // New games start silent: music is something a game asks for (`"music": true`), never a default.
        blueprint["scene"]["music"] = serde_json::json!(false);
        let blueprint = serde_json::to_string_pretty(&blueprint).map_err(|e| e.to_string())? + "\n";
        write(dir, "blueprints/main.blueprint.json", &blueprint, &mut out)?;
        write(dir, "CLAUDE.md", &CLAUDE_MD.replace("{{NAME}}", name), &mut out)?;
    }
    write(dir, "deploy/install.sh", HOST_SH, &mut out)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join("deploy/install.sh");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    write(dir, "scripts/red", RED_SH, &mut out)?;
    make_executable(&dir.join("scripts/red"))?;
    write(dir, "scripts/red.ps1", RED_PS1, &mut out)?;
    write(dir, ".github/workflows/check.yml", CI_YML, &mut out)?;
    write(dir, ".gitignore", ".red/\nout/\ntarget/\n", &mut out)?;
    write(dir, ".gitattributes", "* text=auto eol=lf\n*.png binary\n", &mut out)?;
    // The map the blueprint builds, so the project is green (and playable) from the first commit.
    let cfg = super::game::load(dir).map_err(|e| e.join("; "))?;
    let built = super::game::build_all(&cfg);
    if let Some(l) = built.iter().find(|l| l.failed) {
        return Err(format!("the starter blueprint did not build: {}", l.text));
    }
    if !race {
        out.push(dir.join("maps/main.json"));
    }
    out.push(super::status::init(dir)?);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The starter a fresh author gets is itself a verified game (so it can never ship broken), and a project made from it validates, verifies, and names its own game.
    #[test]
    fn the_2d_starter_is_a_green_project() {
        let dir = std::env::temp_dir().join(format!("re2_newgame2d_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let files = scaffold_kind(&dir, "gem-grab", &EngineRef { path: Some("../engine".into()), ..Default::default() }, Kind::TwoD).unwrap();
        assert!(files.iter().any(|f| f.ends_with("gem-grab.game2d.json")), "{files:?}");
        let game = dir.join("gem-grab.game2d.json");
        let r = super::super::game2d::validate(&game);
        assert!(r.ok, "{}", r.text);
        let v = super::super::game2d::verify(&game, None);
        assert!(v.ok, "{}", v.text);
        assert!(v.text.contains("scenario `walking over every gem wins`") && v.text.contains("save round trip"), "{}", v.text);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert!(
                std::fs::metadata(dir.join("scripts/red")).unwrap().permissions().mode() & 0o111 != 0,
                "scripts/red must be executable: the next: hint runs it directly"
            );
        }
        let guide = std::fs::read_to_string(dir.join("CLAUDE.md")).unwrap();
        assert!(guide.contains("scripts/red verify gem-grab.game2d.json") && guide.contains("describe 2d"), "{guide}");
        assert!(super::super::game::load(&dir).is_ok(), "the project has a loadable game.json (scripts/red reads the engine pin from it)");
        assert!(scaffold_kind(&dir, "gem-grab", &EngineRef::default(), Kind::TwoD).is_err(), "never overwrites");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scaffold_writes_a_green_project_and_refuses_to_overwrite() {
        let dir = std::env::temp_dir().join(format!("re2_newgame_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let files = scaffold(&dir, "cheese", &EngineRef { path: Some("..\\engine".into()), ..Default::default() }).unwrap();
        for f in [
            "game.json",
            "CLAUDE.md",
            "STATUS.md",
            "scripts/red",
            "scripts/red.ps1",
            "maps/main.json",
            "blueprints/main.blueprint.json",
            "assets/gameplay.json",
            "assets/README.md",
        ] {
            assert!(dir.join(f).exists(), "missing {f}: {files:?}");
        }
        assert!(std::fs::read_to_string(dir.join("blueprints/main.blueprint.json")).unwrap().contains("../assets/gameplay.json"));
        let game = std::fs::read_to_string(dir.join("game.json")).unwrap();
        assert!(game.contains("\"path\": \"../engine\""), "backslashes in a path are normalised: {game}");
        let guide = std::fs::read_to_string(dir.join("CLAUDE.md")).unwrap();
        assert!(guide.starts_with("# cheese"));
        assert!(guide.contains("scripts/red play-local"), "every new game must document direct local single-player");
        assert!(scaffold(&dir, "cheese", &EngineRef::default()).is_err(), "second run must not clobber");
        assert!(scaffold(&std::env::temp_dir().join("re2_newgame_bad"), "bad name!", &EngineRef::default()).is_err());
    }

    #[test]
    fn a_race_project_is_green_raceable_and_has_the_hosting_script() {
        let dir = std::env::temp_dir().join(format!("re2_newgame_race_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        scaffold_kind(&dir, "kart-town", &EngineRef { path: Some("../RedEngine".into()), ..Default::default() }, Kind::Race).unwrap();
        for f in ["game.json", "CLAUDE.md", "STATUS.md", "maps/main.json", "deploy/install.sh", "scripts/red"] {
            assert!(dir.join(f).exists(), "missing {f}");
        }
        assert!(!dir.join("blueprints").exists(), "a race is not built from a blueprint");
        let game = std::fs::read_to_string(dir.join("game.json")).unwrap();
        assert!(game.contains("\"spawn_group\": \"race\"") && game.contains("\"blueprints\": []"), "{game}");
        let cfg = super::super::game::load(&dir).unwrap_or_else(|e| panic!("{e:?}"));
        let report = super::super::game::check(&cfg, false);
        assert_eq!(report.failed(), 0, "{}", report.render());
        assert!(report.render().contains("karts: all 8 drivers"), "{}", report.render());
        let race = super::super::racetest::run(&dir.join("maps/main.json"), 8, 0.8, 300.0, &[]).unwrap();
        assert!(race.all_finished(), "{}", super::super::racetest::render(&race));
        let guide = std::fs::read_to_string(dir.join("CLAUDE.md")).unwrap();
        assert!(guide.starts_with("# kart-town") && guide.contains("race-track") && guide.contains("game publish") && guide.contains("deploy/install.sh"));
        let script = std::fs::read_to_string(dir.join("deploy/install.sh")).unwrap();
        assert!(script.contains("game.json") && !script.contains("great-outdoors"), "the hosting script is generic");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_wrapper_scripts_only_use_commands_the_cli_has() {
        // Every subcommand the wrapper forwards to `game` must exist (the CLI's own `describe commands` is the source of truth).
        for sub in ["check", "build-all", "info", "play-local", "serve", "play"] {
            assert!(RED_SH.contains(sub) && RED_PS1.contains(sub), "{sub}");
        }
    }
}
