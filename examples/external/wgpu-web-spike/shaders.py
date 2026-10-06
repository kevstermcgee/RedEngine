import json
from playwright.sync_api import sync_playwright
S = "src/shaders/"  # run from the repository root
r = lambda n: open(S + n + ".wgsl").read()
mods = {
    "scene": r("common") + r("scene"), "shadow": r("common") + r("shadow"), "sky+background": r("common") + r("sky") + r("background"),
    "ocean": r("common") + r("sky") + r("ocean"), "crosshair": r("crosshair"),
    "postfx(single)": r("postfx").replace("DEPTH_TEXTURE_TYPE", "texture_depth_2d"),
    "postfx(msaa)": r("postfx").replace("DEPTH_TEXTURE_TYPE", "texture_depth_multisampled_2d"),
    "fx": r("fx"), "overlay": r("overlay"),
}
JS="""async (mods) => {
  const a = await navigator.gpu.requestAdapter(); const d = await a.requestDevice(); const res = {};
  for (const [k, src] of Object.entries(mods)) {
    const m = d.createShaderModule({code: src}); const info = await m.getCompilationInfo();
    res[k] = info.messages.filter(x => x.type === 'error').map(x => x.lineNum + ': ' + x.message);
  }
  return res; }"""
with sync_playwright() as p:
    b=p.chromium.launch(args=["--enable-unsafe-webgpu","--enable-features=Vulkan,WebGPU","--use-angle=swiftshader","--use-webgpu-adapter=swiftshader","--ignore-gpu-blocklist"])
    pg=b.new_page(); pg.goto("http://127.0.0.1:8765/index.html")
    for k,v in pg.evaluate(JS,mods).items(): print("OK  " if not v else "FAIL", k, v[:3])
    b.close()
