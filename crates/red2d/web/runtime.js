// RedEngine 2D web runtime. One file, no dependencies, no build step.
// It owns what a browser owns: the animation-frame loop, the canvas, key/pointer/gamepad events, localStorage and Web Audio.
// Everything about the game itself (rules, physics, drawing, sound synthesis) is in game.wasm.
// Test hooks: window.__red2d (status(), snapshot(), advance(n), ...) and ?paused=1 (the page does not advance by itself).
(() => {
  'use strict';
  const params = new URLSearchParams(location.search);
  const paused = params.has('paused');
  const status = {
    state: 'loading',          // loading | ready | running | error
    error: null,
    wasm: false,               // module instantiated
    game: null,                // {id,title,...} from manifest
    started: false,            // the player pressed something
    frames: 0,
    ticks: 0,
    storage: 'unknown',        // ok | unavailable
    save: 'none',              // none | fresh | loaded | incompatible | unreadable
    saveMessage: '',
    saves: 0,                  // writes to storage
    audio: 'locked',           // locked | running | suspended | unsupported | failed
    sounds_played: 0,          // sound requests handed to Web Audio
    music: 'off',              // off | rendering | playing | none | failed
    music_ms: 0,               // how long the loop took to render
    music_blocked_ms: 0,       // how long that stalled the page (a few ms: the loop is rendered in a worker)
    music_worker: null,        // true when audio-worker.js rendered it
    gamepad: false,
  };
  let wasm = null, mem = null, canvas, ctx2d, imageData = null, manifest = null;
  let audio = null, soundBuffers = new Map(), musicSource = null, musicBuffer = null, musicWanted = false;
  let storageKey = null, last = 0, acc = 0, rafId = 0;
  const TICK_MS = 1000 / 60;
  const enc = new TextEncoder(), dec = new TextDecoder();

  const fail = (e) => {
    const msg = (e && e.message) ? e.message : String(e);
    status.state = 'error'; status.error = msg;
    document.body.dataset.state = 'error';
    console.error('red2d fatal: ' + msg);
    const box = document.getElementById('error');
    if (box) { box.textContent = 'The game could not run: ' + msg; box.hidden = false; }
  };
  window.addEventListener('error', (e) => fail(e.error || e.message));
  window.addEventListener('unhandledrejection', (e) => fail(e.reason));

  // ---- wasm helpers ---------------------------------------------------------------------------------------------------------------------------------
  const x = () => wasm.exports;
  const view = () => new Uint8Array(x().memory.buffer);
  function put(bytes) { const p = x().alloc(bytes.length); view().set(bytes, p); return p; }
  function withText(s, f) { const b = enc.encode(s); const p = put(b); try { return f(p, b.length); } finally { x().dealloc(p, b.length); } }
  function out(len) { return view().slice(x().out_ptr(), x().out_ptr() + len); }
  function outText(len) { return dec.decode(out(len)); }

  // ---- layout, canvas, drawing -----------------------------------------------------------------------------------------------------------------------
  function relayout() {
    x().window(window.innerWidth, window.innerHeight);
    const l = { x: x().layout_x(), y: x().layout_y(), w: x().layout_w(), h: x().layout_h() };
    canvas.style.left = l.x + 'px'; canvas.style.top = l.y + 'px';
    canvas.style.width = l.w + 'px'; canvas.style.height = l.h + 'px';
    return l;
  }
  function draw() {
    x().render();
    const w = x().view_w(), h = x().view_h(), p = x().frame_ptr(), n = x().frame_len();
    if (!imageData || imageData.width !== w || imageData.height !== h) imageData = new ImageData(w, h);
    imageData.data.set(new Uint8ClampedArray(x().memory.buffer, p, n));
    ctx2d.putImageData(imageData, 0, 0);
    status.frames++;
  }
  function snapshot() { return JSON.parse(outText(x().snapshot())); }

  // ---- persistence (localStorage, behind try/catch: it can be missing, full or blocked) ----------------------------------------------------------------
  function storageProbe() {
    try { const k = storageKey + ':probe'; localStorage.setItem(k, '1'); localStorage.removeItem(k); status.storage = 'ok'; }
    catch (e) { status.storage = 'unavailable'; status.saveMessage = 'storage unavailable: progress will not be kept (' + (e && e.name) + ')'; }
  }
  function loadSave() {
    if (status.storage !== 'ok') return;
    let raw = null;
    try { raw = localStorage.getItem(storageKey); } catch (e) { status.storage = 'unavailable'; return; }
    if (raw === null) { status.save = 'fresh'; return; }
    const code = withText(raw, (p, n) => x().save_load(p, n));
    const msg = outText(x().out_len());
    status.save = ['fresh', 'loaded', 'incompatible', 'unreadable'][code];
    status.saveMessage = msg;
    if (code >= 2) { try { localStorage.setItem(storageKey + ':unreadable', raw); } catch (e) { /* best effort backup */ } }
  }
  function flushSave() {
    const n = x().save_take();
    if (!n) return;
    const text = outText(n);
    if (status.storage !== 'ok') return;
    try { localStorage.setItem(storageKey, text); status.saves++; }
    catch (e) { status.storage = 'unavailable'; status.saveMessage = 'could not save (' + (e && e.name) + '): progress will not be kept'; console.warn('red2d: ' + status.saveMessage); }
  }

  // ---- audio (Web Audio; created only inside a user gesture) -----------------------------------------------------------------------------------------
  function ensureAudio() {
    if (audio || status.audio === 'unsupported') return;
    const AC = window.AudioContext || window.webkitAudioContext;
    if (!AC) { status.audio = 'unsupported'; return; }
    try {
      audio = new AC({ sampleRate: x().sample_rate() });
      audio.resume().then(() => { status.audio = audio.state; }).catch(() => { status.audio = 'failed'; });
      audio.onstatechange = () => { status.audio = audio.state; };
      status.audio = audio.state;
    } catch (e) { status.audio = 'failed'; console.warn('red2d: audio failed: ' + e); }
  }
  function soundBuffer(i) {
    if (soundBuffers.has(i)) return soundBuffers.get(i);
    const n = x().sound_pcm(i);
    if (!n) { soundBuffers.set(i, null); return null; }
    const f = new Float32Array(out(n).buffer);
    const b = audio.createBuffer(1, f.length, x().sample_rate());
    b.copyToChannel(f, 0);
    soundBuffers.set(i, b);
    return b;
  }
  function playSounds() {
    const n = x().sounds_take();
    if (!n) return;
    const idx = new Uint32Array(out(n * 4).buffer);
    if (!audio || audio.state !== 'running') return;       // before the first gesture there is nothing to play with
    for (const i of idx) {
      const b = soundBuffer(i);
      if (!b) continue;
      const s = audio.createBufferSource(); s.buffer = b; s.connect(audio.destination); s.start();
      status.sounds_played++;
    }
  }
  // The loop is made in a worker (audio-worker.js) so rendering it never stalls the game; if a worker cannot be started it is made here, and `music_blocked_ms` says what that cost.
  let musicWorker = null, musicRequested = false;
  function startMusic() {
    if (!musicBuffer || musicSource || !audio || audio.state !== 'running' || x().music_on() !== 1) return;
    musicSource = audio.createBufferSource(); musicSource.buffer = musicBuffer; musicSource.loop = true;
    musicSource.connect(audio.destination); musicSource.start(); status.music = 'playing';
  }
  function haveMusic(pcm, rate, ms, blocked) {
    if (!pcm.length) { status.music = 'none'; return; }
    musicBuffer = audio.createBuffer(2, pcm.length / 2, rate);
    const l = musicBuffer.getChannelData(0), r = musicBuffer.getChannelData(1);
    for (let i = 0; i < l.length; i++) { l[i] = pcm[2 * i]; r[i] = pcm[2 * i + 1]; }
    status.music_ms = ms; status.music_blocked_ms = blocked;
    startMusic();
  }
  function requestMusic() {
    musicRequested = true; status.music = 'rendering';
    try {
      musicWorker = new Worker('audio-worker.js');
      status.music_worker = true;
      musicWorker.onmessage = (e) => {
        musicWorker.terminate(); musicWorker = null;
        if (!e.data.ok) { status.music = 'failed'; console.warn('red2d: music could not be rendered: ' + e.data.error); return; }
        const t0 = performance.now();
        haveMusic(e.data.pcm, e.data.rate, e.data.ms, Math.round(performance.now() - t0));
      };
      musicWorker.onerror = () => { musicWorker = null; status.music_worker = false; renderMusicHere(); };
      musicWorker.postMessage('render');
    } catch (e) { status.music_worker = false; renderMusicHere(); }
  }
  function renderMusicHere() {
    const t0 = performance.now();
    const n = x().music_pcm();
    const f = n ? new Float32Array(out(n).buffer) : new Float32Array(0);
    const ms = Math.round(performance.now() - t0);
    haveMusic(f, x().sample_rate(), ms, ms);
  }
  function syncMusic() {
    const want = x().music_on() === 1 && !!audio && audio.state === 'running' && status.started;
    if (want && !musicSource) {
      if (musicBuffer) startMusic();
      else if (!musicRequested) requestMusic();
    } else if (!want && musicSource) {
      musicSource.stop(); musicSource.disconnect(); musicSource = null; status.music = 'off';
    }
  }

  // ---- input --------------------------------------------------------------------------------------------------------------------------------------------
  function begin() {
    if (status.started || status.state === 'error') return;
    status.started = true; status.state = 'running'; document.body.dataset.state = 'running';
    const o = document.getElementById('start'); if (o) o.hidden = true;
    ensureAudio();
    last = performance.now();
  }
  function toView(clientX, clientY) {                  // the module owns the mapping, so a click lands where the picture says it does
    return x().to_view(clientX, clientY) ? [x().view_x(), x().view_y()] : null;
  }
  function onKey(e, down) {
    if (status.state === 'error') return;
    if (down && !status.started) { begin(); e.preventDefault(); return; }   // the first key press only starts (it unlocks audio)
    if (e.repeat) { e.preventDefault(); return; }
    withText(e.code, (p, n) => x().key(p, n, down ? 1 : 0));
    if (['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Space'].includes(e.code)) e.preventDefault();
  }
  window.addEventListener('keydown', (e) => onKey(e, true));
  window.addEventListener('keyup', (e) => onKey(e, false));
  window.addEventListener('blur', () => { // never leave a key stuck when focus is lost
    for (const a of ['left', 'right', 'up', 'down', 'action', 'secondary', 'pause']) withText(a, (p, n) => x().action(p, n, 0));
  });
  function bindPointer() {
    window.addEventListener('pointermove', (e) => { if (!wasm) return; const p = toView(e.clientX, e.clientY); if (p) x().pointer(p[0], p[1]); });
    window.addEventListener('pointerdown', (e) => {
      if (!wasm || status.state === 'error') return;
      if (!status.started) { begin(); return; }
      const p = toView(e.clientX, e.clientY); if (p) x().click(p[0], p[1]);
    });
  }
  const padState = {};
  function pollGamepad() {
    const pads = navigator.getGamepads ? navigator.getGamepads() : [];
    const pad = [...pads].find((g) => g && g.connected);
    if (!pad) return;
    status.gamepad = true;
    const ax = pad.axes || [], b = (i) => !!(pad.buttons[i] && pad.buttons[i].pressed);
    const want = {
      left: (ax[0] || 0) < -0.5 || b(14), right: (ax[0] || 0) > 0.5 || b(15),
      up: (ax[1] || 0) < -0.5 || b(12), down: (ax[1] || 0) > 0.5 || b(13),
      action: b(0), secondary: b(1), pause: b(9),
    };
    if (Object.values(want).some(Boolean) && !status.started) begin();
    for (const [a, on] of Object.entries(want)) {
      if (padState[a] !== on) { padState[a] = on; withText(a, (p, n) => x().action(p, n, on ? 1 : 0)); }
    }
  }

  // ---- the loop ------------------------------------------------------------------------------------------------------------------------------------------
  function advance(n) {                                // run n ticks and show them (also the test hook)
    x().step(n); status.ticks += n; draw(); playSounds(); syncMusic(); flushSave();
  }
  function frame(now) {
    rafId = requestAnimationFrame(frame);
    try {
      pollGamepad();
      if (status.started) {
        acc += Math.min(now - last, 100); last = now;
        let n = 0;
        while (acc >= TICK_MS && n < 6) { acc -= TICK_MS; n++; }
        if (n) advance(n); else { draw(); }
      } else { last = now; draw(); }
    } catch (e) { cancelAnimationFrame(rafId); fail(e); }
  }

  async function main() {
    try {
      canvas = document.getElementById('screen');
      ctx2d = canvas.getContext('2d', { alpha: false });
      const [mres, gres, wres] = await Promise.all([fetch('manifest.json'), fetch('assets/game.json'), fetch('game.wasm')]);
      for (const [name, r] of [['manifest.json', mres], ['assets/game.json', gres], ['game.wasm', wres]]) if (!r.ok) throw new Error('could not load ' + name + ' (HTTP ' + r.status + ')');
      manifest = await mres.json();
      const gameText = await gres.text();
      const bytes = await wres.arrayBuffer();
      wasm = (await WebAssembly.instantiate(bytes, {})).instance;
      status.wasm = true; status.game = manifest.game || null;
      const seed = (Number(params.get('seed')) || 1) >>> 0;
      const rc = withText(gameText, (p, n) => x().init(p, n, seed));
      if (rc !== 0) throw new Error('the game was refused:\n' + outText(x().error()));
      canvas.width = x().view_w(); canvas.height = x().view_h();
      storageKey = 'red2d:' + manifest.game.id;
      storageProbe(); loadSave();
      relayout(); window.addEventListener('resize', () => { relayout(); draw(); });
      bindPointer();
      draw();
      const t = document.getElementById('title'); if (t) t.textContent = manifest.game.title;
      status.state = 'ready'; document.body.dataset.state = 'ready';
      const o = document.getElementById('start'); if (o) o.hidden = false;
      document.getElementById('loading').hidden = true;
      if (!paused) rafId = requestAnimationFrame(frame);
    } catch (e) { fail(e); }
  }

  window.__red2d = {
    status: () => JSON.parse(JSON.stringify(status)),
    snapshot: () => snapshot(),
    manifest: () => manifest,
    advance: (n) => { advance(n | 0); return snapshot(); },     // with ?paused=1 the page advances only when asked
    begin,
    scenarios: () => JSON.parse(outText(x().scenarios())),
    pixels: () => {                                           // the canvas as it is right now, base64 RGBA (what getImageData returns)
      const w = x().view_w(), h = x().view_h(), d = ctx2d.getImageData(0, 0, w, h).data; let s = '';
      for (let i = 0; i < d.length; i += 0x8000) s += String.fromCharCode.apply(null, d.subarray(i, i + 0x8000));
      return { w, h, b64: btoa(s) };
    },
    toView: (cx, cy) => (x().to_view(cx, cy) ? [x().view_x(), x().view_y()] : null),
    testSound: (i) => {                                       // run sound i through the real Web Audio pipeline: samples from the module, an AudioBuffer, a source node
      ensureAudio(); if (!audio) return { ok: false, reason: 'no audio context: ' + status.audio };
      const b = soundBuffer(i); if (!b) return { ok: false, reason: 'the module could not render sound ' + i };
      const s = audio.createBufferSource(); s.buffer = b; s.connect(audio.destination); s.start();
      return { ok: true, seconds: b.duration, rate: b.sampleRate, state: audio.state };
    },
    resetSave: () => { try { localStorage.removeItem(storageKey); } catch (e) { /* nothing to remove */ } },
    layout: () => ({ x: x().layout_x(), y: x().layout_y(), w: x().layout_w(), h: x().layout_h() }),
  };
  main();
})();
