import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import { randomUUID } from 'node:crypto';
import { createControlServer } from './server.mjs';
import { ProviderBroker } from './providers.mjs';
import { Suite } from './suite.mjs';

async function mailboxServer(t, suite = {}) {
  const token = 'b'.repeat(64), broker = new ProviderBroker({ timeoutMs: 3000 });
  const server = createControlServer({ state: { ready: true }, save: async () => {}, ...suite }, { token, port: 7070, broker });
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  t.after(() => { broker.close(); return new Promise(resolve => { server.closeAllConnections(); server.close(resolve); }); });
  const base = `http://127.0.0.1:${server.address().port}`;
  const call = (path, { headers = {}, method = 'POST', data = {}, timeout = 1500 } = {}) => new Promise((resolve, reject) => {
    const request = http.request(base + path, { method, headers: { Host: 'localhost:7070', 'Content-Type': 'application/json', ...headers } }, response => {
      let value = ''; response.on('data', chunk => { value += chunk; });
      response.on('end', () => resolve({ status: response.statusCode, body: JSON.parse(value) }));
    });
    request.setTimeout(timeout, () => request.destroy(new Error(`Timed out calling ${path}`)));
    request.on('error', reject); request.end(method === 'GET' ? undefined : JSON.stringify(data));
  });
  return { broker, call, headers: { 'x-hoi-sandbox-token': token }, workerId: randomUUID(), providers: { codex: { available: true }, claude: { available: false, error: 'Not installed' } } };
}

test('controller rejects cross-origin mutations, rebinding hosts, and forged persona proxy', async t => {
  let changed = 0;
  const suite = { state: { ready: true }, configure: () => changed++, save: async () => {}, bootstrap: async role => ({ role }) };
  const server = createControlServer(suite, { token: 'a'.repeat(40), port: 7070 });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  t.after(() => new Promise(resolve => { server.closeAllConnections(); server.close(resolve); }));
  const base = `http://127.0.0.1:${server.address().port}`;
  const call = (path, headers = {}, method = 'GET') => new Promise((resolve, reject) => {
    const request = http.request(base + path, { method, headers: { Host: 'localhost:7070', ...headers } }, response => {
      let value = ''; response.on('data', chunk => { value += chunk; });
      response.on('end', () => resolve({ status: response.statusCode, json: async () => JSON.parse(value) }));
    });
    request.on('error', reject); request.end(method === 'POST' ? '{}' : undefined);
  });
  assert.equal((await call('/api/state', { Host: 'attacker.example:7070' })).status, 403);
  assert.equal((await call('/api/config', { Origin: 'https://attacker.example', 'x-sandbox-control': '1', 'Content-Type': 'application/json' }, 'POST')).status, 403);
  assert.equal((await call('/api/config', { 'Content-Type': 'application/json' }, 'POST')).status, 403);
  assert.equal((await call('/api/config', { 'x-sandbox-control': '1', 'Content-Type': 'application/json' }, 'POST')).status, 200);
  assert.equal(changed, 1);
  assert.equal((await call('/__sandbox/bootstrap', { 'x-hoi-sandbox-role': 'owner' })).status, 403);
  const valid = await call('/__sandbox/bootstrap', { 'x-hoi-sandbox-role': 'member', 'x-hoi-sandbox-token': 'a'.repeat(40), Host: 'dashboard:7070' });
  assert.deepEqual(await valid.json(), { role: 'member' });
  assert.equal((await call('/__sandbox/link', { 'x-hoi-sandbox-role': 'member', 'x-hoi-sandbox-token': 'a'.repeat(40), 'Content-Type': 'application/json' }, 'POST')).status, 403);
});

test('CLI mailbox requires its token and grants the role exception only to the two exact worker routes', async t => {
  const { call, headers, workerId, providers } = await mailboxServer(t);
  const data = { workerId, providers, capacity: 2 };
  assert.equal((await call('/__sandbox/provider/take', { data })).status, 403);
  assert.equal((await call('/__sandbox/provider/take', { data, headers: { 'x-hoi-sandbox-role': 'owner' } })).status, 403);
  assert.equal((await call('/__sandbox/provider/take', { data, headers: { 'x-hoi-sandbox-token': 'c'.repeat(64) } })).status, 403);
  const valid = await call('/__sandbox/provider/take', { data, headers });
  assert.equal(valid.status, 200); assert.deepEqual(valid.body, { job: null, activeIds: [] });
  assert.equal((await call('/__sandbox/provider/take', { method: 'GET', headers })).status, 405);
  for (const path of ['/__sandbox/bootstrap', '/__sandbox/link', '/__sandbox/visit', '/__sandbox/provider/anything', '/__sandbox/provider/take/extra']) {
    assert.equal((await call(path, { headers, method: path.endsWith('bootstrap') ? 'GET' : 'POST' })).status, 403, path);
  }
  assert.equal((await call('/__sandbox/provider/take', { data, headers: { ...headers, 'Content-Type': 'text/plain' } })).status, 415);
  const state = await call('/api/state', { method: 'GET' });
  assert.equal(state.body.providers.online, true);
  assert.equal(state.body.providers.providers.codex.available, true);
  assert.ok(!JSON.stringify(state.body).includes(headers['x-hoi-sandbox-token']));
});

test('mailbox completion remains available while the ordinary mutation queue awaits a model decision', async t => {
  let broker, began;
  const started = new Promise(resolve => { began = resolve; });
  const suite = { configure: async () => { const pending = broker.request({ driver: 'codex', messages: [{ role: 'user', content: 'Choose a community action.' }] }); began(); suite.decision = await pending; } };
  const server = await mailboxServer(t, suite); broker = server.broker;
  const { call, headers, workerId, providers } = server;
  await call('/__sandbox/provider/take', { headers, data: { workerId, providers, capacity: 2 } });
  const configuration = call('/api/config', { headers: { 'x-sandbox-control': '1' } });
  await started;
  const claimed = await call('/__sandbox/provider/take', { headers, data: { workerId, providers, capacity: 2 } });
  assert.ok(claimed.body.job);
  const id = claimed.body.job.id;
  assert.equal((await call('/__sandbox/provider/complete', { headers, data: { workerId: randomUUID(), id, content: '{"action":"observe"}' } })).status, 400);
  assert.equal((await call('/__sandbox/provider/complete', { headers, data: { workerId, id, content: '{"action":"observe"}' } })).status, 200);
  assert.equal((await configuration).status, 200);
  assert.equal(suite.decision, '{"action":"observe"}');
});

test('stop cancels an outstanding mailbox decision and discards its late completion', async t => {
  const controller = new AbortController();
  const suite = { state: { ready: true, agents: [], run: { status: 'running', checks: [] } }, controller, jobs: new Map(), stopRun: Suite.prototype.stopRun, event: () => {}, save: async () => {} };
  const { broker, call, headers, workerId, providers } = await mailboxServer(t, suite);
  await call('/__sandbox/provider/take', { headers, data: { workerId, providers, capacity: 2 } });
  let decisionError;
  const pending = broker.request({ driver: 'codex', messages: [{ role: 'user', content: 'Choose an action.' }] }, controller.signal).catch(error => { decisionError = error; });
  suite.jobs.set('agent-one', { job: pending });
  const claimed = await call('/__sandbox/provider/take', { headers, data: { workerId, providers, capacity: 2 } });
  assert.ok(claimed.body.job);
  const stopped = await call('/api/run/stop', { headers: { 'x-sandbox-control': '1' } });
  assert.equal(stopped.status, 200); assert.equal(stopped.body.run.status, 'stopped');
  assert.equal(decisionError.name, 'AbortError');
  const heartbeat = await call('/__sandbox/provider/take', { headers, data: { workerId, providers, capacity: 0 } });
  assert.deepEqual(heartbeat.body.activeIds, []);
  const late = await call('/__sandbox/provider/complete', { headers, data: { workerId, id: claimed.body.job.id, content: '{"action":"post","text":"Too late"}' } });
  assert.equal(late.status, 200); assert.equal(late.body.discarded, true);
  assert.equal(broker.status().running, 0);
});
