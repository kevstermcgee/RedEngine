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
        ctx, pg, log = new_page()
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
                    check("music starts after the gesture", s["music"] == "playing", "music state: %s (rendering the loop blocked the page for %s ms)" % (s["music"], s.get("music_ms")), claim="browser-audio")
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
