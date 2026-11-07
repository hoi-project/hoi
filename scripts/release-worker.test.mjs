import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const origin = 'https://hoi.test';
const scope = `${origin}/app/`;
const sha = (text) => createHash('sha256').update(text).digest('hex');

/** Loads the hand-written worker with a precache manifest and fake caches/fetch. */
function load(precache, served) {
  const handlers = new Map();
  const stored = new Map();
  const cache = { put: async (url, res) => stored.set(typeof url === 'string' ? url : url.url, res), match: async (url) => stored.get(url) };
  const source = readFileSync(new URL('../apps/web/vite/service-worker.js', import.meta.url), 'utf8')
    .replace("'%VERSION%'", '"test"').replace("'%PRECACHE%'", JSON.stringify(precache));
  const fetch = async (req) => {
    const url = typeof req === 'string' ? req : req.url;
    const body = served[url.slice(scope.length)];
    if (body === undefined) return new Response('missing', { status: 404 });
    const res = new Response(body, { status: 200 });
    Object.defineProperty(res, 'type', { value: 'basic' });
    Object.defineProperty(res, 'url', { value: url });
    return res;
  };
  vm.runInNewContext(source, {
    URL, Map, Set, Object, Promise, Request, Response, Error, Uint8Array, crypto, fetch,
    caches: { open: async () => cache, keys: async () => [] },
    self: { location: { origin }, registration: { scope }, addEventListener: (type, callback) => handlers.set(type, callback) },
  });
  return { handlers, stored };
}

async function install(handlers) {
  let done;
  handlers.get('install')({ waitUntil: (p) => { done = p; } });
  return done;
}

test('an installed service worker never serves cached release metadata', () => {
  const { handlers } = load({ 'index.html': null, 'release.json': sha('{}') }, {});
  let handled = false;
  handlers.get('fetch')({ request: { method: 'GET', url: `${scope}release.json`, mode: 'cors' }, respondWith() { handled = true; } });
  assert.equal(handled, false, 'even an older precache containing release.json must defer to the network');
});

test('the worker installs only when every precached file matches the build hash', async () => {
  const files = { 'index.html': '<!doctype html>', 'assets/app.js': 'console.log(1)', 'assets/core.wasm': 'wasm bytes' };
  const precache = { 'index.html': null, 'assets/app.js': sha(files['assets/app.js']), 'assets/core.wasm': sha(files['assets/core.wasm']) };
  const good = load(precache, files);
  await install(good.handlers);
  assert.equal(good.stored.size, 3);

  const bad = load(precache, { ...files, 'assets/core.wasm': 'tampered bytes' });
  await assert.rejects(install(bad.handlers), /does not match this build/);
  assert.equal(bad.stored.size, 0, 'nothing is cached from a failed install');
});

test('a cache miss with bytes that differ from the build is refused', async () => {
  const precache = { 'index.html': null, 'assets/chunk.js': sha('export default 1') };
  const { handlers, stored } = load(precache, { 'assets/chunk.js': 'export default 2' });
  let response;
  handlers.get('fetch')({ request: { method: 'GET', url: `${scope}assets/chunk.js`, mode: 'cors' }, respondWith(p) { response = p; } });
  const res = await response;
  assert.equal(res.type, 'error');
  assert.equal(stored.size, 0);
});
