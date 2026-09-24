# Creates a Desktop shortcut that launches the Red Test Lab in the real-time game (re2).
# Build first:  cargo build --release --bin re2      Then:  powershell -File scripts\make_shortcut.ps1
$repo = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $repo "target\release\re2.exe"
if (-not (Test-Path $exe)) { throw "Build first: cargo build --release --bin re2 ($exe is missing)" }
$lnk = Join-Path ([Environment]::GetFolderPath("Desktop")) "Red Test Lab.lnk"
$s = (New-Object -ComObject WScript.Shell).CreateShortcut($lnk)
$s.TargetPath = $exe
$s.Arguments = "examples\test_lab.json"      # no --as: the launch menu asks Human or Cheddar the rat
$s.WorkingDirectory = $repo
$s.IconLocation = (Join-Path $PSScriptRoot "test_lab.ico") + ",0"
$s.Description = "Red Engine 2 - Test Lab (WASD walk, mouse look, E pick up, click swing/shoot, wheel switches weapon)"
$s.Save()
Write-Output "created $lnk"

# Second shortcut: a real server + two windows to play the Test Lab against yourself (or a friend on the LAN).
$mp = Join-Path ([Environment]::GetFolderPath("Desktop")) "Red Test Lab (Multiplayer).lnk"
$s2 = (New-Object -ComObject WScript.Shell).CreateShortcut($mp)
$s2.TargetPath = (Get-Command powershell.exe).Source
$s2.Arguments = "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File `"$(Join-Path $PSScriptRoot 'play_multiplayer.ps1')`""
$s2.WorkingDirectory = $repo
$s2.IconLocation = (Join-Path $PSScriptRoot "test_lab.ico") + ",0"
$s2.WindowStyle = 7
$s2.Description = "Red Engine 2 - Test Lab multiplayer: starts a server and two client windows"
$s2.Save()
Write-Output "created $mp"
