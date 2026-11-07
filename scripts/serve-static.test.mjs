import assert from 'node:assert/strict';
import { request } from 'node:http';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { serveStatic } from './serve-static.mjs';

// The raw request path, as a browser or a crafted link sends it (fetch would normalise it).
const get = (origin, path) => new Promise((done, fail) => {
  const req = request(origin, { path, method: 'GET' }, (res) => { res.resume(); done({ status: res.statusCode, location: res.headers.location }); });
  req.on('error', fail).end();
});

test('a folder redirects to its own path with a slash, never to another host', async () => {
  const root = mkdtempSync(join(tmpdir(), 'hoi-serve-static-'));
  mkdirSync(join(root, 'assets'));
  writeFileSync(join(root, 'assets', 'index.html'), 'assets');
  mkdirSync(join(root, 'wiki', 'my page'), { recursive: true });
  const server = await serveStatic({ root });
  try {
    assert.deepEqual(await get(server.origin, '/assets'), { status: 301, location: '/assets/' });
    // An encoded slash decodes to "//assets", and "//assets/" in Location is a protocol-relative
    // URL: the browser would go to a host named "assets".
    for (const path of ['/%2Fassets', '/%2F%2Fassets']) assert.deepEqual(await get(server.origin, path), { status: 301, location: '/assets/' }, path);
    assert.equal((await get(server.origin, '/assets/')).status, 200);
    // Nested folders keep their path; names are encoded as they would be in a link.
    assert.deepEqual(await get(server.origin, '/wiki/my%20page'), { status: 301, location: '/wiki/my%20page/' });
  } finally {
    await server.close();
    rmSync(root, { recursive: true, force: true });
  }
});
