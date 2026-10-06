import sys, time, zlib, struct, json
from playwright.sync_api import sync_playwright
FLAGS=["--enable-unsafe-webgpu","--enable-features=Vulkan,WebGPU","--use-angle=swiftshader","--use-webgpu-adapter=swiftshader","--ignore-gpu-blocklist","--enable-unsafe-swiftshader"]
def png(w,h,rgba,path):
    raw=b"".join(b"\x00"+bytes(rgba[y*w*4:(y+1)*w*4]) for y in range(h))
    def ch(t,d): c=struct.pack(">I",len(d))+t+d; return c+struct.pack(">I",zlib.crc32(t+d)&0xffffffff)
    open(path,"wb").write(b"\x89PNG\r\n\x1a\n"+ch(b"IHDR",struct.pack(">IIBBBBB",w,h,8,6,0,0,0))+ch(b"IDAT",zlib.compress(raw,6))+ch(b"IEND",b""))
with sync_playwright() as p:
    b=p.chromium.launch(args=FLAGS)
    for name,q in [("webgpu","")]:  # the WebGL2 fallback is not built yet (docs/analysis/2026-10-05-3d-in-the-browser.md)
        pg=b.new_page(viewport={"width":960,"height":540}); logs=[]
        pg.on("console",lambda m:logs.append(m.type+": "+m.text)); pg.on("pageerror",lambda e:logs.append("pageerror: "+str(e)))
        t0=time.time(); pg.goto("http://127.0.0.1:8770/index.html"+q)
        while time.time()-t0<120 and pg.evaluate("window.__state")=="loading": time.sleep(0.3)
        st=pg.evaluate("window.__state"); print(name,"state:",st,"| info:",pg.evaluate("window.__info"),f"| load {time.time()-t0:.1f}s")
        if st=="ready":
            t1=time.time()
            data=pg.evaluate("async () => { const a = await window.__g.snapshot(true); return Array.from(a); }")
            cols=len({tuple(data[i:i+3]) for i in range(0,len(data),4)})
            print(f"   snapshot {time.time()-t1:.1f}s, {len(data)//4} px, {cols} distinct colours, chunks resident {pg.evaluate('window.__g.chunks()')}, pos {pg.evaluate('Array.from(window.__g.position())')}")
            png(960,540,data,f"out/web3d-{name}.png")
        print("   logs:",[l for l in logs if 'GL Driver' not in l][:5])
    b.close()
