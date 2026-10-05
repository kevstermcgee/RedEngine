#!/usr/bin/env python3
"""Drives a RedEngine 2D web package (a directory, or a deployed URL) in a real headless Chromium and prints one JSON report.

usage: browser_verify.py (--dir DIR | --url URL) --out OUTDIR

Every check is a real browser action: the page is loaded over HTTP, keys are pressed with the browser's input pipeline, the canvas is read back, storage is the browser's.
What it can say is exactly what a row says; it never claims a human played or heard anything.
"""
import argparse, base64, hashlib, http.server, json, os, socketserver, sys, threading, time, urllib.request

from playwright.sync_api import sync_playwright

MIME = {".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8", ".json": "application/json", ".wasm": "application/wasm", ".png": "image/png"}


class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def guess_type(self, path):
        return MIME.get(os.path.splitext(path)[1], "application/octet-stream")

    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()


def serve(directory):
    class H(Quiet):
        def __init__(self, *a, **k):
            super().__init__(*a, directory=directory, **k)

    srv = socketserver.ThreadingTCPServer(("127.0.0.1", 0), H)
    srv.daemon_threads = True
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv, srv.server_address[1]


def wait_js(pg, expr, timeout=20000):
    """Poll a JavaScript expression from outside the page. (`page.wait_for_function` evaluates a string inside the page, which the package's Content-Security-Policy,
    rightly, forbids; a CDP evaluate is not subject to it, so the real policy stays on for every test.)"""
    end = time.time() + timeout / 1000.0
    while time.time() < end:
        if pg.evaluate("() => !!(" + expr + ")"):
            return True
        time.sleep(0.04)
    raise TimeoutError("timed out waiting for: " + expr)


def fnv(data):
    h = 0xCBF29CE484222325
    for b in data:
        h = ((h ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return "%016x" % h


def parse_color(s):
    s = s.lstrip("#")
    if len(s) == 3:
        s = "".join(c * 2 for c in s)
    return tuple(int(s[i:i + 2], 16) for i in (0, 2, 4))


ROWS = []


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir")
    ap.add_argument("--url")
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    rows = ROWS

    def check(name, ok, detail, claim="browser"):
        rows.append({"ok": bool(ok), "claim": claim, "name": name, "detail": detail})

    srv = None
    if a.dir:
        srv, port = serve(a.dir)
        base = "http://127.0.0.1:%d" % port
    else:
        base = a.url.rstrip("/")

    def fetch(path):
        with urllib.request.urlopen(base + "/" + path, timeout=20) as r:
            return r.status, r.read()

    status, mtext = fetch("manifest.json")
    manifest = json.loads(mtext)
    game = manifest["game"]
    gid = game["id"]
    key = "red2d:" + gid
    status, gtext = fetch("assets/game.json")
    gjson = json.loads(gtext)
    bg = parse_color(gjson.get("view", {}).get("background", "#10141c"))
    vw, vh = game["screen"]["width"], game["screen"]["height"]
    native = manifest["native"]
    persists_caps = bool(game["persistence"])
    out = {"base": base, "game": gid, "package_id": manifest.get("package_id")}

    with sync_playwright() as p:
        browser = p.chromium.launch()
        out["browser"] = "Chromium " + browser.version

        def new_page(init_script=None, viewport=(960, 540)):
            ctx = browser.new_context(viewport={"width": viewport[0], "height": viewport[1]})
            if init_script:
                ctx.add_init_script(init_script)
            pg = ctx.new_page()
            log = {"console": [], "pageerrors": [], "failed": [], "http": []}
            pg.on("console", lambda m: log["console"].append((m.type, m.text)) if m.type in ("error", "warning") else None)
            pg.on("pageerror", lambda e: log["pageerrors"].append(str(e)))
            pg.on("requestfailed", lambda r: log["failed"].append(r.url + " " + str(r.failure)))
            pg.on("response", lambda r: log["http"].append((r.status, r.url)) if r.status >= 400 else None)
            return ctx, pg, log

        def ready(pg, timeout=20000):
            wait_js(pg, "window.__red2d && window.__red2d.status().state !== 'loading'", timeout)
            return pg.evaluate("__red2d.status()")

        def fatal(log):
            errs = [t for (k, t) in log["console"] if k == "error"] + log["pageerrors"] + log["failed"] + ["HTTP %d %s" % h for h in log["http"]]
            return errs

        # ---- phase A: paused page, exact comparisons with the native build -------------------------------------------------------------------------
        ctx, pg, log = new_page()
        resp = pg.goto(base + "/index.html?paused=1")
        check("html loads", resp is not None and resp.status == 200 and pg.title() == game["title"], "HTTP %s, title %r" % (resp and resp.status, pg.title()))
        st = ready(pg)
        check("wasm initialises", st["wasm"] and st["state"] == "ready", "state=%s wasm=%s%s" % (st["state"], st["wasm"], (" error=" + str(st["error"])) if st["error"] else ""))
        if st["state"] == "ready":
            snap = pg.evaluate("__red2d.snapshot()")
            check("initial state equals native", snap["hash"] == native["initial"]["state_hash"] and snap["tick"] == 0, "browser %s, native %s (tick %s)" % (snap["hash"], native["initial"]["state_hash"], snap["tick"]))
            px = pg.evaluate("__red2d.pixels()")
            raw = base64.b64decode(px["b64"])
            colors = set(raw[i:i + 3] for i in range(0, len(raw), 4))
            covered = sum(1 for i in range(0, len(raw), 4) if tuple(raw[i:i + 3]) != bg) / max(1, len(raw) // 4)
            check("meaningful render", len(colors) >= 2 and covered >= 0.002, "%dx%d canvas, %d colours, %.1f%% of pixels differ from the background" % (px["w"], px["h"], len(colors), covered * 100))
            h = fnv(raw)
            check("first frame is pixel-identical to native", h == native["initial"]["frame"], "browser canvas %s, native renderer %s" % (h, native["initial"]["frame"]))
            # every packaged file resolves and is the file that was built
            bad = []
            for f in manifest["files"]:
                try:
                    s, b = fetch(f["path"])
                    if s != 200 or hashlib.sha256(b).hexdigest() != f["sha256"]:
                        bad.append(f["path"] + " differs")
                except Exception as e:
                    bad.append("%s: %s" % (f["path"], e))
            check("assets resolve", not bad, "%d file(s) fetched over HTTP, each SHA-256 equals the manifest" % len(manifest["files"]) if not bad else "; ".join(bad))
            # the game's own scenarios, run inside the browser's wasm, end in the native hashes
            sc = pg.evaluate("__red2d.scenarios()")
            want = {s["name"]: s for s in native["scenarios"]}
            wrong = ["%s: browser %s vs native %s" % (s["name"], s["hash"], want[s["name"]]["hash"]) for s in sc if s["name"] in want and s["hash"] != want[s["name"]]["hash"]]
            failed = ["%s: %s" % (s["name"], "; ".join(s["failures"])) for s in sc if not s["ok"]]
            check("scenarios replay identically in the browser", sc and not wrong and not failed, ("%d scenario(s), every final state hash equals the native run" % len(sc)) if sc and not wrong and not failed else "; ".join(wrong + failed) or "the game has no scenarios")
            # resolution independence
            lay_bad = []
            for (w, hh) in [(800, 600), (1600, 400), (400, 800), (1280, 720)]:
                pg.set_viewport_size({"width": w, "height": hh})
                pg.wait_for_timeout(120)
                r = pg.evaluate("(() => { const r = document.getElementById('screen').getBoundingClientRect(); return {x: r.left, y: r.top, w: r.width, h: r.height, lay: __red2d.layout()}; })()")
                inside = r["x"] >= -0.5 and r["y"] >= -0.5 and r["x"] + r["w"] <= w + 0.5 and r["y"] + r["h"] <= hh + 0.5
                if game["screen"]["scale"] == "integer" and w >= vw and hh >= vh:
                    shape = abs(r["w"] / vw - round(r["w"] / vw)) < 1e-6
                else:
                    shape = abs(r["w"] / r["h"] - vw / vh) < 0.01 * vw / vh + 0.01
                c = pg.evaluate("__red2d.toView(%f, %f)" % (r["x"] + r["w"] / 2, r["y"] + r["h"] / 2))
                centre = c is not None and abs(c[0] - vw / 2) < 1.5 and abs(c[1] - vh / 2) < 1.5
                bars = (r["x"] > 2 or r["y"] > 2)
                outside = pg.evaluate("__red2d.toView(%f, %f)" % ((r["x"] - 3) if r["x"] > 2 else r["x"] + 1, (r["y"] - 3) if r["y"] > 2 else r["y"] + 1))
                bar_ok = (outside is None) if bars else True
                if not (inside and shape and centre and bar_ok):
                    lay_bad.append("%dx%d: inside=%s aspect/scale ok=%s centre maps=%s bars map to nothing=%s (%s)" % (w, hh, inside, shape, centre, bar_ok, r))
            check("resolution independence", not lay_bad, "window shapes 800x600, 1600x400, 400x800, 1280x720: the screen keeps its aspect ratio, stays inside, its centre maps to the middle, letterbox bars map to nothing" if not lay_bad else "; ".join(lay_bad))
        errs = fatal(log)
        check("no console errors (paused page)", not errs, "none" if not errs else "; ".join(errs[:5]))
        ctx.close()

        # ---- phase B: the real-time page with real input ----------------------------------------------------------------------------------------------
        ctx, pg, log = new_page("window.__longtasks = []; try { new PerformanceObserver((l) => { for (const e of l.getEntries()) window.__longtasks.push(Math.round(e.duration)); }).observe({entryTypes: ['longtask']}); } catch (e) {}")
        pg.goto(base + "/index.html")
        st = ready(pg)
        if st["state"] != "ready":
            check("page reaches the start screen", False, "state=%s error=%s" % (st["state"], st["error"]))
        else:
            t0 = pg.evaluate("__red2d.snapshot()")["tick"]
            pg.wait_for_timeout(500)
            t1 = pg.evaluate("__red2d.snapshot()")["tick"]
            check("waits for the player", t0 == 0 and t1 == 0, "ticks before any input: %s -> %s; start screen visible: %s" % (t0, t1, pg.is_visible("#start")))
            pg.screenshot(path=os.path.join(a.out, "browser-start.png"))
            pad_state = pg.evaluate("({hidden: document.getElementById('pad').hidden, touchClass: document.body.classList.contains('touch'), pad: __red2d.status().pad})")
            check("desktop: no touch controller", pad_state["hidden"] and not pad_state["touchClass"] and pad_state["pad"] == "hidden", "a mouse-and-keyboard browser shows no on-screen pad (hidden=%s, touch class=%s)" % (pad_state["hidden"], pad_state["touchClass"]))
            pg.keyboard.press("Enter")
            wait_js(pg, "__red2d.status().state === 'running'", 5000)
            has_audio = bool(gjson.get("sounds")) or bool(gjson.get("music"))
            if has_audio:
                try:
                    wait_js(pg, "__red2d.status().audio === 'running'", 4000)
                except Exception:
                    pass
                s = pg.evaluate("__red2d.status()")
                check("browser audio initialised", s["audio"] == "running", "Web Audio context state after the first key press: %s (it starts only inside a user gesture)" % s["audio"], claim="browser-audio")
                if gjson.get("sounds"):
                    r = pg.evaluate("__red2d.testSound(0)")
                    check("a sound plays through Web Audio", r.get("ok") and r.get("seconds", 0) > 0.05, json.dumps(r), claim="browser-audio")
                if gjson.get("music"):
                    try:
                        wait_js(pg, "__red2d.status().music === 'playing'", 6000)
                    except Exception:
                        pass
                    s = pg.evaluate("__red2d.status()")
                    check("music starts after the gesture", s["music"] == "playing", "music state: %s (the loop took %s ms to render, in a worker: %s; it stalled the page for %s ms)" % (s["music"], s.get("music_ms"), s.get("music_worker"), s.get("music_blocked_ms")), claim="browser-audio")
                    longest = max(pg.evaluate("window.__longtasks") or [0])
                    check("the page stays responsive while the music is made", longest < 250, "the longest main-thread task from page load to music playing was %d ms (limit 250 ms; a render on the main thread would show here)" % longest)
            before = pg.evaluate("__red2d.snapshot()")["tick"]
            pg.wait_for_timeout(1000)
            after = pg.evaluate("__red2d.snapshot()")["tick"]
            check("time advances in real time", 30 <= after - before <= 120, "%d ticks in ~1 s (60 expected)" % (after - before))
            # the game's own browser checks, with real key and mouse events
            for bc in manifest.get("browser_checks", []):
                snap0 = pg.evaluate("__red2d.snapshot()")
                lay = pg.evaluate("__red2d.layout()")
                if bc.get("click"):
                    cx = lay["x"] + bc["click"][0] * lay["w"] / vw
                    cy = lay["y"] + bc["click"][1] * lay["h"] / vh
                    pg.mouse.move(cx, cy)
                    pg.mouse.down()
                    pg.mouse.up()
                    pg.wait_for_timeout(150)
                for k in bc.get("keys", []):
                    pg.keyboard.down(k)
                if bc.get("keys"):
                    pg.wait_for_timeout(bc.get("ms", 400))
                for k in reversed(bc.get("keys", [])):
                    pg.keyboard.up(k)
                snap1 = pg.evaluate("__red2d.snapshot()")
                changed = [n for n in bc["changes"] if snap0["vars"].get(n) != snap1["vars"].get(n)]
                check("input: " + bc["name"], bool(changed), ("%s changed (%s -> %s)" % (changed[0], snap0["vars"].get(changed[0]), snap1["vars"].get(changed[0]))) if changed else "none of %s changed after the input" % bc["changes"])
                if bc.get("persists") and changed:
                    pg.wait_for_timeout(250)
                    saved = pg.evaluate("localStorage.getItem(%s)" % json.dumps(key))
                    want = {n: snap1["vars"].get(n) for n in bc["persists"]}
                    pg.reload()
                    ready(pg)
                    snap2 = pg.evaluate("__red2d.snapshot()")
                    st2 = pg.evaluate("__red2d.status()")
                    got = {n: snap2["vars"].get(n) for n in bc["persists"]}
                    check("persistence survives reload: " + bc["name"], saved is not None and got == want and st2["save"] == "loaded", "saved %s; after reload %s (wanted %s); save status %s" % ("yes" if saved else "NO", got, want, st2["save"]))
                    # leave the page running again for the next check
                    pg.keyboard.press("Enter")
                    wait_js(pg, "__red2d.status().state === 'running'", 5000)
            pg.wait_for_timeout(300)
            pg.screenshot(path=os.path.join(a.out, "browser-running.png"))
            s = pg.evaluate("__red2d.status()")
            check("game still running", s["state"] == "running", "state=%s frames=%d ticks=%d" % (s["state"], s["frames"], s["ticks"]))
        errs = fatal(log)
        check("no console errors (played page)", not errs, "none" if not errs else "; ".join(errs[:5]))
        ctx.close()

        # ---- phase C: storage that fails, is corrupt, belongs to another game, or is reset ---------------------------------------------------------------
        if persists_caps:
            ctx, pg, log = new_page("Object.defineProperty(window, 'localStorage', {get() { throw new DOMException('blocked', 'SecurityError'); }});")
            pg.goto(base + "/index.html")
            st = ready(pg)
            pg.keyboard.press("Enter")
            pg.wait_for_timeout(600)
            st = pg.evaluate("__red2d.status()")
            check("storage unavailable: the game still plays", st["state"] == "running" and st["storage"] == "unavailable" and not log["pageerrors"], "state=%s storage=%s (%s)" % (st["state"], st["storage"], st["saveMessage"]))
            ctx.close()

            persists_check = next((bc for bc in manifest.get("browser_checks", []) if bc.get("persists")), None)
            if persists_check:
                ctx, pg, log = new_page("const _s = Storage.prototype.setItem; Storage.prototype.setItem = function(k, v) { if (String(k).endsWith(':probe')) return _s.call(this, k, v); throw new DOMException('full', 'QuotaExceededError'); };")
                pg.goto(base + "/index.html")
                ready(pg)
                pg.keyboard.press("Enter")
                wait_js(pg, "__red2d.status().state === 'running'", 5000)
                lay = pg.evaluate("__red2d.layout()")
                if persists_check.get("click"):
                    pg.mouse.click(lay["x"] + persists_check["click"][0] * lay["w"] / vw, lay["y"] + persists_check["click"][1] * lay["h"] / vh)
                for k in persists_check.get("keys", []):
                    pg.keyboard.down(k)
                pg.wait_for_timeout(persists_check.get("ms", 300) + 300)
                for k in persists_check.get("keys", []):
                    pg.keyboard.up(k)
                st = pg.evaluate("__red2d.status()")
                check("saving fails (quota): the game keeps running and says so", st["state"] == "running" and st["storage"] == "unavailable" and not log["pageerrors"], "state=%s storage=%s (%s)" % (st["state"], st["storage"], st["saveMessage"]))
                ctx.close()

            for label, blob, expect in [("corrupt save", "{this is not json", "unreadable"), ("another game's save", json.dumps({"red2d_save": 1, "game": "some-other-game", "vars": {"x": 1}}), "incompatible"), ("newer save format", json.dumps({"red2d_save": 99, "game": gid, "vars": {}}), "incompatible")]:
                ctx, pg, log = new_page("if (!localStorage.getItem(%s)) localStorage.setItem(%s, %s);" % (json.dumps(key), json.dumps(key), json.dumps(blob)))
                pg.goto(base + "/index.html")
                st = ready(pg)
                backup = pg.evaluate("localStorage.getItem(%s)" % json.dumps(key + ":unreadable"))
                pg.keyboard.press("Enter")
                pg.wait_for_timeout(400)
                st2 = pg.evaluate("__red2d.status()")
                check("%s: ignored, kept as a backup, game plays" % label, st["save"] == expect and backup == blob and st2["state"] == "running" and not log["pageerrors"], "save status %s (wanted %s); backup kept: %s; state %s" % (st["save"], expect, backup == blob, st2["state"]))
                ctx.close()

            if persists_check:
                ctx, pg, log = new_page()
                pg.goto(base + "/index.html")
                ready(pg)
                pg.keyboard.press("Enter")
                wait_js(pg, "__red2d.status().state === 'running'", 5000)
                lay = pg.evaluate("__red2d.layout()")
                if persists_check.get("click"):
                    pg.mouse.click(lay["x"] + persists_check["click"][0] * lay["w"] / vw, lay["y"] + persists_check["click"][1] * lay["h"] / vh)
                pg.wait_for_timeout(400)
                had = pg.evaluate("localStorage.getItem(%s)" % json.dumps(key))
                pg.evaluate("__red2d.resetSave()")
                gone = pg.evaluate("localStorage.getItem(%s)" % json.dumps(key)) is None
                pg.reload()
                st = ready(pg)
                check("reset: removing the save returns the game to a fresh start", had is not None and gone and st["save"] == "fresh", "had a save: %s; removed: %s; after reload: %s" % (had is not None, gone, st["save"]))
                ctx.close()

        # ---- phase E: installable, offline, durable, backed up -----------------------------------------------------------------------------------------------
        ctx, pg, log = new_page()
        pg.goto(base + "/index.html")
        st = ready(pg)
        try:
            wait_js(pg, "__red2d.status().offline === 'ready' || __red2d.status().offline === 'unavailable'", 10000)
        except Exception:
            pass
        st = pg.evaluate("__red2d.status()")
        check("install: a service worker stores the whole game", st["offline"] == "ready", "offline support: %s (a service worker needs https or localhost)" % st["offline"])
        if st["offline"] == "ready":
            pg.reload()
            ready(pg)
            cdp = ctx.new_cdp_session(pg)
            try:
                errs = cdp.send("Page.getInstallabilityErrors").get("installabilityErrors", [])
                check("install: the browser says the page is installable", not errs, "no installability errors (manifest, icons, service worker with a fetch handler)" if not errs else "; ".join("%s %s" % (e.get("errorId"), e.get("errorArguments")) for e in errs))
            except Exception as e:
                check("install: the browser says the page is installable", False, "could not ask the browser: %s" % e)
            ctx.set_offline(True)
            try:
                pg.reload()
                st2 = ready(pg)
                tick0 = pg.evaluate("__red2d.snapshot()")["tick"]
                pg.evaluate("__red2d.advance(30)")
                tick1 = pg.evaluate("__red2d.snapshot()")["tick"]
                check("install: it plays with the network off", st2["state"] == "ready" and tick1 - tick0 == 30, "reloaded offline: state=%s, advanced %d ticks" % (st2["state"], tick1 - tick0))
            except Exception as e:
                check("install: it plays with the network off", False, "reload with no network failed: %s" % e)
            ctx.set_offline(False)
        pg.keyboard.press("Enter")
        wait_js(pg, "__red2d.status().state === 'running'", 5000)
        try:
            wait_js(pg, "__red2d.status().persistent !== null", 3000)
        except Exception:
            pass
        st = pg.evaluate("__red2d.status()")
        check("storage: the game asked the browser to keep its saves", st["persistent"] is not None, "navigator.storage.persist() answered %s (a browser may say no; the answer is shown, not assumed)" % st["persistent"])
        if game["persistence"]:
            pc = next((bc for bc in manifest.get("browser_checks", []) if bc.get("persists")), None)
            if pc:
                lay = pg.evaluate("__red2d.layout()")
                if pc.get("click"):
                    pg.mouse.click(lay["x"] + pc["click"][0] * lay["w"] / vw, lay["y"] + pc["click"][1] * lay["h"] / vh)
                pg.wait_for_timeout(400)
                before = pg.evaluate("__red2d.snapshot()")["vars"]
                pg.reload()
                ready(pg)
                try:
                    with pg.expect_download(timeout=5000) as dl:
                        pg.evaluate("document.getElementById('backup').click()")
                    path = dl.value.path()
                    text = open(path).read()
                    b = json.loads(text)
                    ok = b.get("red2d_backup") == 1 and b.get("game") == gid and isinstance(b.get("save"), str) and json.loads(b["save"]).get("game") == gid
                    check("backup: 'Back up progress' downloads the saved progress", ok, "%d bytes, game %s" % (len(text), b.get("game")))
                    pg.evaluate("__red2d.resetSave()")
                    pg.reload()
                    fresh = ready(pg)["save"]
                    pg.set_input_files("#restorefile", path)
                    wait_js(pg, "__red2d.status().state === 'ready' && __red2d.status().save === 'loaded'", 8000)
                    after = pg.evaluate("__red2d.snapshot()")["vars"]
                    same = all(after.get(n) == before.get(n) for n in pc["persists"])
                    check("backup: restoring the file brings the progress back", fresh == "fresh" and same, "after the reset the save was %s; after restoring, %s" % (fresh, {n: after.get(n) for n in pc["persists"]}))
                    wrong = pg.evaluate("__red2d.restoreText(%s)" % json.dumps(json.dumps({"red2d_backup": 1, "game": "some-other-game", "save": "{}"})))
                    check("backup: another game's backup is refused", "not this one" in wrong, wrong)
                except Exception as e:
                    check("backup: download and restore", False, "%s: %s" % (type(e).__name__, e))
        errs = fatal(log)
        check("install: no console errors", not errs, "none" if not errs else "; ".join(errs[:5]))
        ctx.close()

        # ---- phase D: a phone (touch, small screen): the controller sits below the game, never over it, and real touches drive the game ----------------------------------
        controls = game.get("controls") or {}
        if "touch" in game["input"]:
            KEY_ACTION = {"ArrowLeft": "left", "KeyA": "left", "ArrowRight": "right", "KeyD": "right", "ArrowUp": "up", "KeyW": "up", "ArrowDown": "down", "KeyS": "down", "Space": "action", "KeyZ": "action", "KeyJ": "action", "ShiftLeft": "secondary", "KeyX": "secondary", "KeyK": "secondary", "Escape": "pause", "KeyP": "pause"}
            UA = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1"
            ctx = browser.new_context(viewport={"width": 390, "height": 760}, device_scale_factor=3, is_mobile=True, has_touch=True, user_agent=UA)
            pg = ctx.new_page()
            log = {"console": [], "pageerrors": [], "failed": [], "http": []}
            pg.on("console", lambda m: log["console"].append((m.type, m.text)) if m.type in ("error", "warning") else None)
            pg.on("pageerror", lambda e: log["pageerrors"].append(str(e)))
            pg.on("requestfailed", lambda r: log["failed"].append(r.url + " " + str(r.failure)))
            pg.on("response", lambda r: log["http"].append((r.status, r.url)) if r.status >= 400 else None)
            pg.goto(base + "/index.html")
            st = ready(pg)
            geo_js = "(() => { const q = (id) => { const r = document.getElementById(id).getBoundingClientRect(); return [r.left, r.top, r.width, r.height]; }; return {stage: q('stage'), canvas: q('screen'), pad: q('pad'), padHidden: document.getElementById('pad').hidden, vw: innerWidth, vh: innerHeight, scrollH: document.documentElement.scrollHeight, coarse: matchMedia('(pointer: coarse)').matches}; })()"

            def geometry(tag):
                g = pg.evaluate(geo_js)
                cx, cy, cw, ch = g["canvas"]
                px, py, pw, ph = g["pad"]
                sx, sy, sw, sh = g["stage"]
                problems = []
                want_pad = bool(controls.get("visible"))
                if want_pad and g["padHidden"]:
                    problems.append("the controller is hidden")
                if not want_pad and not g["padHidden"]:
                    problems.append("a controller is shown for a game that needs none")
                if want_pad and not g["padHidden"]:
                    if py < cy + ch - 0.6 or py < sy + sh - 0.6:
                        problems.append("the controller (top %.0f) is not below the picture (bottom %.0f)" % (py, cy + ch))
                    if py + ph > g["vh"] + 0.6:
                        problems.append("the controller runs off the bottom of the screen")
                    if pw > g["vw"] + 0.6:
                        problems.append("the controller is wider than the screen")
                if cx < -0.6 or cx + cw > g["vw"] + 0.6 or cy < -0.6:
                    problems.append("the picture is outside the screen")
                if abs(cw / ch - vw / vh) > 0.02 * vw / vh + 0.01:
                    problems.append("the picture lost its aspect ratio (%.3f vs %.3f)" % (cw / ch, vw / vh))
                if g["scrollH"] > g["vh"] + 1:
                    problems.append("the page scrolls (%d > %d)" % (g["scrollH"], g["vh"]))
                if cw < sw - 1.5 and ch < sh - 1.5:
                    problems.append("the picture does not use the space it has (%dx%d in a %dx%d stage)" % (cw, ch, sw, sh))
                check("phone %s: the controller is below the game and nothing overlaps" % tag, g["coarse"] and not problems, ("pad shown=%s, picture %dx%d at y=%d, controller y=%d h=%d on a %dx%d screen" % (not g["padHidden"], cw, ch, cy, py, ph, g["vw"], g["vh"])) if not problems else "; ".join(problems) + " (coarse pointer: %s)" % g["coarse"])
                return g

            geometry("portrait")
            pg.screenshot(path=os.path.join(a.out, "mobile-portrait.png"))
            pg.set_viewport_size({"width": 844, "height": 390})
            pg.wait_for_timeout(250)
            geometry("landscape")
            pg.screenshot(path=os.path.join(a.out, "mobile-landscape.png"))
            pg.set_viewport_size({"width": 390, "height": 760})
            pg.wait_for_timeout(250)

            # a touch starts the game (and is the gesture that unlocks audio)
            g = pg.evaluate(geo_js)
            sx, sy, sw, sh = g["stage"]
            pg.touchscreen.tap(sx + sw / 2, sy + sh / 2)
            wait_js(pg, "__red2d.status().state === 'running'", 5000)
            has_audio = bool(gjson.get("sounds")) or bool(gjson.get("music"))
            if has_audio:
                try:
                    wait_js(pg, "__red2d.status().audio === 'running'", 4000)
                except Exception:
                    pass
                check("phone: audio starts after the first touch", pg.evaluate("__red2d.status().audio") == "running", "Web Audio context state after a tap: %s" % pg.evaluate("__red2d.status().audio"), claim="browser-audio")
            over = pg.evaluate("(() => { const g = document.getElementById('screen').getBoundingClientRect(); const e = document.elementFromPoint(g.left + g.width / 2, g.top + g.height / 2); return e ? e.id : null; })()")
            check("phone: nothing covers the picture", over == "screen", "the element at the centre of the picture is #%s" % over)

            cdp = ctx.new_cdp_session(pg)

            def touch(points, kind):
                cdp.send("Input.dispatchTouchEvent", {"type": kind, "touchPoints": [{"x": x, "y": y, "id": i} for i, (x, y) in enumerate(points)] if kind != "touchEnd" else []})

            def centre(sel):
                r = pg.evaluate("(sel) => { const e = document.querySelector(sel); if (!e) return null; const r = e.getBoundingClientRect(); return [r.left, r.top, r.width, r.height]; }", sel)
                return r

            def point_for(action):
                if action in ("left", "right", "up", "down"):
                    d = centre("#pad .dir")
                    if not d:
                        return None
                    x0, y0, w, h = d
                    off = {"left": (-0.34, 0), "right": (0.34, 0), "up": (0, -0.34), "down": (0, 0.34)}[action]
                    return (x0 + w / 2 + off[0] * w, y0 + h / 2 + off[1] * h)
                r = centre('#pad [data-action="%s"]' % action)
                return (r[0] + r[2] / 2, r[1] + r[3] / 2) if r else None

            if controls.get("visible"):
                tested = 0
                for bc in manifest.get("browser_checks", []):
                    acts = [KEY_ACTION.get(k) for k in bc.get("keys", [])]
                    if not acts or None in acts:
                        continue
                    pts = [point_for(ac) for ac in acts]
                    if None in pts:
                        continue
                    snap0 = pg.evaluate("__red2d.snapshot()")
                    touch(pts[:1] if len(pts) == 1 else pts, "touchStart")
                    pg.wait_for_timeout(bc.get("ms", 400))
                    held_now = pg.evaluate("__red2d.held()")
                    touch([], "touchEnd")
                    pg.wait_for_timeout(120)
                    snap1 = pg.evaluate("__red2d.snapshot()")
                    after = pg.evaluate("__red2d.held()")
                    changed = [n for n in bc["changes"] if snap0["vars"].get(n) != snap1["vars"].get(n)]
                    check("phone touch: " + bc["name"], bool(changed) and (acts == ["pause"] or all(ac in held_now for ac in acts)) and not after, ("touching %s held %s, %s changed (%s -> %s), released: %s" % (acts, held_now, changed[0] if changed else "nothing", snap0["vars"].get(changed[0]) if changed else "", snap1["vars"].get(changed[0]) if changed else "", not after)))
                    tested += 1
                    break
                dirs = [ac for ac in ("left", "right", "up", "down") if point_for(ac)]
                if len(dirs) >= 2:
                    # sliding the thumb across the pad switches direction without lifting
                    a1, a2 = ("left", "right") if "left" in dirs and "right" in dirs else (dirs[0], dirs[1])
                    p1, p2 = point_for(a1), point_for(a2)
                    touch([p1], "touchStart"); pg.wait_for_timeout(120)
                    h1 = pg.evaluate("__red2d.held()")
                    touch([p2], "touchMove"); pg.wait_for_timeout(120)
                    h2 = pg.evaluate("__red2d.held()")
                    touch([], "touchEnd"); pg.wait_for_timeout(100)
                    check("phone touch: sliding the thumb changes direction", h1 == [a1] and h2 == [a2], "start on %s held %s; slid to %s held %s" % (a1, h1, a2, h2))
                btns = [b for b in controls.get("buttons", []) if point_for(b["action"])]
                if btns and dirs:
                    # two thumbs at once: a direction and a button
                    d1, b1 = point_for(dirs[0]), point_for(btns[0]["action"])
                    touch([d1, b1], "touchStart"); pg.wait_for_timeout(150)
                    both = pg.evaluate("__red2d.held()")
                    touch([], "touchEnd"); pg.wait_for_timeout(100)
                    check("phone touch: two thumbs at once", dirs[0] in both and btns[0]["action"] in both, "touching %s and %s held %s" % (dirs[0], btns[0]["action"], both))
                if controls.get("pause"):
                    touch([point_for("pause")], "touchStart"); pg.wait_for_timeout(60); touch([], "touchEnd"); pg.wait_for_timeout(250)
                    check("phone touch: the pause button is a press", pg.evaluate("__red2d.held()") == [], "held after the tap: %s" % pg.evaluate("__red2d.held()"))
            # tapping and dragging the picture works for click games on a phone
            for bc in manifest.get("browser_checks", []):
                if bc.get("click") and not bc.get("persists"):
                    lay = pg.evaluate("__red2d.layout()")
                    snap0 = pg.evaluate("__red2d.snapshot()")
                    pg.touchscreen.tap(lay["x"] + bc["click"][0] * lay["w"] / vw, lay["y"] + bc["click"][1] * lay["h"] / vh)
                    pg.wait_for_timeout(250)
                    snap1 = pg.evaluate("__red2d.snapshot()")
                    changed = [n for n in bc["changes"] if snap0["vars"].get(n) != snap1["vars"].get(n)]
                    check("phone touch: tapping the picture: " + bc["name"], bool(changed), ("%s changed" % changed[0]) if changed else "none of %s changed after the tap" % bc["changes"])
                    break
            pg.screenshot(path=os.path.join(a.out, "mobile-playing.png"))
            errs = fatal(log)
            check("phone: no console errors", not errs, "none" if not errs else "; ".join(errs[:5]))
            ctx.close()
        browser.close()

    out["rows"] = rows
    out["ok"] = all(r["ok"] for r in rows) and len(rows) > 0
    print(json.dumps(out))
    if srv:
        srv.shutdown()


if __name__ == "__main__":
    try:
        main()
    except Exception as e:  # report, do not hide, a failure of the harness itself
        ROWS.append({"ok": False, "claim": "browser", "name": "the verification itself", "detail": "stopped by %s: %s (the rows above ran; the rest did not)" % (type(e).__name__, e)})
        print(json.dumps({"ok": False, "browser": "unknown", "rows": ROWS}))
        sys.exit(2)
