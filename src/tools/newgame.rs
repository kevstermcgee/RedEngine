//! `red_engine2 new-game <dir>`: scaffold a game project that *uses* Red instead of forking it (see [`super::game`]).
//!
//! The scaffold is deliberately tiny and already green: a `game.json`, one blueprint and the map it builds, a
//! `CLAUDE.md` that teaches the loop in about thirty lines, a `STATUS.md` handoff, the `scripts/red` wrapper that
//! finds and builds the pinned engine (bash and PowerShell), and a CI workflow that runs `red check` headless.

use super::game::EngineRef;
use std::path::{Path, PathBuf};

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

## Multiplayer
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
#   scripts/red doctor | check | build-all | info | serve | play [HOST:PORT] | status ... | <any red_engine2 command>
# Env: RED_ENGINE=/path/to/checkout (override), RED_UPDATE=1 (git fetch the pinned ref), RED_HEADLESS=1 (no graphics crates:
# CLI + server only, ideal for CI and containers), RED_PROFILE=debug|release (default debug).
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

cmd="${1:-help}"; [ $# -gt 0 ] && shift
if [ "$cmd" = "doctor" ]; then
  echo "project  $ROOT"; echo "engine   $ENGINE ($(git -C "$ENGINE" rev-parse --short HEAD 2>/dev/null || echo 'not a git checkout'))"
  command -v cargo >/dev/null 2>&1 && echo "cargo    $(cargo --version)" || echo "cargo    NOT FOUND (install Rust: https://rustup.rs)"
  for b in red_engine2 red_server re2; do [ -f "$(exe $b)" ] && echo "built    $b" || echo "missing  $b (built on first use)"; done
  exit 0
fi
command -v cargo >/dev/null 2>&1 || { echo "red: cargo not found (install Rust: https://rustup.rs)" >&2; exit 127; }

BINS="--bin red_engine2 --bin red_server"; FEATURES=""
if [ "${RED_HEADLESS:-0}" = "1" ]; then FEATURES="--no-default-features"; else [ "$cmd" = "play" ] && BINS="$BINS --bin re2"; fi
[ -f "$(exe red_engine2)" ] || echo "red: building the engine (first time takes a few minutes; later runs are instant) ..." >&2
cargo build --quiet $PFLAG $FEATURES --manifest-path "$ENGINE/Cargo.toml" $BINS

CLI="$(exe red_engine2)"
case "$cmd" in
  check|build-all|info|serve) exec "$CLI" game "$cmd" "$@" ;;
  play) exec "$CLI" game play "$@" ;;
  help|-h|--help) sed -n '2,5p' "$0" ;;
  *) exec "$CLI" "$cmd" "$@" ;;
esac
"##;

const RED_PS1: &str = r##"# scripts/red.ps1: the Windows-native twin of scripts/red (same commands and env vars).
#   powershell -File scripts\red.ps1 doctor | check | build-all | info | serve | play [HOST:PORT] | status ... | <any red_engine2 command>
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

if ($Cmd -eq 'doctor') {
    "project  $Root"; "engine   $Engine"
    if (Get-Command cargo -ErrorAction SilentlyContinue) { "cargo    $(cargo --version)" } else { 'cargo    NOT FOUND (install Rust: https://rustup.rs)' }
    foreach ($b in 'red_engine2', 'red_server', 're2') { if (Test-Path (Exe $b)) { "built    $b" } else { "missing  $b (built on first use)" } }
    exit 0
}
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { Write-Error 'red: cargo not found (install Rust: https://rustup.rs)'; exit 127 }
$bins = @('--bin', 'red_engine2', '--bin', 'red_server'); $features = @()
if ($env:RED_HEADLESS -eq '1') { $features = @('--no-default-features') } elseif ($Cmd -eq 'play') { $bins += @('--bin', 're2') }
if (-not (Test-Path (Exe 'red_engine2'))) { Write-Host 'red: building the engine (first time takes a few minutes; later runs are instant) ...' }
& cargo build --quiet @PFlag @features --manifest-path (Join-Path $Engine 'Cargo.toml') @bins
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$cli = Exe 'red_engine2'
switch ($Cmd) {
    { $_ -in 'check', 'build-all', 'info', 'serve', 'play' } { & $cli game $Cmd @Rest }
    'help' { Get-Content $PSCommandPath -TotalCount 2 | ForEach-Object { $_ -replace '^# ?', '' } }
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

/// Creates a game project in `dir` (which may exist but must not contain any of the files). Returns the files written.
pub fn scaffold(dir: &Path, name: &str, engine: &EngineRef) -> Result<Vec<PathBuf>, String> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(format!("--name '{name}' must be letters, digits, _ or - (it becomes the blueprint and map name)"));
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
    write(
        dir,
        "game.json",
        &format!(
            "{{\n  \"game\": 1,\n  \"name\": \"{name}\",\n  \"engine\": {engine_json},\n  \"blueprints\": [\"blueprints/main.blueprint.json\"],\n  \"maps\": [\"maps/main.json\"],\n  \"server\": {{ \"map\": \"maps/main.json\", \"port\": 27015, \"spawn_group\": \"duel\" }}\n}}\n"
        ),
        &mut out,
    )?;
    let mut blueprint: serde_json::Value =
        serde_json::from_str(&super::blueprint::example().replace("three_rooms", name)).map_err(|e| format!("internal starter blueprint is invalid: {e}"))?;
    blueprint["prefab_files"] = serde_json::json!(["../assets/gameplay.json"]);
    let blueprint = serde_json::to_string_pretty(&blueprint).map_err(|e| e.to_string())? + "\n";
    write(dir, "assets/gameplay.json", "[]\n", &mut out)?;
    write(dir, "assets/README.md", LOCAL_ASSETS_README, &mut out)?;
    write(dir, "blueprints/main.blueprint.json", &blueprint, &mut out)?;
    write(dir, "CLAUDE.md", &CLAUDE_MD.replace("{{NAME}}", name), &mut out)?;
    write(dir, "scripts/red", RED_SH, &mut out)?;
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
    out.push(dir.join("maps/main.json"));
    out.push(super::status::init(dir)?);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(std::fs::read_to_string(dir.join("CLAUDE.md")).unwrap().starts_with("# cheese"));
        assert!(scaffold(&dir, "cheese", &EngineRef::default()).is_err(), "second run must not clobber");
        assert!(scaffold(&std::env::temp_dir().join("re2_newgame_bad"), "bad name!", &EngineRef::default()).is_err());
    }

    #[test]
    fn the_wrapper_scripts_only_use_commands_the_cli_has() {
        // Every subcommand the wrapper forwards to `game` must exist (the CLI's own `describe commands` is the source of truth).
        for sub in ["check", "build-all", "info", "serve", "play"] {
            assert!(RED_SH.contains(sub) && RED_PS1.contains(sub), "{sub}");
        }
    }
}
