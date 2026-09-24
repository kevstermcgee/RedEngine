# Multiplayer demo / proof: one authoritative server process + two REAL graphical clients, side by side.
#
#   powershell -File scripts\multiplayer_demo.ps1                 # duel scenario (players see each other)
#   powershell -File scripts\multiplayer_demo.ps1 -Scenario props # one player pushes a barrel, the other watches
#   -Build   use the release build in target\release (default: target\fast if present, else release)
#
# What it does: starts red_server (headless), starts client A (stands still) and client B (walks in a circle
# by itself, RE2_AUTOWALK), screenshots the screen a few times, prints each window title (which carries the
# live connection status: player id, ping, number of other players), then kills and restarts the SERVER to show
# both clients reconnecting on their own, then kills client B to show A noticing. Screenshots go to out\.
param(
    [ValidateSet("duel", "props")][string]$Scenario = "duel",
    [int]$Port = 27015,
    [string]$Out = "out\mp_",
    [ValidateSet("human", "rat")][string]$BWho = "human",   # what client B plays as
    [int]$Turn = 90,                                        # B's turn rate while walking, degrees/second
    [switch]$Keep      # leave everything running at the end
)
$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo
Add-Type -AssemblyName System.Windows.Forms, System.Drawing

$bin = if (Test-Path "target\fast\release\re2.exe") { "target\fast\release" } else { "target\release" }
$server = Join-Path $repo "$bin\red_server.exe"
$client = Join-Path $repo "$bin\re2.exe"
foreach ($f in $server, $client) { if (-not (Test-Path $f)) { throw "missing $f - cargo build --release --bin re2 --bin red_server" } }

function Shot($name) {
    $bd = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bmp = New-Object System.Drawing.Bitmap $bd.Width, $bd.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($bd.Location, [System.Drawing.Point]::Empty, $bd.Size)
    $bmp.Save((Join-Path $repo "$Out$name.png")); $g.Dispose(); $bmp.Dispose()
}
function Titles($label) {
    Write-Output "--- window titles: $label"
    Get-Process re2 -ErrorAction SilentlyContinue | Sort-Object Id | ForEach-Object { "  pid $($_.Id): $($_.MainWindowTitle)" }
}
function StartServer() {
    $args = @("--port", $Port, "--bind", "127.0.0.1", "--spawn-group", $Scenario, "--stats-secs", "2", "--map", "examples\test_lab.json")
    Start-Process -FilePath $server -ArgumentList $args -WorkingDirectory $repo -RedirectStandardOutput (Join-Path $repo "out\mp_server.log") -RedirectStandardError (Join-Path $repo "out\mp_server.err") -WindowStyle Hidden -PassThru
}
function StartClient($who, $autowalk, $x) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $client
    $psi.Arguments = "examples\test_lab.json --as $who --connect 127.0.0.1:$Port"
    $psi.WorkingDirectory = $repo; $psi.UseShellExecute = $false
    $psi.EnvironmentVariables["RE2_WINDOW"] = "$x,0,500,640"
    if ($autowalk) { $psi.EnvironmentVariables["RE2_AUTOWALK"] = $autowalk }
    [System.Diagnostics.Process]::Start($psi)
}

New-Item -ItemType Directory -Force out | Out-Null
$srv = StartServer
Start-Sleep -Seconds 2
Write-Output "server started (pid $($srv.Id)):"; Get-Content (Join-Path $repo "out\mp_server.log") | Select-Object -First 3 | ForEach-Object { "  $_" }

# Client A on the left (stands still), client B on the right (walks by itself).
# (props scenario: A walks straight into the barrel in front of it, B stands and watches.)
$aWalk = if ($Scenario -eq "props") { "forward" } else { $null }
$bWalk = if ($Scenario -eq "props") { $null } else { "circle:$Turn" }
$bWho2 = if ($Scenario -eq "props") { "human" } else { $BWho }
if ($Scenario -eq "props") {
    # B must be watching before A starts pushing: start B first, then A a moment later.
    $b = StartClient $bWho2 $bWalk 512
    Start-Sleep -Seconds 4
    $a = StartClient "human" $aWalk 0
    Start-Sleep -Seconds 2
} else {
    $a = StartClient "human" $aWalk 0
    Start-Sleep -Seconds 4
    $b = StartClient $bWho2 $bWalk 512
    Start-Sleep -Seconds 7
}
Titles "both connected"
Shot "1_both"
Start-Sleep -Milliseconds 350; Shot "2_both_moved"
Start-Sleep -Milliseconds 350; Shot "3_both_moved_more"
Write-Output "--- server log so far"; Get-Content (Join-Path $repo "out\mp_server.log") | Select-Object -Last 6 | ForEach-Object { "  $_" }

# Server dies and comes back: clients must reconnect by themselves.
Write-Output "--- killing the server"
Stop-Process -Id $srv.Id -Force
Start-Sleep -Seconds 4
Titles "server is down"
Shot "4_server_down"
$srv = StartServer
Start-Sleep -Seconds 6
Titles "server restarted"
Shot "5_reconnected"
Write-Output "--- server log after restart"; Get-Content (Join-Path $repo "out\mp_server.log") | Select-Object -Last 6 | ForEach-Object { "  $_" }

# Client B crashes (no goodbye): the server times it out and A notices.
Write-Output "--- killing client B without a goodbye"
Stop-Process -Id $b.Id -Force
Start-Sleep -Seconds 5
Titles "after B died"
Shot "6_b_gone"
Get-Content (Join-Path $repo "out\mp_server.log") | Select-Object -Last 3 | ForEach-Object { "  $_" }

if (-not $Keep) {
    Get-Process re2, red_server -ErrorAction SilentlyContinue | Stop-Process -Force
    Write-Output "all stopped"
}
