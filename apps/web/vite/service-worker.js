/* hoi service worker. Hand-written; the build fills in VERSION and PRECACHE (see vite/sw-plugin.js).
 * - precaches every file of this build, each checked against the SHA-256 the build recorded:
 *   a file whose bytes differ is never cached or served, and an install that meets one fails
 * - cache-first for hashed assets (and the other precached files), network-first for index.html
 * - never touches cross-origin requests
 * - waits for the page to say "skip-waiting" (the "A new version is ready · Reload" button) */
const VERSION = '%VERSION%';
/** Path → SHA-256 hex of the file's bytes; null for index.html, whose own SRI pins the entry. */
const PRECACHE = '%PRECACHE%';
const CACHE = 'hoi-' + VERSION;
const SCOPE = new URL(self.registration.scope);
const abs = (p) => new URL(p, SCOPE).href;
const EXPECTED = new Map(Object.entries(PRECACHE).map(([p, hash]) => [abs(p), hash]));
const INDEX = abs('index.html');

self.addEventListener('install', (event) => {
  event.waitUntil(
    (async () => {
      const cache = await caches.open(CACHE);
      const checked = await Promise.all(
        [...EXPECTED].map(async ([url, hash]) => [url, await verified(await fetch(new Request(url, { cache: 'reload' })), hash)]),
      );
      for (const [url, res] of checked) await cache.put(url, res);
    })(),
  );
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    (async () => {
      for (const key of await caches.keys()) {
        if (key.startsWith('hoi-') && key !== CACHE) await caches.delete(key);
      }
      await self.clients.claim();
    })(),
  );
});

self.addEventListener('message', (event) => {
  if (event.data && event.data.type === 'skip-waiting') self.skipWaiting();
});

self.addEventListener('fetch', (event) => {
  const req = event.request;
  if (req.method !== 'GET') return;
  const url = new URL(req.url);
  if (url.origin !== self.location.origin) return; // never cache cross-origin
  if (!url.href.startsWith(SCOPE.href)) return;
  const clean = url.origin + url.pathname;
  // Release notices can be replaced independently of a build. Never freeze them in an
  // installed service worker or turn a network failure into stale authoritative metadata.
  if (clean === abs('release.json')) return;
  if (req.mode === 'navigate' || clean === INDEX || clean === SCOPE.href) {
    event.respondWith(networkFirst(req, clean === INDEX || clean === SCOPE.href));
    return;
  }
  if (EXPECTED.has(clean)) event.respondWith(cacheFirst(req, clean));
});

/** The response with its bytes checked against the build's hash; throws when they differ. */
async function verified(res, hash) {
  if (!res.ok || res.type !== 'basic') throw new Error(`precache: ${res.status} for ${res.url}`);
  if (hash === null) return res;
  const bytes = await res.arrayBuffer();
  const digest = [...new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))].map((b) => b.toString(16).padStart(2, '0')).join('');
  if (digest !== hash) throw new Error(`precache: ${res.url} does not match this build`);
  return new Response(bytes, { status: res.status, statusText: res.statusText, headers: res.headers });
}

async function networkFirst(req, isIndex) {
  const cache = await caches.open(CACHE);
  try {
    const res = await fetch(req);
    if (isIndex && res.ok && res.type === 'basic') await cache.put(INDEX, res.clone());
    return res;
  } catch (err) {
    const hit = await cache.match(INDEX);
    if (hit) return hit;
    throw err;
  }
}

async function cacheFirst(req, key) {
  const cache = await caches.open(CACHE);
  const hit = await cache.match(key);
  if (hit) return hit;
  const res = await fetch(req);
  if (!res.ok || res.type !== 'basic') return res;
  // Bytes that differ from this build's are refused, not passed on.
  const good = await verified(res, EXPECTED.get(key)).catch(() => null);
  if (!good) return Response.error();
  await cache.put(key, good.clone());
  return good;
}
