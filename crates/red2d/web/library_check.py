#!/usr/bin/env python3
"""Checks a published library page (index.html of a site written by `publish`) in a real browser: hearts keep favourites at the top and survive a reload,
the 2D/3D and favourites filters work. usage: library_check.py SITE_DIR   Prints PASS/FAIL lines; exit 1 on any failure."""
import http.server, os, socketserver, sys, threading
from playwright.sync_api import sync_playwright

site = sys.argv[1]


class H(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **k):
        super().__init__(*a, directory=site, **k)

    def log_message(self, *a):
        pass


srv = socketserver.ThreadingTCPServer(("127.0.0.1", 0), H)
threading.Thread(target=srv.serve_forever, daemon=True).start()
base = "http://127.0.0.1:%d/" % srv.server_address[1]
bad = 0


def check(name, ok, detail=""):
    global bad
    bad += 0 if ok else 1
    print("%s  library: %s %s" % ("PASS" if ok else "FAIL", name, detail))


with sync_playwright() as p:
    b = p.chromium.launch()
    pg = b.new_context().new_page()
    errs = []
    pg.on("pageerror", lambda e: errs.append(str(e)))
    pg.goto(base)
    ids = lambda: pg.evaluate("[...document.querySelectorAll('#games .card:not(.hidden)')].map(c => c.dataset.id)")
    first = ids()
    check("lists the published games", len(first) >= 1, str(first))
    last = first[-1]
    pg.click('.card[data-id="%s"] .heart' % last)
    check("a heart moves the game to the top", ids()[0] == last, str(ids()))
    pg.reload()
    check("the heart survives a reload", ids()[0] == last)
    pg.select_option("#kind", "fav")
    check("the favourites filter shows only hearted games", ids() == [last], str(ids()))
    pg.select_option("#kind", "")
    pg.select_option("#pres", "3d")
    check("the 3D filter hides 2D games", ids() == [], str(ids()))
    pg.select_option("#pres", "2d")
    check("the 2D filter shows them", len(ids()) == len(first), str(ids()))
    pg.click('.card[data-id="%s"] .heart' % last)
    check("un-hearting returns the order", ids() == first, str(ids()))
    check("no page errors", not errs, "; ".join(errs))
    b.close()
srv.shutdown()
sys.exit(1 if bad else 0)
