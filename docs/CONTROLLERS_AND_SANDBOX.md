# Controllers and RedEngineSandbox

RedEngineSandbox is a standalone project in RedEngineGames/projects/redengine-sandbox.
The local development copy is a sibling of RedEngine. Start its Play-RedEngineSandbox.cmd,
or run scripts/red.ps1 play-local from the project.

The native gamepad backend is gilrs, included only with the gfx feature.
Windows uses Windows Gaming Input; Linux needs libudev-dev to build the graphics client.
macOS uses the gilrs native backend. Hardware availability and mappings depend on the OS/device;
SDL_GAMECONTROLLERCONFIG can provide an SDL-compatible mapping.

| Action | Controller | Keyboard/mouse |
|---|---|---|
| Move at variable speed | Left stick | WASD |
| Look | Right stick | Mouse |
| Fire / aim | RT / LT | Left / right mouse |
| Jump / crouch | A / hold B | Space / Ctrl |
| Interact / reload | X / Y | E / R |
| Previous / next weapon | LB / RB | Wheel |
| Sprint | Hold left stick click | Shift |
| First/third person | Back / Select | Q |
| Pause / resume | Start | Escape |
| Project maps | D-pad up; Y while paused | M |
| Choose character | D-pad left/right, A | 1–6, arrows + Enter |
| Map selection | D-pad up/down, A; LB/RB pages | Arrows + Enter |
| Cancel map selection | B | Escape |

Button labels use Xbox names; equivalent physical positions are used on other supported pads.
The first connected pad owns input until it disconnects. Keyboard and mouse remain usable.
Radial stick dead zones are 18% movement and 15% look. Look speed is frame-rate independent and
scales with aim magnification. Release controls after reconnecting, returning focus or leaving
a menu: neutral rearming prevents a held trigger from firing on resume.

Online ready/character/leave use A/Y/B. The connect form accepts A to connect, B to return and
D-pad up/down to advance fields; entering arbitrary server text still needs a keyboard.
There is no rumble or in-game remapping UI in this version.

Human, wizard, cowboy, alien and robot share human physics and weapons. Cheddar retains rat physics.
Use humanoid.style in scene JSON for model exhibits, or re2 --as wizard/cowboy/alien/robot to play one.
All six choices appear in the character picker and have distinct replicated identities.
Protocol v8 is required on both client and server (v7 added the analog axes; v8 adds shooter feedback, ADR 0051).

The map browser discovers game.json and loads only declared files within that project.
Travel is local-only and resets the destination map's props/rules. It keeps the current character
unless the destination enforces a character. This makes map reload a repeatable test reset.

The sandbox generator reads the current catalogue, lays out full-size exhibits and writes
asset-index.json with names, sizes, coordinates and gallery locations. Run scripts/generate.py
then scripts/red.ps1 check. scripts/generate.py --check verifies generated content is current.
Reference maps retain their source checks; the two animation demos get documented perimeter adaptations.
Three low-overhang decorative exhibits disable display collision; their source definitions are unchanged.
