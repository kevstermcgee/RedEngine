# scripts/dev.ps1 — the Windows-native twin of scripts/dev (same commands). Run from any directory:
#   powershell -File scripts\dev.ps1 doctor | fast | test [filter] | build [--headless] | red <args> | verify <scene> [...]
#                                    walk <scene> [...] | server <scene> [...] | ci | status [...]
# Env: RED_PROFILE = debug | release (default debug). (The bash twin also takes RED_TIMEOUT; PowerShell relies on the tool's own timeout.)
param([Parameter(Position = 0)][string]$Cmd = 'help', [Parameter(ValueFromRemainingArguments = $true)][string[]]$Rest)
$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root
$env:CARGO_TERM_COLOR = 'never'
if (-not $env:RUST_BACKTRACE) { $env:RUST_BACKTRACE = '1' }
$cargoBin = Join-Path $HOME '.cargo\bin'
if (($env:PATH -split ';') -notcontains $cargoBin -and (Test-Path $cargoBin)) { $env:PATH = "$cargoBin;$env:PATH" }
$Profile_ = if ($env:RED_PROFILE) { $env:RED_PROFILE } else { 'debug' }
$PFlag = if ($Profile_ -eq 'release') { @('--release') } else { @() }

function Need-Cargo {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { Write-Error 'dev: cargo not found. Install Rust from https://rustup.rs'; exit 127 }
}
function Exe([string]$Name) { Join-Path $Root "target\$Profile_\$Name.exe" }
function Cli {
    Need-Cargo
    & cargo build @PFlag --bin red_engine2 --quiet
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    return (Exe 'red_engine2')
}

switch ($Cmd) {
    'doctor' {
        "repo      $Root"
        if (Get-Command cargo -ErrorAction SilentlyContinue) { "cargo     $(cargo --version)"; "rustc     $(rustc --version)" } else { 'cargo     NOT FOUND (install Rust: https://rustup.rs)' }
        foreach ($b in 'red_engine2', 'red_server', 'red_bot', 're2') { if (Test-Path (Exe $b)) { "built     $b ($Profile_)" } else { "not built $b" } }
        if (Get-Command git -ErrorAction SilentlyContinue) { "git       $(git rev-parse --abbrev-ref HEAD) @ $(git rev-parse --short HEAD), $((git status --short | Measure-Object).Count) changed file(s)" }
        if (Test-Path STATUS.md) { 'status    STATUS.md exists: run scripts\dev.ps1 status' } else { 'status    no STATUS.md yet: scripts\dev.ps1 status --init' }
    }
    'fast' { Need-Cargo; & cargo test @PFlag --lib @Rest; exit $LASTEXITCODE }
    'test' { Need-Cargo; & cargo test @PFlag @Rest; exit $LASTEXITCODE }
    'build' {
        Need-Cargo
        if ($Rest -contains '--headless') { & cargo build @PFlag --no-default-features --bin red_engine2 --bin red_server --bin red_bot }
        else { & cargo build @PFlag --bin red_engine2 --bin red_server --bin red_bot --bin re2 }
        exit $LASTEXITCODE
    }
    'red' { $e = Cli; & $e @Rest; exit $LASTEXITCODE }
    'verify' { $e = Cli; & $e verify @Rest; exit $LASTEXITCODE }
    'walk' { $e = Cli; & $e walk @Rest; exit $LASTEXITCODE }
    'status' { $e = Cli; & $e status @Rest; exit $LASTEXITCODE }
    'server' {
        Need-Cargo
        & cargo build @PFlag --no-default-features --bin red_server --quiet
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        & (Exe 'red_server') --map @Rest; exit $LASTEXITCODE
    }
    'ci' { if (Get-Command bash -ErrorAction SilentlyContinue) { & bash scripts/ci.sh; exit $LASTEXITCODE } else { Write-Error 'ci needs bash (Git for Windows ships one)'; exit 2 } }
    default { Get-Content $PSCommandPath -TotalCount 5 | ForEach-Object { $_ -replace '^# ?', '' } }
}
