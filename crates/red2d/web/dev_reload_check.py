#!/usr/bin/env python3
"""Live reload of a 2D game in a real headless Chromium (ADR 2026-10-06-hot-reload-of-scene-content): `web serve --watch` and the player page.

usage: dev_reload_check.py --engine PATH_TO_red_engine2 --game examples/2d/coin-dash.game2d.json [--runs 5] [--keep]

It builds the package, starts `red_engine2 web serve PKG --port 0 --watch COPY`, opens the page, walks the player, then saves the game file and checks, by looking at the
page and not at the code:
  1. a valid edit is applied in place (no page reload) and the player is where they were, while the game changed (the screen size);
  2. an invalid edit changes nothing, the diagnostics appear on the page (the text `validate` prints) and the game keeps running;
  3. fixing the file applies it and clears the diagnostics;
  4. a plain `web serve` (no --watch) page never asks for `__dev/state`.
It also times the edit-to-visible latency (the file write to the page's `reloads` counter changing) over --runs saves and prints one JSON report. Exit 1 on any failure.
"""
import argparse, json, os, re, shutil, subprocess, sys, tempfile, time

from playwright.sync_api import sync_playwright


def wait_until(pg, expr, timeout_ms=15000):
    """Poll a JavaScript expression from outside the page (the page's Content-Security-Policy forbids string evaluation inside it)."""
    end = time.time() + timeout_ms / 1000
    while time.time() < end:
        if pg.evaluate(expr):
            return True
        time.sleep(0.02)
    return False


def start_server(engine, pkg, watch=None):
    cmd = [engine, "web", "serve", pkg, "--port", "0"] + (["--watch", watch] if watch else [])
    p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    line = p.stdout.readline()
    m = re.search(r"127\.0\.0\.1:(\d+)", line)
    if not m:
        p.kill()
        raise RuntimeError("web serve did not announce a port: " + line)
    return p, int(m.group(1))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--engine", required=True)
    ap.add_argument("--game", required=True)
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--keep", action="store_true")
    a = ap.parse_args()
    work = tempfile.mkdtemp(prefix="re2_devreload_")
    game = os.path.join(work, os.path.basename(a.game))
    shutil.copy(a.game, game)
    pkg_out = os.path.join(work, "pkg")
    b = subprocess.run([a.engine, "web", "build", game, "--out", pkg_out], capture_output=True, text=True)
    if b.returncode != 0:
        print(json.dumps({"ok": False, "step": "web build", "output": (b.stdout + b.stderr)[-1500:]}))
        return 1
    pkg = next((os.path.join(pkg_out, d) for d in os.listdir(pkg_out) if os.path.isfile(os.path.join(pkg_out, d, "manifest.json"))), pkg_out)
    report = {"ok": True, "checks": [], "latency_ms": []}

    def check(name, ok, detail=""):
        report["checks"].append({"name": name, "ok": bool(ok), "detail": str(detail)[:300]})
        report["ok"] = report["ok"] and bool(ok)

    text = open(game).read()
    server, port = start_server(a.engine, pkg, game)
    try:
        with sync_playwright() as pw:
            br = pw.chromium.launch()
            pg = br.new_page()
            errors = []
            pg.on("pageerror", lambda e: errors.append(str(e)))
            nav_before = []
            pg.on("framenavigated", lambda f: nav_before.append(f.url))
            pg.goto(f"http://127.0.0.1:{port}/")
            check("the page is ready", wait_until(pg, "window.__red2d && __red2d.status().state === 'ready'"))
            check("the dev server asked the page to poll", pg.evaluate("!!__red2d.manifest().dev_reload"))
            # Walk the player so there is a place worth keeping.
            pg.keyboard.down("ArrowRight")
            time.sleep(0.5)
            pg.keyboard.up("ArrowRight")
            before = pg.evaluate("__red2d.snapshot().named.p")
            width0 = pg.evaluate("document.getElementById('screen').width")
            check("the player walked", before and before[0] > 0, before)

            def save(text):                              # compute the new text BEFORE opening for write (opening truncates the file)
                open(game, "w").write(text)

            def edit_width(delta):                       # always from the ORIGINAL text, so a fix after a broken save is a valid file
                d = json.loads(text)
                d["view"]["width"] = d["view"]["width"] + delta
                return json.dumps(d, indent=1)

            # 1. A valid edit applies in place, keeping the player where they are.
            t0 = time.time()
            save(edit_width(8))
            ok = wait_until(pg, "__red2d.status().reloads === 1")
            report["latency_ms"].append(round((time.time() - t0) * 1000))
            after = pg.evaluate("__red2d.snapshot().named.p")
            check("a valid save was applied", ok)
            check("the game changed (screen width)", pg.evaluate("document.getElementById('screen').width") == width0 + 8, pg.evaluate("document.getElementById('screen').width"))
            check("the player stayed where they were", after == before, (before, after))
            check("the page itself did not reload", len(nav_before) == 1, nav_before)
            # 2. An invalid edit changes nothing and says what is wrong; the game keeps running.
            bad = json.loads(open(game).read())
            bad["view"]["widht"] = 1
            open(game, "w").write(json.dumps(bad))
            check("the diagnostics appear", wait_until(pg, "!!__red2d.status().dev_error"))
            err = pg.evaluate("__red2d.status().dev_error") or ""
            check("they are validate's words", "widht" in err and "SAVED FILE NOT APPLIED" in err, err[:200])
            check("nothing was applied", pg.evaluate("__red2d.status().reloads") == 1)
            tick0 = pg.evaluate("__red2d.snapshot().tick")
            time.sleep(0.4)
            check("the old game keeps running", pg.evaluate("__red2d.snapshot().tick") > tick0)
            # 3. Fixing it applies and clears the message.
            save(edit_width(16))
            check("the fixed save was applied", wait_until(pg, "__red2d.status().reloads === 2"))
            check("the diagnostics cleared", pg.evaluate("__red2d.status().dev_error") in (None, ""))
            # Timing: the same measurement over several saves.
            for i in range(a.runs - 1):
                n = pg.evaluate("__red2d.status().reloads")
                t0 = time.time()
                save(edit_width(16 + 8 * (i + 1)))
                if wait_until(pg, f"__red2d.status().reloads === {n + 1}"):
                    report["latency_ms"].append(round((time.time() - t0) * 1000))
            check("no page errors", not errors, errors)
            br.close()
    finally:
        server.kill()
    # 4. A plain server (no --watch) never polls.
    plain, pport = start_server(a.engine, pkg)
    try:
        with sync_playwright() as pw:
            br = pw.chromium.launch()
            pg = br.new_page()
            asked = []
            pg.on("request", lambda r: asked.append(r.url) if "__dev" in r.url else None)
            pg.goto(f"http://127.0.0.1:{pport}/")
            wait_until(pg, "window.__red2d && __red2d.status().state === 'ready'")
            time.sleep(1.0)
            check("a plain web serve is not asked for __dev/state", not asked and not pg.evaluate("!!__red2d.manifest().dev_reload"), asked)
            br.close()
    finally:
        plain.kill()
    lat = sorted(report["latency_ms"])
    report["latency_summary_ms"] = {"min": lat[0], "median": lat[len(lat) // 2], "max": lat[-1]} if lat else None
    if not a.keep:
        shutil.rmtree(work, ignore_errors=True)
    print(json.dumps(report, indent=1))
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
