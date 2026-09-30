# scripts/dev.ps1 — the Windows-native twin of scripts/dev (same commands). Run from any directory:
#   powershell -File scripts\dev.ps1 doctor | fast | test [filter] | build [--headless] | red <args> | verify <scene> [...]
#                                    walk <scene> [...] | server <scene> [...] | ci | status [...]
#                                    affected|check [--quick|--full] | iterate [--check-only] [--headless] | context <feature|file|words>   (verify only what a
#                                    change can affect; `iterate` = only what changed since HEAD, type-check + focused unit tests, never counts as verification)
#                                    preflight [--fix]   (bookkeeping; always builds a current binary here, the bash twin can skip the two checks that need one)
# Env: RED_PROFILE = debug | release | fast (default debug). (The bash twin also takes RED_TIMEOUT; PowerShell relies on the tool's own timeout.)
param([Parameter(Position = 0)][string]$Cmd = 'help', [Parameter(ValueFromRemainingArguments = $true)][string[]]$Rest)
$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root
$env:CARGO_TERM_COLOR = 'never'
if (-not $env:RUST_BACKTRACE) { $env:RUST_BACKTRACE = '1' }
$cargoBin = Join-Path $HOME '.cargo\bin'
if (($env:PATH -split ';') -notcontains $cargoBin -and (Test-Path $cargoBin)) { $env:PATH = "$cargoBin;$env:PATH" }
$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root 'target' }
$Profile_ = if ($env:RED_PROFILE) { $env:RED_PROFILE } else { 'debug' }
$PFlag = if ($Profile_ -eq 'release') { @('--release') } elseif ($Profile_ -ne 'debug') { @('--profile', $Profile_) } else { @() }   # fast = [profile.fast] in Cargo.toml

function Need-Cargo {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { Write-Error 'dev: cargo not found. Install Rust from https://rustup.rs'; exit 127 }
}
function Exe([string]$Name) { Join-Path $TargetDir "$Profile_\$Name.exe" }
function Needs-Build([string]$Name, [string]$Mode = 'default') {
    $out = Exe $Name
    if ($env:RED_REBUILD -eq '1' -or -not (Test-Path $out)) { return $true }
    $stamp = Join-Path $TargetDir "$Profile_\.red-dev-$Name.mode"
    if (-not (Test-Path $stamp) -or (Get-Content $stamp -Raw) -ne $Mode) { return $true }
    $inputs = @((Join-Path $Root 'Cargo.toml'), (Join-Path $Root 'Cargo.lock'), (Join-Path $Root 'build.rs'))
    $inputs += Get-ChildItem (Join-Path $Root 'src'), (Join-Path $Root 'assets') -Recurse -File -ErrorAction SilentlyContinue | Select-Object -ExpandProperty FullName
    $built = (Get-Item $out).LastWriteTimeUtc
    return $null -ne ($inputs | Where-Object { (Test-Path $_) -and (Get-Item $_).LastWriteTimeUtc -gt $built } | Select-Object -First 1)
}
# The CLI for `affected`/`context`: a stale binary plans identically unless the index or the planner changed, so skip the rebuild.
function Planner {
    $exe = Exe 'red_engine2'
    if ((Test-Path $exe) -and $env:RED_REBUILD -ne '1') {
        $built = (Get-Item $exe).LastWriteTimeUtc
        # Logic files only: the feature index is read from the checkout at run time, so editing it needs no rebuild.
        $inputs = 'src\tools\affected.rs', 'src\tools\features.rs', 'src\tools\context.rs', 'src\tools\symbols.rs', 'src\crypto.rs', 'src\cli\args.rs', 'src\cli\analyze.rs', 'src\cli\info.rs'
        if (-not ($inputs | Where-Object { (Test-Path $_) -and (Get-Item $_).LastWriteTimeUtc -gt $built })) { return $exe }
    }
    return (Cli)
}
function Cli {
    Need-Cargo
    if (Needs-Build 'red_engine2') {
        & cargo build @PFlag --bin red_engine2 --quiet
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        Set-Content -NoNewline -Path (Join-Path $TargetDir "$Profile_\.red-dev-red_engine2.mode") -Value 'default'
    }
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
    { $_ -in 'affected', 'check' } { Need-Cargo; $e = Planner; & $e affected @Rest; exit $LASTEXITCODE }
    'iterate' { Need-Cargo; $e = Planner; & $e affected --partial @Rest; exit $LASTEXITCODE }
    'preflight' { $e = Cli; & $e preflight @Rest; exit $LASTEXITCODE }
    'context' { Need-Cargo; $e = Planner; & $e context @Rest; exit $LASTEXITCODE }
    'server' {
        Need-Cargo
        if (Needs-Build 'red_server' 'headless') {
            & cargo build @PFlag --no-default-features --bin red_server --quiet
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
            Set-Content -NoNewline -Path (Join-Path $TargetDir "$Profile_\.red-dev-red_server.mode") -Value 'headless'
        }
        & (Exe 'red_server') --map @Rest; exit $LASTEXITCODE
    }
    'ci' { if (Get-Command bash -ErrorAction SilentlyContinue) { & bash scripts/ci.sh; exit $LASTEXITCODE } else { Write-Error 'ci needs bash (Git for Windows ships one)'; exit 2 } }
    default { Get-Content $PSCommandPath -TotalCount 5 | ForEach-Object { $_ -replace '^# ?', '' } }
}
