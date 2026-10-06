# Real-device qualification (what a person must do, and what the machine already did)

RedEngine verifies browser games in **headless desktop Chromium** (and, optionally, headless Firefox), with an *emulated* phone (iPhone user agent, emulated touch points, a narrow viewport).
That is valuable and it is **not** a test on a phone: no real touch hardware, no iOS WebKit, no mobile Chrome, no real speaker, no battery or thermal limits, no real install flow.
A passing `web verify` therefore never sets `human_playtested`. This page is the short procedure that earns it, and the record format `web status` understands.

## What is automated, and what only a device answers

| Check | Automated stand-in (what it proves) | Only a real device can say |
|---|---|---|
| startup | Chromium and Firefox: the page loads, the module starts, early input and failed loads are safe | start time on a real network and CPU; the start card is readable at arm's length |
| touch controls | Chromium with emulated touch: the pad sits below the picture, nothing overlaps, touches drive the game, two thumbs, slide | the pad feels right, thumbs reach it, no accidental zoom, scroll or pull-to-refresh, notch and home-bar safe areas |
| orientation / resizing | five window shapes: the picture keeps its aspect ratio and stays inside | rotating a real phone, the browser's address bar collapsing, split-screen |
| audio unlock | a Web Audio context is `running` after the first gesture | you *hear* a sound and the music; iOS silent switch; Bluetooth; volume |
| save / load | localStorage write, reload, corrupt and blocked storage | closing the tab or the app, a private tab, "Clear site data", iOS's 7-day storage rule for unused sites |
| offline restart | service worker caches the whole game; reload with the network off | airplane mode, then relaunching from the home screen icon |
| install (PWA) | Chromium's own "installable" probe; manifest, icons, service worker | Android: install prompt or menu; iOS: Share, Add to Home Screen; the icon, the name, launching in standalone mode |
| fullscreen | nothing: the runtime has no fullscreen button | installed standalone mode is the full-screen path today; record whether it hides the browser bars |

Not covered by anything here: Safari itself (Playwright's WebKit is the engine, not the browser), Samsung Internet, old Android WebView, low-memory phones, thermal throttling.

## Get the build in front of the device (a secure context is required for install and offline)

* **Real URL (any device, the honest test):** `red_engine2 publish G --backend github-pages --repo ../RedEngineGames --push`, open the printed URL. This also tests the deployed copy.
* **Android Chrome on a USB cable:** `red_engine2 web serve out/web/<id> --port 8080`, then `adb reverse tcp:8080 tcp:8080` and open `http://localhost:8080/` on the phone (localhost counts as secure).
* **iPhone:** use the real URL, or an https tunnel to `web serve`; iOS has no `adb reverse`.
* **Desktop Chromium / Firefox:** `red_engine2 web serve out/web/<id>` and open the URL. Machine-run Firefox: `red_engine2 web setup-browser --engines firefox`, then `web verify G --engine firefox`.

## The procedure (about 10 minutes per device)

Use the build named by `red_engine2 web status G` (`package.package_id`). For each device, in order:

1. **startup** Open the URL. The start card appears, no error text, the first key press or tap starts the game.
2. **touch_controls** (phones) The controller is below the picture. Every button and the direction pad respond; two thumbs at once; nothing scrolls, zooms or pulls to refresh.
3. **orientation** Rotate. The pad stays below the picture, nothing is cut off, the game keeps running. Pull the address bar in and out.
4. **audio_unlock** Before the first tap there is silence; after it, sound effects and music play and the on-screen music button turns them off and on. (iPhone: ringer switch on.)
5. **save_load** Make progress the game saves, close the tab (or the app), reopen the URL: the progress is back. Use "Back up progress", then restore it.
6. **offline_restart** Wait a few seconds after the first load, switch on airplane mode, reload (or relaunch the installed app): the game starts and plays.
7. **install** Android: install from the browser menu. iPhone: Share, Add to Home Screen. Launch from the icon: it opens without browser bars and the saved progress is still there.
8. **fullscreen** Say whether the installed app hides the browser bars (that is the supported full-screen path), or `unsupported` if the device offers neither.

## Record it (this is the only thing that lets `human_playtested` become true)

Write `qualification/<game id>-<device>.json` next to the game file (a person's answers; an AI may type them for the person but must not invent them):

```json
{
  "schema": "red2d-device-record/1",
  "device": "Pixel 8, Android 15, Chrome 140",
  "physical": true,
  "tester": "name",
  "date": "2026-10-06",
  "build_id": "<package.package_id from web status>",
  "results": {"startup": "pass", "touch_controls": "pass", "orientation": "pass", "audio_unlock": "pass",
              "save_load": "pass", "offline_restart": "pass", "install": "pass", "fullscreen": "unsupported"},
  "notes": "anything that felt wrong"
}
```

Each answer is `pass`, `fail` or `unsupported` (the device or browser has no such thing). `red_engine2 web status G` then lists every record and says whether it counts: it must name **this build**,
come from a **physical** device, answer **all eight** checks and have **no fail**. Only then does `status` print `human_playtested: true`. `publish` still writes `human_playtested: false` into
`publication.json`: that file is the machine's account of one run, and a person's record is deliberately a separate file.

Minimum before telling players it works on phones: one iPhone (Safari), one Android phone (Chrome), desktop Chromium. Desktop Firefox is worth one pass (`web verify --engine firefox` does the machine half).
