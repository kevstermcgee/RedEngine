// RedEngine 2D music renderer. Rendering a loop (reverb included) takes from a fraction of a second to a couple of seconds, so it runs here, off the page's main thread:
// the game keeps its 60 ticks a second while the music is made. This worker loads the same game.wasm and game.json as the page, asks the module for the first music track's
// samples (stereo interleaved f32) and hands them back. Nothing else.
'use strict';
let exports_ = null;
const x = () => exports_;

self.onmessage = async () => {
  try {
    const [g, w] = await Promise.all([fetch('assets/game.json'), fetch('game.wasm')]);
    if (!g.ok || !w.ok) throw new Error('could not load the game (HTTP ' + g.status + ', ' + w.status + ')');
    const text = new TextEncoder().encode(await g.text());
    exports_ = (await WebAssembly.instantiate(await w.arrayBuffer(), {})).instance.exports;
    const p = x().alloc(text.length);
    new Uint8Array(x().memory.buffer).set(text, p);
    if (x().init(p, text.length, 1) !== 0) throw new Error('the game was refused in the music worker');
    const t0 = performance.now();
    const n = x().music_pcm();
    const pcm = new Float32Array(x().memory.buffer.slice(x().out_ptr(), x().out_ptr() + n));
    self.postMessage({ ok: true, pcm, rate: x().sample_rate(), ms: Math.round(performance.now() - t0) }, [pcm.buffer]);
  } catch (e) {
    self.postMessage({ ok: false, error: String((e && e.message) || e) });
  }
};
