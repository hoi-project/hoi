import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer, request as httpRequest } from 'node:http';
import { createWalkthroughGateway } from './walkthrough-gateway.mjs';

async function listen(server) {
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  return `http://127.0.0.1:${server.address().port}`;
}

async function close(server) {
  await new Promise(resolve => { server.closeAllConnections(); server.close(resolve); });
}

async function fixture(t, handler) {
  const upstream = createServer(handler);
  const destination = await listen(upstream);
  const gateway = createWalkthroughGateway({ upstream: destination, port: 7080 });
  const address = await listen(gateway);
  t.after(async () => { await close(gateway); await close(upstream); });
  const options = (path, extra = {}) => ({ hostname: '127.0.0.1', port: gateway.address().port, path,
    ...extra, headers: { Host: 'localhost:7080', ...extra.headers } });
  const call = (path, { method = 'GET', headers = {}, body } = {}) => new Promise((resolve, reject) => {
    const request = httpRequest(options(path, { method, headers }), response => {
      const chunks = [];
      response.on('data', chunk => chunks.push(chunk));
      response.on('error', reject);
      response.on('end', () => resolve({ status: response.statusCode, headers: response.headers, body: Buffer.concat(chunks) }));
    });
    request.on('error', reject);
    request.setTimeout(2000, () => request.destroy(new Error('Gateway test request timed out.')));
    request.end(body);
  });
  return { upstream, gateway, address, options, call };
}

test('gateway rejects foreign hosts, origins and absolute request targets before forwarding', async t => {
  const seen = [];
  const { call } = await fixture(t, (request, response) => { seen.push(request.url); response.end('local'); });
  for (const [path, headers] of [
    ['/api/state', { Host: 'example.com' }],
    ['/api/state', { Host: 'localhost:7081' }],
    ['/api/state', { Origin: 'https://example.com' }],
    ['/api/state', { Origin: 'null' }],
    ['/api/state', { Origin: 'http://127.0.0.1:7080' }],
    ['http://example.com/api/state', {}],
    ['//example.com/api/state', {}],
  ]) {
    const result = await call(path, { headers });
    assert.equal(result.status, 403, path);
    assert.equal(result.headers['cache-control'], 'no-store');
  }
  assert.deepEqual(seen, []);
  for (const host of ['localhost:7080', '127.0.0.1:7080', '[::1]:7080']) {
    assert.equal((await call('/api/state', { headers: { Host: host, Origin: `http://${host}` } })).status, 200);
  }
  assert.deepEqual(seen, ['/api/state', '/api/state', '/api/state']);
});

test('encoded URLs and forwarding headers cannot change the fixed destination', async t => {
  const seen = [];
  const { call } = await fixture(t, async (request, response) => {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    seen.push({ path: request.url, host: request.headers.host, method: request.method, body: Buffer.concat(chunks).toString(),
      authorization: request.headers['proxy-authorization'], upgrade: request.headers.upgrade });
    response.writeHead(201, { 'Content-Type': 'application/json' }); response.end('{"stored":true}');
  });
  const path = '/api/publish?target=http%3A%2F%2Fexample.com%2F';
  const result = await call(path, { method: 'POST', headers: {
    Origin: 'http://localhost:7080', 'Content-Type': 'application/json', 'x-sandbox-control': '1',
    'proxy-authorization': 'fixture-proxy-secret', 'x-forwarded-host': 'example.com', upgrade: 'websocket',
  }, body: '{"title":"Local sample"}' });
  assert.equal(result.status, 201);
  assert.deepEqual(JSON.parse(result.body), { stored: true });
  assert.deepEqual(seen, [{ path, host: 'localhost:7080', method: 'POST', body: '{"title":"Local sample"}', authorization: undefined, upgrade: undefined }]);
  assert.equal((await call('/%2f%2fexample.com/')).status, 201);
  assert.equal(seen[1].path, '/%2f%2fexample.com/');
});

test('gateway streams binary request and response data before the upload finishes', async t => {
  const { options } = await fixture(t, (request, response) => {
    response.writeHead(207, { 'Content-Type': 'application/octet-stream', 'X-Local-Evidence': 'streamed' });
    request.on('data', chunk => response.write(chunk));
    request.on('end', () => response.end());
  });
  const first = Buffer.from([0, 255, 13, 10, 240, 159, 140, 177]);
  const last = Buffer.alloc(64 * 1024, 137);
  await new Promise((resolve, reject) => {
    const request = httpRequest(options('/api/publish', { method: 'POST', headers: { Origin: 'http://localhost:7080', 'Content-Type': 'application/octet-stream' } }), response => {
      assert.equal(response.statusCode, 207);
      assert.equal(response.headers['x-local-evidence'], 'streamed');
      const chunks = [];
      let completedUpload = false;
      response.on('data', chunk => {
        chunks.push(chunk);
        if (!completedUpload) {
          assert.equal(request.writableEnded, false, 'the response arrived while the request body was still open');
          completedUpload = true;
          request.end(last);
        }
      });
      response.on('error', reject);
      response.on('end', () => { assert.deepEqual(Buffer.concat(chunks), Buffer.concat([first, last])); resolve(); });
    });
    request.on('error', reject);
    request.setTimeout(2000, () => request.destroy(new Error('Gateway buffered the streaming request.')));
    request.write(first);
  });
});

test('an unavailable upstream returns a bounded local error without its address', async t => {
  const { call, upstream } = await fixture(t, (_request, response) => response.end('ready'));
  await close(upstream);
  const result = await call('/api/state');
  assert.equal(result.status, 503);
  assert.equal(result.headers['cache-control'], 'no-store');
  assert.match(JSON.parse(result.body).error, /local network is starting/);
  assert.doesNotMatch(result.body.toString(), /ECONNREFUSED|127\.0\.0\.1|walkthrough:7080/);
});

test('an interrupted upstream response terminates the downstream and keeps the gateway alive', async t => {
  const { options, call } = await fixture(t, (request, response) => {
    if (request.url !== '/interrupted') { response.end('still ready'); return; }
    response.writeHead(200, { 'Content-Type': 'image/png', 'Content-Length': '1000' });
    response.write(Buffer.from([137, 80, 78, 71]));
    setImmediate(() => response.destroy());
  });
  await new Promise((resolve, reject) => {
    const request = httpRequest(options('/interrupted'), response => {
      response.resume();
      response.on('aborted', resolve);
      response.on('error', error => error.code === 'ECONNRESET' ? resolve() : reject(error));
      response.on('end', () => reject(new Error('A truncated image must not look like a complete response.')));
    });
    request.on('error', error => error.code === 'ECONNRESET' ? resolve() : reject(error));
    request.setTimeout(2000, () => request.destroy(new Error('Interrupted response was left open.')));
    request.end();
  });
  assert.equal((await call('/health')).body.toString(), 'still ready');
});
