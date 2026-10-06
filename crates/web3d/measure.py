#!/usr/bin/env python3
"""Measures the engine's own 3D player (red_engine2::web3d) in a real headless Chromium, for one scene, and prints one JSON report.

usage: measure.py --dist DIR --scene SCENE.json [--name NAME] [--parity-script "KeyW:120;:30"] [--expected EXPECTED.json] [--frames 30] [--out OUTDIR]

DIR is the wasm-bindgen output with crates/web3d/web/index.html copied in (scripts/web3d_measure.sh builds it). What this can say is exactly what a field says; it never claims a real GPU:
headless Chromium here renders on a software adapter, so frame times describe that adapter, not a laptop or a phone, and the report says which adapter it was.
`--expected` is the output of `cargo run --example web3d_parity -- SCENE "SCRIPT"`: the same scripted inputs run natively. The browser runs the same script through
`key()` / `run()` and the two states are compared (checksum, position, rules state).
"""
import argparse, gzip, http.server, json, math, os, shutil, socketserver, struct, sys, threading, time, zlib

from playwright.sync_api import sync_playwright

FLAGS = ["--enable-unsafe-webgpu", "--enable-features=Vulkan,WebGPU", "--use-angle=swiftshader", "--use-webgpu-adapter=swiftshader", "--ignore-gpu-blocklist",
         "--enable-unsafe-swiftshader", "--enable-precise-memory-info"]
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


def png(w, h, rgba, path):
    raw = b"".join(b"\x00" + bytes(rgba[y * w * 4:(y + 1) * w * 4]) for y in range(h))

    def ch(t, d):
        c = struct.pack(">I", len(d)) + t + d
        return c + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)

    open(path, "wb").write(b"\x89PNG\r\n\x1a\n" + ch(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)) + ch(b"IDAT", zlib.compress(raw, 6)) + ch(b"IEND", b""))


def stats(v):
    v = sorted(v)
    return {"n": len(v), "min": round(v[0], 2), "median": round(v[len(v) // 2], 2), "p95": round(v[min(len(v) - 1, int(len(v) * 0.95))], 2), "max": round(v[-1], 2)} if v else {"n": 0}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dist", required=True)
    ap.add_argument("--scene", required=True)
    ap.add_argument("--name")
    ap.add_argument("--parity-script", default="KeyW:90;KeyW+ShiftLeft:60;KeyD:45;:20")
    ap.add_argument("--expected")
    ap.add_argument("--frames", type=int, default=30)
    ap.add_argument("--size", default="480x270", help="canvas size in pixels: a software adapter's cost grows with it, so the default is modest")
    ap.add_argument("--out", default="out/web3d")
    a = ap.parse_args()
    name = a.name or os.path.splitext(os.path.basename(a.scene))[0]
    W, H = (int(x) for x in a.size.split("x"))
    os.makedirs(a.out, exist_ok=True)
    scene_file = "scene-%s.json" % name
    shutil.copy(a.scene, os.path.join(a.dist, scene_file))
    report = {"scene": name, "scene_file": a.scene, "canvas": [W, H],
              "host": {"cpus": os.cpu_count(), "loadavg_1m": round(os.getloadavg()[0], 2), "note": "other work on the machine slows a software adapter: read the numbers with the load beside them"}}

    # ---- build size (what a player downloads) ----------------------------------------------------------------------------------------------------------
    sizes = {}
    for f in ("web3d_bg.wasm", "web3d.js", "index.html", scene_file):
        p = os.path.join(a.dist, f)
        if os.path.isfile(p):
            raw = open(p, "rb").read()
            sizes[f] = {"bytes": len(raw), "gzip_bytes": len(gzip.compress(raw, 9))}
    report["size"] = sizes

    srv, port = serve(a.dist)
    base = "http://127.0.0.1:%d" % port
    with sync_playwright() as p:
        browser = p.chromium.launch(args=FLAGS)
        report["browser"] = "Chromium " + browser.version

        def page(query):
            ctx = browser.new_context(viewport={"width": 960, "height": 540})
            pg = ctx.new_page()
            log = []
            pg.on("console", lambda m: log.append((m.type, m.text)))
            pg.on("pageerror", lambda e: log.append(("pageerror", str(e))))
            t0 = time.time()
            pg.goto(base + "/index.html?" + query, wait_until="domcontentloaded")
            while time.time() - t0 < 180 and pg.evaluate("window.__state") == "loading":
                time.sleep(0.2)
            return ctx, pg, log, (time.time() - t0) * 1000.0

        # ---- startup, shader and render compatibility, asset loading -------------------------------------------------------------------------------------
        ctx, pg, log, total_ms = page("scene=%s&w=%d&h=%d" % (scene_file, W, H))
        state = pg.evaluate("window.__state")
        report["state"] = state
        report["log"] = [t for (k, t) in log if "GL Driver" not in t][:8]
        if state != "ready":
            report["ok"] = False
            report["error"] = state
            print(json.dumps(report))
            srv.shutdown()
            sys.exit(1)
        info = pg.evaluate("window.__info")
        report["adapter"] = info
        report["software_adapter"] = any(w in info.lower() for w in ("swiftshader", "llvmpipe", "software", "cpu"))
        res = pg.evaluate("performance.getEntriesByType('resource').map(r => ({name: r.name.split('/').pop(), transfer: r.transferSize, body: r.encodedBodySize, ms: Math.round(r.duration)}))")
        report["startup"] = {"navigation_to_ready_ms": round(total_ms), "module_ms": round(pg.evaluate("window.__t.module_ms")), "scene_fetch_ms": round(pg.evaluate("window.__t.scene_fetch_ms")),
                             "gpu_and_pipelines_ms": round(pg.evaluate("window.__t.start_ms")), "scene_bytes": pg.evaluate("window.__t.scene_bytes")}
        report["assets"] = {"fetched": [r for r in res if r["name"].startswith(("web3d", "scene"))], "textures_or_models_fetched": [r["name"] for r in res if r["name"].endswith((".png", ".glb", ".obj", ".jpg"))]}
        shader_msgs = [t for (k, t) in log if k in ("error", "warning", "pageerror") and any(w in t.lower() for w in ("shader", "wgsl", "validation", "pipeline", "naga"))]
        report["render"] = {"shader_or_pipeline_messages": shader_msgs[:5], "pipelines_built": not shader_msgs}

        # ---- the first picture, read back from the GPU ---------------------------------------------------------------------------------------------------
        t1 = time.time()
        data = pg.evaluate("async () => { const a = await window.__g.snapshot(true); return Array.from(a); }")
        snap_ms = (time.time() - t1) * 1000.0
        colours = len({tuple(data[i:i + 3]) for i in range(0, len(data), 4)})
        png(W, H, data, os.path.join(a.out, "%s.png" % name))
        report["render"].update({"snapshot_ms": round(snap_ms), "distinct_colours": colours, "not_blank": colours >= 8, "picture": os.path.join(a.out, "%s.png" % name),
                                 "chunks_resident": pg.evaluate("window.__g.chunks()")})

        # ---- frame time ----------------------------------------------------------------------------------------------------------------------------------
        mem0 = {"js_heap": pg.evaluate("performance.memory ? performance.memory.usedJSHeapSize : null"), "wasm_memory": pg.evaluate("window.__wasm_memory()")}
        calls = pg.evaluate("""(n) => { const out = []; for (let i = 0; i < n; i++) { const t = performance.now(); window.__g.frame(t + i * 16.7); out.push(performance.now() - t); } return out; }""", a.frames)
        presented = pg.evaluate("""() => new Promise((res) => { let n = 0; const t0 = performance.now(); const loop = (t) => { window.__g.frame(t); n++; if (t - t0 < 3000) requestAnimationFrame(loop); else res({frames: n, ms: t - t0}); }; requestAnimationFrame(loop); })""")
        gpu = []
        for _ in range(3):  # one whole frame, GPU included: draw into a texture and wait for the pixels to come back
            t = time.time()
            try:
                pg.evaluate("async () => { await window.__g.snapshot(false); return 1; }")
            except Exception as e:  # a very slow adapter can miss the player's own 10 s limit: say so, keep measuring
                report.setdefault("errors", []).append("gpu frame: " + str(e).splitlines()[0])
                break
            gpu.append((time.time() - t) * 1000.0)
        report["frame"] = {"frame_call_ms": stats(calls), "gpu_frame_ms": stats(gpu), "presented_frames_in_3s": presented["frames"], "presented_fps": round(presented["frames"] * 1000.0 / presented["ms"], 2),
                           "note": "frame_call_ms is the CPU side of one frame (simulation ticks, recording and submitting the GPU commands); gpu_frame_ms is a whole frame including the GPU and the read-back of its pixels; presented_fps is what requestAnimationFrame delivered. On a software adapter all three describe that adapter at this canvas size, nothing about a laptop or a phone."}
        mem1 = {"js_heap": pg.evaluate("performance.memory ? performance.memory.usedJSHeapSize : null"), "wasm_memory": pg.evaluate("window.__wasm_memory()")}
        report["memory"] = {"before_frames": mem0, "after_frames": mem1, "note": "js_heap is Chromium's usedJSHeapSize; wasm_memory is the module's linear memory (only grows). GPU memory is not measurable from a page."}

        # ---- input: the page's real events reach the module ----------------------------------------------------------------------------------------------
        before = json.loads(pg.evaluate("window.__g.state()"))
        pg.keyboard.down("KeyW")
        pg.wait_for_timeout(300)
        pg.evaluate("window.__g.frame(performance.now())")
        pg.evaluate("window.__g.run(40)")
        pg.keyboard.up("KeyW")
        after = json.loads(pg.evaluate("window.__g.state()"))
        moved = math.dist(before["pos"][::2], after["pos"][::2])
        ydir0 = (after["pos"][0] - before["pos"][0], after["pos"][2] - before["pos"][2])
        pg.evaluate("window.__g.look(400, 0)")
        pg.keyboard.down("KeyW")
        pg.evaluate("window.__g.run(40)")
        pg.keyboard.up("KeyW")
        after2 = json.loads(pg.evaluate("window.__g.state()"))
        ydir1 = (after2["pos"][0] - after["pos"][0], after2["pos"][2] - after["pos"][2])
        cos = (ydir0[0] * ydir1[0] + ydir0[1] * ydir1[1]) / ((math.hypot(*ydir0) * math.hypot(*ydir1)) or 1.0)
        report["input"] = {
            "keyboard": {"ok": moved > 0.5, "detail": "KeyW held through the browser's keyboard events moved the player %.2f m" % moved},
            "mouse_look": {"ok": cos < 0.9, "detail": "after look(400, 0) walking forward goes a different way (cosine between the two headings %.2f)" % cos},
            "touch": {"ok": None, "detail": "not wired in the 3D player yet (the 2D path has the phone pads): not run"},
            "gamepad": {"ok": None, "detail": "not wired in the 3D player yet: not run"},
        }
        ctx.close()

        # ---- parity with the native engine: the same scripted inputs, the same match -------------------------------------------------------------------------
        ctx, pg, log, _ = page("scene=%s&w=320&h=180" % scene_file)
        if pg.evaluate("window.__state") == "ready":
            try:
                first = pg.evaluate("async () => Array.from(await window.__g.snapshot(false))")
            except Exception as e:
                first = None
                report.setdefault("errors", []).append("parity picture before: " + str(e).splitlines()[0])
            for step in a.parity_script.split(";"):
                keys, ticks = step.split(":")
                codes = [k for k in keys.split("+") if k]
                for k in codes:
                    pg.evaluate("window.__g.key(%s, true)" % json.dumps(k))
                pg.evaluate("window.__g.run(%d)" % int(ticks))
                for k in codes:
                    pg.evaluate("window.__g.key(%s, false)" % json.dumps(k))
            got = json.loads(pg.evaluate("window.__g.state()"))
            try:
                last = pg.evaluate("async () => Array.from(await window.__g.snapshot(false))")
                png(320, 180, last, os.path.join(a.out, "%s-after.png" % name))
            except Exception as e:
                last = None
                report.setdefault("errors", []).append("parity picture after: " + str(e).splitlines()[0])
            parity = {"script": a.parity_script, "browser": got, "picture_after": os.path.join(a.out, "%s-after.png" % name) if last else None,
                      "picture_changed_by_play": (first != last) if (first and last) else None}
            if a.expected:
                want = json.load(open(a.expected))
                dpos = math.dist(got["pos"], want["pos"])
                parity.update({"native": want, "same_checksum": got["checksum"] == want["checksum"], "position_difference_m": dpos, "same_tick": got["tick"] == want["tick"],
                               "same_rules_state": got["collision_disabled"] == want["collision_disabled"] and got["hidden"] == want["hidden"] and got["ended"] == want["ended"]})
            report["parity"] = parity
        ctx.close()

        # ---- the WebGL2 fallback: what happens today ----------------------------------------------------------------------------------------------------
        ctx, pg, log, _ = page("scene=%s&w=%d&h=%d&gl=1" % (scene_file, 320, 180))
        report["webgl2"] = {"state": pg.evaluate("window.__state")[:200], "works": pg.evaluate("window.__state") == "ready"}
        ctx.close()
        browser.close()
    srv.shutdown()
    r = report
    r["ok"] = bool(r["render"]["not_blank"] and r["render"]["pipelines_built"] and r["input"]["keyboard"]["ok"] and r["input"]["mouse_look"]["ok"])
    print(json.dumps(r))
    sys.exit(0 if r["ok"] else 1)


if __name__ == "__main__":
    main()
