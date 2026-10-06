"""Starts a scene in headless Chromium (WebGPU), reads pictures back from the GPU and checks what a player would meet: the start card, then the world with its HUD, then a save.

Run from the repository root after building the package into /tmp/w3d (see README.md) and serving it on 127.0.0.1:8770.  Usage: verify.py [scene.json]
"""
import sys, time, zlib, struct, json
from playwright.sync_api import sync_playwright

FLAGS = ["--enable-unsafe-webgpu", "--enable-features=Vulkan,WebGPU", "--use-angle=swiftshader", "--use-webgpu-adapter=swiftshader", "--ignore-gpu-blocklist", "--enable-unsafe-swiftshader"]
W, H = 960, 540
failures = []


def check(ok, what, detail=""):
    print(("PASS  " if ok else "FAIL  ") + what + (f": {detail}" if detail else ""))
    if not ok:
        failures.append(what)


def png(rgba, path):
    raw = b"".join(b"\x00" + bytes(rgba[y * W * 4:(y + 1) * W * 4]) for y in range(H))
    def ch(t, d):
        c = struct.pack(">I", len(d)) + t + d
        return c + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
    open(path, "wb").write(b"\x89PNG\r\n\x1a\n" + ch(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 6, 0, 0, 0)) + ch(b"IDAT", zlib.compress(raw, 6)) + ch(b"IEND", b""))


def colours(px):
    return len({tuple(px[i:i + 3]) for i in range(0, len(px), 4)})


def dark_card_pixels(px):
    """How many pixels are near-black blue: the start card's panel dims the world behind it."""
    return sum(1 for i in range(0, len(px), 4) if px[i] < 40 and px[i + 1] < 40 and px[i + 2] < 60)


with sync_playwright() as p:
    b = p.chromium.launch(args=FLAGS)
    pg = b.new_page(viewport={"width": W, "height": H})
    logs = []
    pg.on("console", lambda m: logs.append(m.type + ": " + m.text))
    pg.on("pageerror", lambda e: logs.append("pageerror: " + str(e)))
    scene = sys.argv[1] if len(sys.argv) > 1 else "scene.json"
    t0 = time.time()
    pg.goto(f"http://127.0.0.1:8770/index.html?w={W}&h={H}&scene={scene}")
    while time.time() - t0 < 120 and pg.evaluate("window.__state") == "loading":
        time.sleep(0.3)
    state = pg.evaluate("window.__state")
    check(state == "ready", "the game starts in a WebGPU browser", f"{state} | {pg.evaluate('window.__info')} | {time.time() - t0:.1f}s")
    if state == "ready":
        snap = lambda settle: pg.evaluate("async (s) => Array.from(await window.__g.snapshot(s))", settle)
        check(pg.evaluate("window.__g.card_up()"), "the start card is up and holds the game")
        t1 = time.time()
        card = snap(True)
        png(card, "out/web3d-card.png")
        check(colours(card) > 1000, "the world is drawn behind the card", f"{colours(card)} colours, {time.time() - t1:.1f}s")
        pg.evaluate("window.__g.key('Enter', true)")
        check(not pg.evaluate("window.__g.card_up()"), "Enter uses the card's button and the game begins")
        pg.evaluate("window.__g.key('KeyW', true)")
        pg.evaluate("window.__g.step(150)")  # two and a half seconds of walking, no drawing: a software GPU takes seconds per frame
        pos = pg.evaluate("Array.from(window.__g.position())")
        check(pos[2] < -0.1, "walking forward moves the player", f"z = {pos[2]:.2f}")
        play = snap(False)
        png(play, "out/web3d-play.png")
        check(colours(play) > 1000, "the world is drawn in play", f"{colours(play)} colours")
        check(play != card, "the picture changed once the card went away")
        # Marcel's first sunrise is within a minute: live it, and the day it counts is offered for saving once.
        pg.evaluate("window.__g.key('KeyW', false); window.__g.step(60 * 60)")
        saved = pg.evaluate("window.__g.save_text()")
        check(saved is not None and json.loads(saved).get("days_lived") == 1, "a day lived is offered for saving", str(saved))
        check(pg.evaluate("window.__g.save_text()") is None, "an unchanged save is not offered twice")
    errs = [l for l in logs if l.startswith(("error", "pageerror")) and "GL Driver" not in l]
    check(not errs, "the console is clean", "; ".join(errs[:2])[:300])
    b.close()
sys.exit(1 if failures else 0)
