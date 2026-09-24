# Play the Red Test Lab with two windows on this PC (both yours to drive), against a real server process.
#
#   powershell -File scripts\play_multiplayer.ps1              # server + 2 clients, side by side
#   powershell -File scripts\play_multiplayer.ps1 -Clients 1   # server + 1 client (join the 2nd from another PC)
#   powershell -File scripts\play_multiplayer.ps1 -Group props  # spawn beside the barrels instead of the duel hall
#
# Click a window to take control of it (Esc releases the mouse). WASD walk, Shift sprint, Space jump, Ctrl crouch,
# mouse look, Q third person. Walk into the barrels/crates in the props room and watch the other window: the
# server, not your window, decides how they move. Close a window to leave; the server keeps running until you
# close the "Red Server" console (or run this script's -Stop).
#
# From another PC on the LAN:   re2 --connect <this PC's IP>:27015 examples\test_lab.json
param(
    [int]$Clients = 2,
    [ValidateSet("duel", "props")][string]$Group = "duel",
    [int]$Port = 27015,
    [switch]$Stop
)
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo
if ($Stop) { Get-Process re2, red_server -ErrorAction SilentlyContinue | Stop-Process -Force; "stopped"; return }

$bin = if (Test-Path "target\release\re2.exe") { "target\release" } else { "target\fast\release" }
$server = Join-Path $repo "$bin\red_server.exe"
$client = Join-Path $repo "$bin\re2.exe"
if (-not (Test-Path $client) -or -not (Test-Path $server)) { throw "Build first: cargo build --release --bin re2 --bin red_server" }

# One server. If something already listens on the port, reuse it.
if (-not (Get-Process red_server -ErrorAction SilentlyContinue)) {
    Start-Process -FilePath $server -ArgumentList @("--port", $Port, "--bind", "0.0.0.0", "--spawn-group", $Group, "--map", "examples\test_lab.json") -WorkingDirectory $repo -WindowStyle Minimized
    Start-Sleep -Seconds 2
}
for ($i = 0; $i -lt $Clients; $i++) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $client
    $psi.Arguments = "examples\test_lab.json --connect 127.0.0.1:$Port"
    $psi.WorkingDirectory = $repo
    $psi.UseShellExecute = $false
    if ($Clients -gt 1) { $psi.EnvironmentVariables["RE2_WINDOW"] = "$($i * 512),0,500,640" }
    [System.Diagnostics.Process]::Start($psi) | Out-Null
    Start-Sleep -Milliseconds 800
}
