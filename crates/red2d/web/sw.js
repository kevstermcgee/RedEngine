// RedEngine 2D service worker: makes the game an installable app that also plays with no network.
// On install it stores every file of the package (the list is manifest.json's `files`). After that every request goes to the network first and falls back to the stored
// copy, refreshing it on each success, so an online player always gets the newest build and an offline one gets the last build they had. Nothing else is cached.
'use strict';
const CACHE = 'red2d:' + new URL(self.registration.scope).pathname;
const sleep = (ms) => new Promise((_, reject) => setTimeout(() => reject(new Error('timeout')), ms));
const keyOf = (url) => { const u = new URL(url); u.search = ''; u.hash = ''; return u.href; };   // ?paused=1 and friends are the same file

self.addEventListener('install', (event) => {
  event.waitUntil((async () => {
    const m = await (await fetch('manifest.json', { cache: 'reload' })).json();
    const cache = await caches.open(CACHE);
    for (const path of ['./', 'manifest.json', ...m.files.map((f) => f.path)]) {
      const res = await fetch(new Request(path, { cache: 'reload' }));
      if (!res.ok) throw new Error('could not store ' + path + ' (HTTP ' + res.status + ')');
      await cache.put(keyOf(new URL(path, self.registration.scope)), res);
    }
    await self.skipWaiting();
  })());
});

self.addEventListener('activate', (event) => { event.waitUntil(self.clients.claim()); });

self.addEventListener('fetch', (event) => {
  const req = event.request;
  if (req.method !== 'GET' || new URL(req.url).origin !== self.location.origin) return;
  event.respondWith((async () => {
    const cache = await caches.open(CACHE);
    const key = keyOf(req.url);
    try {
      const res = await Promise.race([fetch(req), sleep(4000)]);
      if (res && res.ok) cache.put(key, res.clone());
      return res;
    } catch (e) {
      const hit = await cache.match(key);
      if (hit) return hit;
      if (req.mode === 'navigate') { const page = await cache.match(keyOf(new URL('index.html', self.registration.scope))); if (page) return page; }
      throw e;
    }
  })());
});
