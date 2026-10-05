import sys, time, io
from playwright.sync_api import sync_playwright
FLAGS=["--enable-unsafe-webgpu","--enable-features=Vulkan,WebGPU","--use-angle=swiftshader","--use-webgpu-adapter=swiftshader","--ignore-gpu-blocklist","--enable-unsafe-swiftshader"]
with sync_playwright() as p:
    b=p.chromium.launch(args=FLAGS)
    for name,q in [("WebGPU","?gl=0"),("WebGL2 (wgpu GL backend)","?gl=1")]:
        pg=b.new_page(viewport={"width":640,"height":360}); logs=[]
        pg.on("console",lambda m:logs.append(m.type+": "+m.text)); pg.on("pageerror",lambda e:logs.append("pageerror: "+str(e)))
        pg.goto("http://127.0.0.1:8765/index.html"+q)
        t=time.time()
        while time.time()-t<20 and pg.evaluate("window.__result") in (None,"starting"): time.sleep(0.2)
        res=pg.evaluate("window.__result"); time.sleep(1.0)
        f0=pg.evaluate("(window.__spike||{}).frames||0"); time.sleep(2.0); f1=pg.evaluate("(window.__spike||{}).frames||0")
        probe = pg.evaluate("""() => new Promise(res => requestAnimationFrame(() => {
            const c = document.getElementById('c'); const t = document.createElement('canvas'); t.width = c.width; t.height = c.height;
            const x = t.getContext('2d'); x.drawImage(c, 0, 0); const d = x.getImageData(0, 0, t.width, t.height).data;
            const seen = new Set(); let lit = 0;
            for (let i = 0; i < d.length; i += 4) { seen.add(d[i] + ',' + d[i+1] + ',' + d[i+2]); if (d[i] + d[i+1] + d[i+2] > 400) lit++; }
            res({distinct: seen.size, center: [...d.slice((180*640+320)*4, (180*640+320)*4+4)], corner: [...d.slice(0,4)]});
        }))""")
        print("   canvas readback:", probe)
        png=pg.screenshot(path=f"out/wgpu-web-spike-shot-{'gl' if 'GL' in name else 'gpu'}.png"); cols='see file'
        print("   offscreen readback:", pg.evaluate("window.__probe || null"))
        print(f"{name}: {res}\n   frames in 2 s: {f1-f0}  distinct colours: {cols}  logs: {logs[:4]}")
    b.close()
