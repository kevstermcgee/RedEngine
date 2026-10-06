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
    offline: 'unknown',        // unknown | installing | ready | unavailable (a service worker stored the whole game: it plays with no network)
    installable: false,        // the browser offered to install it as an app
    standalone: false,         // running as an installed app
    persistent: null,          // true when the browser promised not to clear this game's storage
    quota: null,
    backup: '',
    pad: 'hidden',             // hidden | shown (the touch controller below the game)
    held: [],                  // input actions the touch controller is holding right now
  };
  let wasm = null, mem = null, canvas, stage, ctx2d, imageData = null, manifest = null;
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
  function relayout() {                                // the picture is fitted into the stage: the whole window, or what is left above the touch controller
    x().window(stage.clientWidth, stage.clientHeight);
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
    ensureAudio(); requestDurableStorage();
    last = performance.now();
  }
  function toView(clientX, clientY) {                  // the module owns the mapping, so a click lands where the picture says it does
    const r = stage.getBoundingClientRect();
    return x().to_view(clientX - r.left, clientY - r.top) ? [x().view_x(), x().view_y()] : null;
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
  window.addEventListener('blur', () => { // never leave a key (or a thumb) stuck when focus is lost
    for (const a of ['left', 'right', 'up', 'down', 'action', 'secondary', 'pause']) withText(a, (p, n) => x().action(p, n, 0));
    held.clear(); status.held = []; padVisual();
  });
  function bindPointer() {
    window.addEventListener('pointermove', (e) => { if (!wasm) return; const p = toView(e.clientX, e.clientY); if (p) x().pointer(p[0], p[1]); });
    window.addEventListener('pointerdown', (e) => {
      if (!wasm || status.state === 'error') return;
      if (e.target.closest && e.target.closest('[data-nostart]')) return;      // install / backup links on the start card
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

  // ---- install, offline, durable storage, backup ------------------------------------------------------------------------------------------------------
  // The page is also an installable app: a web app manifest + icons + a service worker (sw.js) that stores the package, so an installed game plays with no network and
  // keeps its progress. The browser decides how to offer it; on iOS the player is shown the Share menu hint.
  let installEvent = null;
  const isStandalone = () => (window.matchMedia && window.matchMedia('(display-mode: standalone)').matches) || navigator.standalone === true;
  function setupInstall() {
    status.standalone = isStandalone();
    const btn = document.getElementById('install');
    window.addEventListener('beforeinstallprompt', (e) => { e.preventDefault(); installEvent = e; status.installable = true; if (btn && !status.standalone) btn.hidden = false; });
    window.addEventListener('appinstalled', () => { status.standalone = true; if (btn) btn.hidden = true; });
    if (btn) btn.addEventListener('click', async () => { if (!installEvent) return; installEvent.prompt(); try { await installEvent.userChoice; } catch (e) { /* dismissed */ } installEvent = null; btn.hidden = true; });
    const hint = document.getElementById('ioshint');
    if (hint && /iphone|ipad|ipod/i.test(navigator.userAgent) && !status.standalone) hint.hidden = false;
    if ('serviceWorker' in navigator && window.isSecureContext) {
      status.offline = 'installing';
      navigator.serviceWorker.register('sw.js').then(() => navigator.serviceWorker.ready).then(() => { status.offline = 'ready'; }).catch((e) => { status.offline = 'unavailable'; console.warn('red2d: offline support unavailable: ' + (e && e.message)); });
    } else { status.offline = 'unavailable'; }
  }
  function requestDurableStorage() {                   // after the first gesture: ask the browser not to evict this game's saves under storage pressure
    const s = navigator.storage;
    if (!s) return;
    if (s.persist) s.persist().then((ok) => { status.persistent = !!ok; }).catch(() => { status.persistent = false; });
    if (s.estimate) s.estimate().then((e) => { status.quota = { used: e.usage, quota: e.quota }; }).catch(() => {});
  }
  function backupText() {
    const raw = status.storage === 'ok' ? localStorage.getItem(storageKey) : null;
    return JSON.stringify({ red2d_backup: 1, game: manifest.game.id, save: raw });
  }
  function restoreText(text) {                         // returns '' on success, else why not
    let b; try { b = JSON.parse(text); } catch (e) { return 'that file is not a RedEngine backup (not JSON)'; }
    if (!b || b.red2d_backup !== 1) return 'that file is not a RedEngine backup';
    if (b.game !== manifest.game.id) return 'that backup is for the game "' + b.game + '", not this one';
    if (typeof b.save !== 'string') return 'that backup holds no progress yet';
    if (status.storage !== 'ok') return 'this browser is not letting the game store anything (private mode?)';
    try { localStorage.setItem(storageKey, b.save); } catch (e) { return 'could not store the backup (' + (e && e.name) + ')'; }
    return '';
  }
  function setupBackup() {
    const wrap = document.getElementById('saves');
    if (!wrap || !(manifest.game.persistence || []).length) return;
    wrap.hidden = false;
    document.getElementById('backup').addEventListener('click', () => {
      const a = document.createElement('a');
      a.href = URL.createObjectURL(new Blob([backupText()], { type: 'application/json' }));
      a.download = manifest.game.id + '-progress.json';
      document.body.appendChild(a); a.click(); a.remove();
      status.backup = 'saved';
    });
    const file = document.getElementById('restorefile');
    document.getElementById('restore').addEventListener('click', () => file.click());
    file.addEventListener('change', async () => {
      const f = file.files && file.files[0]; if (!f) return;
      const why = restoreText(await f.text());
      status.backup = why ? 'failed: ' + why : 'restored';
      if (why) alert(why); else location.reload();
    });
  }

  // ---- the touch controller ---------------------------------------------------------------------------------------------------------------------------
  // Drawn by the page BELOW the game, only when the main pointer is a finger (`pointer: coarse`); a desktop has none and keeps keyboard, mouse and gamepad. A pad holds and
  // releases the same input actions as the keyboard and a gamepad, so the module cannot tell them apart. The layout comes from the manifest (`game.controls`).
  const held = new Set();
  const coarse = window.matchMedia ? window.matchMedia('(pointer: coarse)') : { matches: false };
  function setHeld(name, on) {
    if (held.has(name) === on) return;
    if (on) held.add(name); else held.delete(name);
    withText(name, (p, n) => x().action(p, n, on ? 1 : 0));
    status.held = [...held];
    if (on && navigator.vibrate) { try { navigator.vibrate(8); } catch (e) { /* not everywhere */ } }
  }
  function padVisual() {
    for (const e of document.querySelectorAll('#pad [data-action]')) e.classList.toggle('on', held.has(e.dataset.action));
    for (const d of document.querySelectorAll('#pad .dir')) for (const a of ['left', 'right', 'up', 'down']) d.classList.toggle('on-' + a, held.has(a));
  }
  const make = (tag, cls, text) => { const e = document.createElement(tag); if (cls) e.className = cls; if (text) e.textContent = text; return e; };
  function bindDir(elm, horizontalOnly) {
    let pid = null;
    const knob = elm.querySelector('.knob');
    const update = (e) => {
      const r = elm.getBoundingClientRect();
      const dx = (e.clientX - (r.left + r.width / 2)) / (r.width / 2), dy = (e.clientY - (r.top + r.height / 2)) / (r.height / 2);
      const dead = horizontalOnly ? 0.05 : 0.24;
      let l = dx < -dead, rt = dx > dead, u = dy < -dead, d = dy > dead;
      if (horizontalOnly) { u = d = false; }
      else if ((l || rt) && (u || d)) {                // a diagonal only when the thumb is clearly between two directions
        const ax = Math.abs(dx), ay = Math.abs(dy);
        if (Math.min(ax, ay) / Math.max(ax, ay) < 0.5) { if (ax > ay) { u = d = false; } else { l = rt = false; } }
      }
      setHeld('left', l); setHeld('right', rt); setHeld('up', u); setHeld('down', d);
      if (knob) knob.style.transform = 'translate(' + Math.max(-1, Math.min(1, dx)) * 28 + '%,' + Math.max(-1, Math.min(1, dy)) * 28 + '%)';
      padVisual();
    };
    const end = (e) => {
      if (e.pointerId !== pid) return;
      pid = null;
      for (const a of ['left', 'right', 'up', 'down']) setHeld(a, false);
      if (knob) knob.style.transform = '';
      padVisual();
    };
    elm.addEventListener('pointerdown', (e) => { e.preventDefault(); if (!status.started) { begin(); return; } pid = e.pointerId; elm.setPointerCapture(pid); update(e); });
    elm.addEventListener('pointermove', (e) => { if (e.pointerId === pid) update(e); });
    elm.addEventListener('pointerup', end); elm.addEventListener('pointercancel', end); elm.addEventListener('lostpointercapture', end);
    elm.addEventListener('contextmenu', (e) => e.preventDefault());
  }
  function bindButton(elm, action, tap) {
    let pid = null;
    const off = (e) => { if (e.pointerId !== pid) return; pid = null; setHeld(action, false); padVisual(); };
    elm.addEventListener('pointerdown', (e) => {
      e.preventDefault(); if (!status.started) { begin(); return; }
      pid = e.pointerId; elm.setPointerCapture(pid); setHeld(action, true); padVisual();
      if (tap) setTimeout(() => { pid = null; setHeld(action, false); padVisual(); }, 80);   // pause is a press, not a hold
    });
    elm.addEventListener('pointerup', off); elm.addEventListener('pointercancel', off); elm.addEventListener('lostpointercapture', off);
    elm.addEventListener('contextmenu', (e) => e.preventDefault());
  }
  function buildPad(c) {
    const pad = document.getElementById('pad');
    pad.textContent = ''; pad.dataset.layout = c.layout;
    const left = make('div', 'side left'), mid = make('div', 'mid'), right = make('div', 'side right');
    if (c.layout === 'dpad' || c.layout === 'stick') {
      const d = make('div', 'dir ' + c.layout);
      if (c.layout === 'dpad') { d.append(make('i', 'up', '\u25B2'), make('i', 'down', '\u25BC'), make('i', 'left', '\u25C0'), make('i', 'right', '\u25B6')); }
      else { d.append(make('b', 'knob')); }
      d.dataset.testid = 'dir'; left.append(d); bindDir(d, false);
    } else if (c.layout === 'platformer' || c.layout === 'lr') {
      const d = make('div', 'dir lr'); d.append(make('i', 'left', '\u25C0'), make('i', 'right', '\u25B6'));
      d.dataset.testid = 'dir'; left.append(d); bindDir(d, true);
    }
    for (const b of [...c.buttons].reverse()) {       // B then A, so A is the outermost on the right
      const e = make('div', 'btn ' + b.id, b.label); e.dataset.action = b.action; e.dataset.testid = 'btn-' + b.id; right.append(e); bindButton(e, b.action, false);
    }
    if (c.pause) { const p = make('div', 'pause', 'PAUSE'); p.dataset.action = 'pause'; p.dataset.testid = 'pause'; mid.append(p); bindButton(p, 'pause', true); }
    pad.append(left, mid, right);
  }
  function applyPad() {
    const c = manifest && manifest.game && manifest.game.controls;
    const q = params.get('pad');                      // ?pad=1 forces the controller on a desktop (for testing), ?pad=0 turns it off
    const touchDevice = q === '1' ? true : q === '0' ? false : coarse.matches;
    document.body.classList.toggle('touch', touchDevice);
    const on = !!(c && c.visible && touchDevice);
    const pad = document.getElementById('pad');
    if (on && !pad.firstChild) buildPad(c);
    pad.hidden = !on; status.pad = on ? 'shown' : 'hidden';
    if (!on) { for (const a of [...held]) setHeld(a, false); }
    if (wasm) { relayout(); draw(); }
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
      canvas = document.getElementById('screen'); stage = document.getElementById('stage');
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
      applyPad(); setupInstall(); setupBackup();
      relayout(); draw();
      if (window.ResizeObserver) new ResizeObserver(() => { relayout(); draw(); }).observe(stage); else window.addEventListener('resize', () => { relayout(); draw(); });
      if (coarse.addEventListener) coarse.addEventListener('change', applyPad);
      document.addEventListener('visibilitychange', () => { if (document.hidden) { for (const a of [...held]) setHeld(a, false); padVisual(); } });
      // iOS only lets a gesture END resume audio; the first touch begins the game, and any touch afterwards resumes a suspended context.
      window.addEventListener('touchend', () => { if (audio && audio.state === 'suspended') audio.resume(); }, { passive: true });
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
    toView: (cx, cy) => toView(cx, cy),
    held: () => [...held],
    backupText, restoreText,
    padInfo: () => { const p = document.getElementById('pad'); const r = p.getBoundingClientRect(); return { shown: !p.hidden, x: r.left, y: r.top, w: r.width, h: r.height }; },
    testSound: (i) => {                                       // run sound i through the real Web Audio pipeline: samples from the module, an AudioBuffer, a source node
      ensureAudio(); if (!audio) return { ok: false, reason: 'no audio context: ' + status.audio };
      const b = soundBuffer(i); if (!b) return { ok: false, reason: 'the module could not render sound ' + i };
      const s = audio.createBufferSource(); s.buffer = b; s.connect(audio.destination); s.start();
      return { ok: true, seconds: b.duration, rate: b.sampleRate, state: audio.state };
    },
    resetSave: () => { try { localStorage.removeItem(storageKey); } catch (e) { /* nothing to remove */ } },
    layout: () => { const r = stage.getBoundingClientRect(); return { x: x().layout_x() + r.left, y: x().layout_y() + r.top, w: x().layout_w(), h: x().layout_h() }; },
  };
  main();
})();
