# Lobby / round / rematch demo and proof: a keyed lobby server, one scripted bot (auto-ready) and ONE REAL graphical client whose
# lobby screen, countdown, HUD and results screen are screenshotted, with the Ready / rematch keys pressed for it.
#
#   powershell -File scripts\lobby_demo.ps1                 # uses the newer of target\release and target\debug
#   -Port N  -Out out\lobby_   -Keep (leave everything running)
#
# Screenshots: out\lobby_1_lobby.png, _2_countdown.png, _3_playing.png, _4_results.png, _5_rematch.png. The server log is printed at the end:
# it shows the join key handshake, the countdown, the round, why it ended, and the rematch.
param(
    [int]$Port = 27931,
    [string]$Out = "out\lobby_",
    [switch]$Keep
)
$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public class Win { [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra); }
"@

# Whichever build is newer (a stale release build must not shadow a fresh debug one).
$cands = @("target\release", "target\debug") | Where-Object { Test-Path (Join-Path $repo "$_\red_server.exe") }
$bin = $cands | Sort-Object { (Get-Item (Join-Path $repo "$_\red_server.exe")).LastWriteTime } -Descending | Select-Object -First 1
foreach ($f in "red_server.exe", "red_bot.exe", "re2.exe") { if (-not (Test-Path (Join-Path $repo "$bin\$f"))) { throw "missing $bin\$f - cargo build --bins" } }
New-Item -ItemType Directory -Force out | Out-Null

function Shot($name) {
    $bd = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bmp = New-Object System.Drawing.Bitmap $bd.Width, $bd.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($bd.Location, [System.Drawing.Point]::Empty, $bd.Size)
    $bmp.Save((Join-Path $repo "$Out$name.png")); $g.Dispose(); $bmp.Dispose()
}
function Press($vk) {
    [Win]::keybd_event($vk, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 80; [Win]::keybd_event($vk, 0, 2, [UIntPtr]::Zero)
}

$srvArgs = @("--port", $Port, "--bind", "127.0.0.1", "--map", "examples\test_lab.json", "--spawn-group", "duel", "--stats-secs", "0", "--key", "demo-key",
             "--min-players", "2", "--countdown-secs", "3", "--round-secs", "6", "--results-secs", "60")
$srv = Start-Process -FilePath (Join-Path $repo "$bin\red_server.exe") -ArgumentList $srvArgs -WorkingDirectory $repo -RedirectStandardOutput (Join-Path $repo "out\lobby_server.log") -RedirectStandardError (Join-Path $repo "out\lobby_server.err") -WindowStyle Hidden -PassThru
Start-Sleep -Seconds 2
$botArgs = @("--server", "127.0.0.1:$Port", "--key", "demo-key", "--name", "Bo", "--ready", "--behavior", "circle:60", "--duration", "120", "--report-every", "60")
$bot = Start-Process -FilePath (Join-Path $repo "$bin\red_bot.exe") -ArgumentList $botArgs -WorkingDirectory $repo -RedirectStandardOutput (Join-Path $repo "out\lobby_bot.log") -WindowStyle Hidden -PassThru

$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = Join-Path $repo "$bin\re2.exe"
$psi.Arguments = "examples\test_lab.json --as human --connect 127.0.0.1:$Port --key demo-key --name Kev"
$psi.WorkingDirectory = $repo; $psi.UseShellExecute = $false
$psi.EnvironmentVariables["RE2_WINDOW"] = "0,0,980,640"
$client = [System.Diagnostics.Process]::Start($psi)
Start-Sleep -Seconds 6
[Win]::SetForegroundWindow($client.MainWindowHandle) | Out-Null
Start-Sleep -Milliseconds 500
Shot "1_lobby"
Write-Output "lobby: window title = $($client.MainWindowTitle)"
Press 0x52   # R = Ready: with Bo already ready, the countdown starts
Start-Sleep -Milliseconds 1200
Shot "2_countdown"
Start-Sleep -Seconds 4
Shot "3_playing"
Start-Sleep -Seconds 6
Shot "4_results"
Press 0x52   # R again = rematch vote; Bo (auto-ready) has voted too, so the next countdown starts at once
Start-Sleep -Milliseconds 1500
Shot "5_rematch"
Write-Output "--- server log"
Get-Content (Join-Path $repo "out\lobby_server.log") | ForEach-Object { "  $_" }
if (-not $Keep) {
    foreach ($p in $client, $bot, $srv) { try { if (-not $p.HasExited) { $p.Kill() } } catch {} }
}
