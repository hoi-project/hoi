import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer, request as httpRequest } from 'node:http';
import { mkdtemp, writeFile, unlink, rmdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createAppHandler, validHost } from './app-server.mjs';

const token = 'local-test-token-that-is-long-enough';
const listen = (server) => new Promise((resolve) => server.listen(0, '127.0.0.1', () => resolve(`http://127.0.0.1:${server.address().port}`)));
const close = (server) => new Promise((resolve) => { server.close(resolve); server.closeAllConnections(); });
// Node's fetch may normalize Host; HTTP requests let us exercise the proxy's exact Host boundary.
const fetch = (url, options = {}) => new Promise((resolve, reject) => {
  const request = httpRequest(url, options, (response) => {
    let body = '';
    response.on('data', (chunk) => { body += chunk; });
    response.on('end', () => resolve({ status: response.statusCode, text: async () => body }));
  });
  request.on('error', reject);
  request.end(options.body);
});

test('only exact loopback origins and the assigned role are accepted', () => {
  assert.equal(validHost('localhost:7071', 7071), true);
  assert.equal(validHost('[::1]:7071', 7071), true);
  for (const host of ['localhost:7072', 'attacker.example:7071', 'localhost.attacker.example:7071', 'localhost:7071@evil.test']) {
    assert.equal(validHost(host, 7071), false);
  }
  assert.throws(() => createAppHandler({ token, role: 'owner', port: 7072 }));
  assert.throws(() => createAppHandler({ token: '', role: 'owner', port: 7071 }));
});

test('app proxy protects owner linking, ignores caller credentials, and prevents cross-origin mutations', async () => {
  const root = await mkdtemp(join(tmpdir(), 'hoi-sandbox-app-'));
  await writeFile(join(root, 'index.html'), '<h1>Local app</h1>');
  const received = [];
  const upstream = createServer(async (request, response) => {
    let text = '';
    for await (const chunk of request) text += chunk;
    received.push({ path: request.url, role: request.headers['x-hoi-sandbox-role'], token: request.headers['x-hoi-sandbox-token'], body: text });
    response.writeHead(200, { 'content-type': 'application/json' });
    response.end(JSON.stringify({ ok: true }));
  });
  const controller = await listen(upstream);
  const server = createServer(createAppHandler({ root, controller, token, role: 'member', port: 7072 }));
  const base = await listen(server);
  const headers = { host: 'localhost:7072' };
  try {
    const index = await fetch(base, { headers });
    assert.equal(index.status, 200);
    assert.match(await index.text(), /Local app/);
    assert.match(await (await fetch(base + '/app/', { headers })).text(), /Local app/);
    assert.equal((await fetch(base, { headers: { host: 'outside.example:7072' } })).status, 403);
    assert.equal((await fetch(base + '/__sandbox/bootstrap', { headers: { ...headers, 'x-hoi-sandbox-role': 'owner', 'x-hoi-sandbox-token': 'forged' } })).status, 200);
    assert.deepEqual(received[0], { path: '/__sandbox/bootstrap', role: 'member', token, body: '' });
    assert.equal((await fetch(base + '/__sandbox/link', { method: 'POST', headers: { ...headers, origin: 'http://localhost:7072', 'content-type': 'application/json' }, body: '{}' })).status, 403);
    assert.equal((await fetch(base + '/__sandbox/click', { method: 'POST', headers: { ...headers, origin: 'https://evil.example', 'content-type': 'application/json' }, body: '{}' })).status, 403);
    assert.equal((await fetch(base + '/__sandbox/click', { method: 'POST', headers: { ...headers, origin: 'http://localhost:7072', 'content-type': 'text/plain' }, body: '{}' })).status, 403);
    assert.equal((await fetch(base + '/__sandbox/visit', { method: 'POST', headers: { ...headers, origin: 'http://localhost:7072', 'content-type': 'application/json' }, body: '{"community":"test"}' })).status, 200);
    assert.equal(received.length, 2);
    assert.equal(received[1].role, 'member');
    assert.equal((await fetch(base + '/__sandbox/config', { headers })).status, 404);
    assert.equal((await fetch(base + '/%2e%2e%5csecret', { headers })).status, 400);
  } finally {
    await close(server); await close(upstream);
    await unlink(join(root, 'index.html')); await rmdir(root);
  }
});
