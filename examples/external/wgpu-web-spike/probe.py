import sys, json
from playwright.sync_api import sync_playwright
JS = """async () => {
  const out = {webgl2: false, webgpu_api: !!navigator.gpu, adapter: null};
  const c = document.createElement('canvas'); const gl = c.getContext('webgl2');
  if (gl) { out.webgl2 = true; const d = gl.getExtension('WEBGL_debug_renderer_info'); out.gl_renderer = d ? gl.getParameter(d.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER); out.max_tex = gl.getParameter(gl.MAX_TEXTURE_SIZE); }
  if (navigator.gpu) { try { const a = await navigator.gpu.requestAdapter(); if (a) { out.adapter = {features:[...a.features].length, maxTex:a.limits.maxTextureDimension2D, maxStorageBuf: a.limits.maxStorageBuffersPerShaderStage, info: a.info ? {vendor:a.info.vendor, arch:a.info.architecture, desc:a.info.description} : null, fallback: a.isFallbackAdapter}; } } catch(e) { out.adapter_err = String(e); } }
  return out; }"""
variants = {"default": [], "webgpu flags": ["--enable-unsafe-webgpu", "--enable-features=Vulkan,WebGPU", "--use-angle=swiftshader", "--use-webgpu-adapter=swiftshader", "--ignore-gpu-blocklist"], "swiftshader gl": ["--use-gl=angle", "--use-angle=swiftshader", "--enable-unsafe-swiftshader"]}
with sync_playwright() as p:
    for name, args in variants.items():
        b = p.chromium.launch(args=args)
        pg = b.new_page(); pg.goto("about:blank")
        pg.goto("http://localhost:1/") if False else None
        ctx = b.new_context(); pg2 = ctx.new_page()
        pg2.goto("http://127.0.0.1:8765/index.html")
        try:
            print(name, json.dumps(pg2.evaluate(JS)))
        except Exception as e:
            print(name, "ERR", e)
        b.close()
