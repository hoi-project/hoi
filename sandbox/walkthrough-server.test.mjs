import test from 'node:test';
import assert from 'node:assert/strict';
import { get } from 'node:http';
import { createWalkthroughServer, validatePost } from './walkthrough-server.mjs';

async function fixture(t, overrides = {}) {
  const actions = [];
  const network = {
    snapshot: async () => ({ status: 'ready', participants: [] }),
    publish: async (post) => { actions.push(post); },
    startJourney: async (post) => { actions.push({ startJourney: post }); },
    nextJourney: async (step) => { actions.push({ nextJourney: step }); },
    reset: async () => { actions.push('reset'); },
    screenshot: async () => Buffer.from('image'),
    ...overrides,
  };
  const server = createWalkthroughServer(network, { port: 0 });
  await new Promise(done => server.listen(0, '127.0.0.1', done));
  const origin = `http://127.0.0.1:${server.address().port}`;
  t.after(() => new Promise(done => { server.closeAllConnections(); server.close(done); }));
  const post = (path, body, headers = {}) => fetch(origin + path, {
    method: 'POST', headers: { origin, 'content-type': 'application/json', 'x-sandbox-control': '1', ...headers }, body: JSON.stringify(body),
  });
  return { origin, post, actions };
}

test('walkthrough publishes only bounded seeded-persona posts', async t => {
  const { post, actions } = await fixture(t);
  const input = { author: 'alice', visibility: 'private', title: 'Dinner', text: 'At seven.' };
  assert.equal((await post('/api/publish', input)).status, 200);
  assert.deepEqual(actions, [input]);
  for (const change of [{ author: 'external' }, { visibility: 'secret' }, { title: '' }, { text: 'x'.repeat(4001) }, { url: 'https://example.com' }]) {
    assert.equal((await post('/api/publish', { ...input, ...change })).status, 400);
  }
  assert.equal(actions.length, 1);
  assert.throws(() => validatePost(null));
});

test('guided actions start with a bounded message and advance only one server-owned step', async t => {
  const { post, actions } = await fixture(t);
  const message = { author: 'alice', visibility: 'private', title: 'Dinner plan', text: 'Meet by the blue door at seven.' };
  const step = { journeyId: 'journey-1', stage: 'draft' };
  assert.equal((await post('/api/journey/start', message)).status, 200);
  assert.equal((await post('/api/journey/next', step)).status, 200);
  assert.deepEqual(actions, [{ startJourney: message }, { nextJourney: step }]);
  assert.equal((await post('/api/journey/start', { ...message, receiver: 'external' })).status, 400);
  assert.equal((await post('/api/journey/next', { step: 'open', key: 'not-accepted' })).status, 400);
  assert.equal((await post('/api/journey/next', {})).status, 400);
  assert.equal((await post('/api/journey/next', { ...step, stage: 'arbitrary' })).status, 400);
  assert.equal((await post('/api/journey/next', {}, { origin: 'https://example.com' })).status, 403);
  assert.equal((await post('/api/journey/next', {}, { 'x-sandbox-control': '' })).status, 403);
  assert.equal(actions.length, 2);
});

test('guided steps reject stale sequence without exposing internal errors', async t => {
  const { post } = await fixture(t, { nextJourney: async () => { throw Object.assign(new Error('internal detail'), { code: 'WALKTHROUGH_SEQUENCE' }); } });
  const response = await post('/api/journey/next', { journeyId: 'journey-1', stage: 'draft' });
  assert.equal(response.status, 409);
  assert.doesNotMatch(await response.text(), /internal detail/);
});

test('duplicate messages return an editable input error without exposing core details', async t => {
  const { post } = await fixture(t, { nextJourney: async () => { throw Object.assign(new Error('internal detail'), { code: 'WALKTHROUGH_DUPLICATE' }); } });
  const response = await post('/api/journey/next', { journeyId: 'journey-1', stage: 'draft' });
  assert.equal(response.status, 422);
  const body = await response.json();
  assert.match(body.error, /already.*sent|already.*published/i);
  assert.doesNotMatch(body.error, /internal detail/);
});

test('reset remains available after network setup fails', async t => {
  let state = { status: 'error', error: 'Startup failed.' };
  const { post, actions } = await fixture(t, { snapshot: async () => state, reset: async () => { actions.push('reset'); state = { status: 'ready' }; } });
  const response = await post('/api/reset', {});
  assert.equal(response.status, 200);
  assert.equal((await response.json()).status, 'ready');
  assert.deepEqual(actions, ['reset']);
});

test('walkthrough rejects foreign host, origin, control-less and oversized writes', async t => {
  const { origin, post, actions } = await fixture(t);
  const foreignHost = await new Promise((done, reject) => {
    get(origin + '/api/state', { headers: { host: 'example.com' } }, response => { response.resume(); done(response.statusCode); }).once('error', reject);
  });
  assert.equal(foreignHost, 403);
  assert.equal((await post('/api/reset', {}, { origin: 'https://example.com' })).status, 403);
  assert.equal((await post('/api/reset', {}, { 'x-sandbox-control': '' })).status, 403);
  assert.equal((await post('/api/reset', {}, { 'sec-fetch-site': 'cross-site' })).status, 403);
  assert.equal((await post('/api/reset', {}, { 'content-type': 'text/plain' })).status, 415);
  assert.equal((await post('/api/publish', { text: 'x'.repeat(20000) })).status, 413);
  assert.deepEqual(actions, []);
});

test('walkthrough refuses overlapping mutations and recovers after failure', async t => {
  let release;
  const waiting = new Promise(resolve => { release = resolve; });
  let entered;
  const started = new Promise(resolve => { entered = resolve; });
  const { origin, post } = await fixture(t, { reset: async () => { entered(); await waiting; throw new Error('Reset failed.'); } });
  const first = post('/api/reset', {});
  await started;
  assert.equal((await post('/api/reset', {})).status, 409);
  assert.equal((await post('/api/journey/next', { journeyId: 'journey-1', stage: 'draft' })).status, 409);
  assert.equal((await (await fetch(origin + '/api/state')).json()).busy, true);
  release();
  assert.equal((await first).status, 500);
  assert.equal((await (await fetch(origin + '/api/state')).json()).busy, false);
});

test('walkthrough prevents reset and publication from racing initial seeding', async t => {
  const { origin, post, actions } = await fixture(t, { snapshot: async () => ({ status: 'starting' }) });
  assert.equal((await (await fetch(origin + '/api/state')).json()).busy, true);
  assert.equal((await post('/api/reset', {})).status, 409);
  assert.equal((await post('/api/journey/start', { author: 'alice', visibility: 'private', title: 'Wait', text: 'Still starting.' })).status, 409);
  assert.equal((await post('/api/publish', { author: 'alice', visibility: 'public', title: 'Wait', text: 'Still starting.' })).status, 409);
  assert.deepEqual(actions, []);
});

test('walkthrough serves only allowlisted assets and participant screenshots', async t => {
  const { origin } = await fixture(t);
  const response = await fetch(origin + '/');
  assert.equal(response.status, 200);
  assert.match(response.headers.get('content-security-policy'), /connect-src 'self'/);
  assert.match(response.headers.get('content-security-policy'), /frame-ancestors 'none'/);
  for (const path of ['/walkthrough-story.mjs', '/walkthrough-player.mjs']) {
    const asset = await fetch(origin + path);
    assert.equal(asset.status, 200);
    assert.match(asset.headers.get('content-type'), /text\/javascript/);
  }
  assert.equal((await fetch(origin + '/api/screenshot/alice')).headers.get('content-type'), 'image/png');
  for (const path of ['/api/screenshot/owner', '/package.json', '/%2e%2e/AGENTS.md']) assert.equal((await fetch(origin + path)).status, 404);
});
