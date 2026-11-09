import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdir, readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { createServer } from 'node:http';
import { chromium } from 'playwright';
import { createHash } from 'node:crypto';
import { describeCryptoJourney } from './walkthrough-crypto-view.mjs';
import { describeFlow } from './walkthrough-flow.mjs';

const ids = ['draft', 'seal', 'store', 'download', 'open'];
const actions = ['Sign and encrypt', 'Store encrypted copies', 'Download as Bob', 'Decrypt and read as Bob', null];
const object = { alias: 'E1', id: createHash('sha256').update(Buffer.alloc(4096, 0xab)).digest('hex'), room: 'f'.repeat(64), size: 4096, expires: 1800000000, from: 'P1', hex: 'ab'.repeat(4096), segments: [
  { name: 'Nonce', offset: 0, bytes: 24, hex: 'ab'.repeat(24) },
  { name: 'Encrypted bytes', offset: 24, bytes: 4056, hex: 'ab'.repeat(4056) },
  { name: 'Authentication tag', offset: 4080, bytes: 16, hex: 'ab'.repeat(16) },
] };
const copy = { service: 'relay-a', label: 'Private relay A', objectAlias: 'E1', objectId: object.id, size: 4096, sha256: object.id, identical: true };
function permissions(index) { return [
  { id: 'alice', label: 'Alice', state: 'readable', detail: 'Has the original message.' },
  ...['A', 'B'].map(label => ({ id: 'relay-' + label.toLowerCase(), label: 'Relay ' + label, state: index >= 2 ? 'ciphertext' : 'none', detail: index >= 2 ? 'Only encrypted bytes and delivery metadata.' : 'Have not received this message.' })),
  { id: 'bob', label: 'Bob', state: index === 4 ? 'readable' : index === 3 ? 'ciphertext' : 'none', detail: index === 4 ? 'The core recovered and checked the message.' : index === 3 ? 'Downloaded bytes have not been opened yet.' : 'Has not received this message.' },
]; }
function journey(index = 0, message = { title: 'The private plan', text: 'Meet by the blue door at seven.' }) {
  const value = { id: 'journey-one', mode: 'live', visibility: 'private', author: 'alice', receiver: 'bob', community: { id: 'c'.repeat(64), name: 'Our group' }, stage: ids[index], complete: index === 4,
    message: { alias: 'M1', title: message.title, text: message.text, utf8Bytes: 32 },
    ...(index >= 1 ? { post: { alias: 'P1', id: 'a'.repeat(64), title: message.title, text: message.text, checks: { signature: 'valid' } } } : {}),
    objects: index >= 1 ? [object] : [], stored: index >= 2 ? [copy, { ...copy, service: 'relay-b', label: 'Private relay B' }] : [],
    downloads: index >= 3 ? [{ ...copy, hex: object.hex }] : [],
    ...(index === 4 ? { recovered: { alias: 'P1', id: 'a'.repeat(64), title: message.title, text: message.text, samePost: true, sameText: true, sameTitle: true, checks: { signature: 'valid' } } } : {}),
    permissions: permissions(index), nextAction: actions[index], stages: ids.map((id, i) => ({ id, title: id, status: i <= index ? 'complete' : 'pending', input: i ? ['E1'] : ['M1'], output: i === 4 ? ['P1'] : ['E1'], explanation: 'Measured by the local simulation.', observations: ['Actual output captured.'], internals: [], permissions: permissions(i) })),
  };
  value.crypto = describeCryptoJourney(value);
  value.flow = { mode: 'private-replicas', nodes: [
    { id: 'alice', label: 'Alice', kind: 'person', detail: 'Sending device.' },
    { id: 'relay-a', label: 'Relay A', kind: 'relay', detail: 'Encrypted storage.' },
    { id: 'relay-b', label: 'Relay B', kind: 'relay', detail: 'Encrypted storage.' },
    { id: 'bob', label: 'Bob', kind: 'person', detail: 'Receiving device.' },
    { id: 'registry-a', label: 'Registry A', kind: 'registry', detail: 'Public discovery only.' },
  ], edges: [
    ...['a', 'b'].map(id => ({ id: 'store-' + id, from: 'alice', to: 'relay-' + id, kind: 'delivery', stage: 'store', status: index >= 2 ? 'observed' : 'pending', label: 'Copy E1 to relay ' + id.toUpperCase(), input: ['Encrypted E1 on Alice'], action: 'Upload the whole encrypted object', output: ['A stored copy of E1'], artifactIds: ['E1'], objectId: object.id, sameBytes: index >= 2, evidence: index >= 2 ? ['Actual upload receipt and complete bytes match.'] : [] })),
    { id: 'download-a', from: 'relay-a', to: 'bob', kind: 'delivery', stage: 'download', status: index >= 3 ? 'observed' : 'pending', label: 'Download E1', input: ['Stored E1'], action: 'Fetch the complete encrypted object', output: ['E1 in Bob’s browser'], artifactIds: ['E1'], objectId: object.id, sameBytes: index >= 3, evidence: index >= 3 ? ['Actual download receipt and complete bytes match.'] : [] },
  ], discovery: [] };
  return value;
}

test('guided message journey links input, encryption, stored bytes and recovered text', async t => {
  const requests = [];
  let state = { status: 'ready', phase: 'Ready', session: 'test-session', journey: null, services: [{ id: 'registry-a', label: 'Registry A', kind: 'registry', records: [] }], communities: { private: { name: 'Our group' }, public: { name: 'Town square' } } };
  const server = createServer(async (req, res) => {
    if (req.url === '/api/state') { res.setHeader('content-type', 'application/json'); res.end(JSON.stringify(state)); return; }
    if (req.url.startsWith('/api/screenshot/')) {
      res.setHeader('content-type', 'image/png');
      res.end(Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a8WQAAAAASUVORK5CYII=', 'base64')); return;
    }
    if (req.url.startsWith('/api/journey/')) {
      let body = ''; for await (const part of req) body += part;
      const payload = JSON.parse(body);
      requests.push({ path: req.url, payload, control: req.headers['x-sandbox-control'] });
      state.journey = req.url.endsWith('/start') ? journey(0, payload) : journey(ids.indexOf(state.journey.stage) + 1, state.journey.message);
      res.setHeader('content-type', 'application/json'); res.end(JSON.stringify(state)); return;
    }
    const paths = { '/': 'public/walkthrough.html', '/walkthrough.mjs': 'public/walkthrough.mjs', '/walkthrough-player.mjs': 'public/walkthrough-player.mjs', '/walkthrough-story.mjs': 'public/walkthrough-story.mjs', '/walkthrough.css': 'public/walkthrough.css', '/shared/tokens.css': '../packages/tokens/generated/tokens.css', '/shared/site.css': '../apps/site/src/styles/site.css' };
    if (!paths[req.url]) { res.writeHead(404).end(); return; }
    res.setHeader('content-type', req.url.endsWith('.mjs') ? 'text/javascript' : req.url.endsWith('.css') ? 'text/css' : 'text/html');
    res.end(await readFile(new URL(paths[req.url], import.meta.url)));
  });
  await new Promise(done => server.listen(0, '127.0.0.1', done));
  t.after(() => new Promise(done => { server.closeAllConnections(); server.close(done); }));
  const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
  t.after(() => browser.close());
  const page = await browser.newPage({ viewport: { width: 1280, height: 960 }, reducedMotion: 'reduce' });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('http://127.0.0.1:' + server.address().port + '/');
  await page.getByRole('button', { name: 'Prepare this message', exact: true }).waitFor({ timeout: 3000 });
  assert.equal(await page.locator('#scenario-private').getAttribute('aria-pressed'), 'true');
  assert.doesNotMatch(await page.locator('#journey-panel').textContent(), /Registry A|discovery metadata only/i);
  const title = '<img src=x onerror=alert(1)>';
  const text = 'Meet by the blue door at seven.';
  await page.getByLabel('Post title', { exact: true }).fill(title);
  await page.getByLabel('Message', { exact: true }).fill(text);
  await page.getByRole('button', { name: 'Prepare this message', exact: true }).click();
  await page.waitForFunction(() => document.querySelector('#story-player').dataset.frame === 'private-community' && !document.querySelector('#story-next').disabled);
  assert.equal(await page.evaluate(() => document.activeElement.id), 'story-title');
  assert.equal(await page.evaluate(() => scrollY > 0 && document.querySelector('#journey-panel').getBoundingClientRect().top < 110), true);
  assert.equal(await page.locator('#example-composer').getAttribute('open'), null);
  assert.equal(await page.locator('#journey-details').getAttribute('open'), null);
  await page.locator('#journey-details > summary').click();
  await page.getByRole('button', { name: actions[0], exact: true }).waitFor();
  assert.deepEqual(requests[0], { path: '/api/journey/start', payload: { author: 'alice', visibility: 'private', title, text }, control: '1' });
  assert.equal(await page.locator('#step-input img, #step-result img').count(), 0);
  assert.match(await page.locator('#readability').textContent(), /Has not received this message/);
  for (let index = 0; index < 4; index++) {
    await page.getByRole('button', { name: actions[index], exact: true }).click();
    await page.waitForFunction(stage => document.querySelector('#journey-panel').dataset.stage === stage, ids[index + 1]);
    assert.equal(requests[index + 1].path, '/api/journey/next');
    assert.deepEqual(requests[index + 1].payload, { journeyId: 'journey-one', stage: ids[index] });
    assert.equal(requests[index + 1].control, '1');
    if (index === 0) {
      assert.equal(await page.locator('#scenario-public').isDisabled(), true);
      assert.equal(await page.locator('#post-title').isDisabled(), true);
      assert.equal(await page.locator('#prepare').isDisabled(), true);
      assert.match(await page.locator('#step-result').textContent(), /Encrypted package/);
      await page.locator('#step-result').getByText('Inspect the encrypted bytes', { exact: true }).click();
      assert.equal(await page.locator('#step-result .journey-byte-section').count(), 3);
    }
    if (index === 2) {
      assert.match(await page.locator('#readability').textContent(), /Downloaded bytes have not been opened yet/);
      assert.match(await page.locator('#step-result').textContent(), /identical/i);
    }
  }
  assert.match(await page.locator('#step-result').textContent(), /Original text recovered/);
  assert.match(await page.locator('#step-result').textContent(), /Same signed post/);
  assert.match(await page.locator('#message-comparison').textContent(), /Original message|Recovered on Bob/);
  while (await page.locator('#story-player').getAttribute('data-frame') !== 'open-readable') {
    const previous = await page.locator('#story-player').getAttribute('data-frame');
    await page.locator('#story-next').click();
    await page.waitForFunction(id => document.querySelector('#story-player').dataset.frame !== id, previous);
    if (process.env.HOI_UI_SCREENSHOTS === '1' && ['protect-outer', 'upload-relay-a', 'open-readable'].includes(await page.locator('#story-player').getAttribute('data-frame'))) {
      const output = new URL('../test-results/walkthrough/', import.meta.url);
      await mkdir(output, { recursive: true });
      await page.locator('#story-player').screenshot({ path: fileURLToPath(new URL('player-' + await page.locator('#story-player').getAttribute('data-frame') + '.png', output)) });
    }
  }
  assert.match(await page.locator('#story-output').textContent(), /Meet by the blue door/);
  assert.equal(requests.length, 5, 'playing captured steps does not repeat a mutation');
  assert.equal(await page.locator('#network-map button[data-edge]').count(), 3);
  assert.equal(await page.locator('#network-map svg path[data-edge]').count(), 3);
  assert.equal(await page.locator('#network-map [data-node="registry-a"]').count(), 0);
  assert.match(await page.locator('#network-map button[data-edge="store-a"]').textContent(), /Observed/);
  assert.match(await page.locator('#crypto-receiver .journey-packet-view').textContent(), /Meet by the blue door/);
  await page.locator('#network-map button[data-edge="store-a"]').focus();
  await page.keyboard.press('Enter');
  assert.equal(await page.evaluate(() => document.activeElement.id), 'selection-title');
  assert.match(await page.locator('#selection-source').textContent(), /Actual upload receipt/);
  assert.match(await page.locator('#step-result').textContent(), /transferred bytes match/);
  await page.locator('#crypto-sender .journey-layer[data-layer="padding"] > button').click();
  assert.match(await page.locator('#selection-source').textContent(), /Explained from the production Rust core/);
  assert.doesNotMatch(await page.locator('#selection-source').textContent(), /Observed at/);
  assert.equal(await page.locator('#step-result .journey-bytes').count(), 0, 'uncaptured padding has no fabricated bytes');
  await page.locator('#crypto-sender .journey-layer[data-layer="E1-nonce"] > button').click();
  assert.equal(await page.locator('#step-result .journey-bytes').textContent(), 'ab'.repeat(24));
  assert.match(await page.locator('#step-result').textContent(), /Observed/);
  assert.equal(requests.length, 5, 'map and layer selections do not run network operations');
  await page.getByRole('button', { name: '4 Download', exact: true }).click();
  assert.doesNotMatch(await page.locator('#crypto-receiver .journey-packet-view').textContent(), /Meet by the blue door/);
  assert.match(await page.locator('#crypto-receiver .journey-packet-view').textContent(), /Not reached yet/);
  await page.getByRole('button', { name: '1 Write', exact: true }).click();
  await page.locator('#network-map button[data-edge="store-a"]').click();
  assert.match(await page.locator('#selection-source').textContent(), /planned connection/);
  assert.doesNotMatch(await page.locator('#selection-source').textContent(), /Actual upload receipt/);
  assert.equal(await page.locator('#step-input .journey-bytes').count(), 0);
  await page.getByRole('button', { name: 'Back to whole step', exact: true }).click();
  await page.getByRole('button', { name: '2 Protect', exact: true }).click();
  assert.match(await page.locator('#view-mode').textContent(), /Reviewing/);
  assert.match(await page.locator('#readability').textContent(), /Has not received this message/);
  assert.equal(requests.length, 5, 'reviewing never re-executes the operation');
  await page.getByRole('button', { name: 'Return to latest step', exact: true }).click();
  assert.equal(await page.locator('#scenario-public').isEnabled(), true);
  await page.setViewportSize({ width: 320, height: 900 });
  await page.waitForFunction(() => document.querySelector('#story-map').dataset.narrow === 'true');
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  if (process.env.HOI_UI_SCREENSHOTS === '1') {
    const output = new URL('../test-results/walkthrough/', import.meta.url);
    await mkdir(output, { recursive: true });
    await page.screenshot({ path: fileURLToPath(new URL('guided-mobile.png', output)), fullPage: true });
    await page.setViewportSize({ width: 1280, height: 960 });
    await page.evaluate(() => scrollTo(0, 0));
    await page.screenshot({ path: fileURLToPath(new URL('guided-desktop.png', output)), fullPage: true });
  }
  await page.locator('#app-preview > summary').click();
  await page.waitForFunction(() => !document.querySelector('#app-screen').hidden);
  const publicMessage = { title: 'A public announcement', text: 'The town meeting begins at seven.' };
  const publicJourney = journey(4, publicMessage);
  publicJourney.id = 'journey-public'; publicJourney.mode = 'recorded'; publicJourney.visibility = 'public';
  publicJourney.objects = []; publicJourney.downloads = [];
  publicJourney.stages = publicJourney.stages.filter(stage => stage.id !== 'download');
  publicJourney.stored = ['a', 'b'].map(id => ({ service: 'node-' + id, label: 'Public node ' + id.toUpperCase(), objectId: publicJourney.post.id, matchingPost: true, readable: true }));
  const topology = { enforced: true, nativeEntry: { configured: true }, clients: [{ id: 'alice', node: 'node-a', communityConnected: true }, { id: 'bob', node: 'node-b', communityConnected: true }] };
  publicJourney.flow = describeFlow({ visibility: 'public', journey: publicJourney, topology });
  state.journey = publicJourney;
  await page.getByRole('button', { name: 'Refresh evidence', exact: true }).click();
  await page.waitForFunction(() => document.querySelector('#scenario-public').getAttribute('aria-pressed') === 'true');
  assert.equal(await page.locator('#post-title').inputValue(), publicMessage.title);
  assert.equal(await page.locator('#post-text').inputValue(), publicMessage.text);
  assert.equal(await page.locator('#app-preview').getAttribute('open'), null);
  assert.equal(await page.locator('#app-screen').getAttribute('src'), null);
  assert.equal(await page.locator('#app-screen').isHidden(), true);
  await page.reload();
  await page.waitForFunction(() => document.querySelector('#scenario-public').getAttribute('aria-pressed') === 'true');
  assert.equal(await page.locator('#post-title').inputValue(), publicMessage.title);
  assert.match(await page.locator('#step-location').textContent(), /Public/);
  assert.equal(await page.locator('#crypto-visual').isHidden(), true);
  assert.equal(await page.locator('#story-player').getAttribute('data-frame'), 'public-existing-connections');
  assert.match(await page.locator('#story-evidence').innerText(), /Illustrated explanation/);
  assert.equal(await page.locator('#story-map .story-route').count(), 2, 'earlier node contacts do not imply a node-to-node link');
  assert.equal(await page.locator('#story-map [data-registry="true"]').count(), 0);
  assert.match(await page.locator('#story-members').innerText(), /not yet assumed/);
  for (const width of [1280, 320]) {
    await page.setViewportSize({ width, height: 960 });
    await page.waitForFunction(narrow => document.querySelector('#story-map').dataset.narrow === String(narrow), width === 320);
    const boxes = await page.locator('#story-map .story-device').evaluateAll(cards => cards.map(card => {
      const { left, right, top, bottom } = card.getBoundingClientRect(); return { left, right, top, bottom };
    }));
    for (const [index, box] of boxes.entries()) for (const other of boxes.slice(index + 1)) {
      assert.equal(box.left < other.right && box.right > other.left && box.top < other.bottom && box.bottom > other.top, false, 'context labels stay separate');
    }
  }
  await page.locator('#story-next').click();
  assert.equal(await page.locator('#story-player').getAttribute('data-frame'), 'public-introduce-community');
  assert.match(await page.locator('#story-output').innerText(), /invitation alone does not notify every node/);
  assert.equal(await page.locator('#story-map .story-marker-label').textContent(), 'Carry request');
  assert.equal(await page.locator('#story-map .story-route[data-edge="node-a-node-b-public"]').count(), 0);
  await page.locator('#journey-details > summary').click();
  await page.getByRole('button', { name: '2 Sign', exact: true }).click();
  assert.match(await page.locator('#map-caption').textContent(), /Completed public run/);
  assert.equal(await page.locator('#network-map .journey-evidence-badge[data-evidence="inferred"]').count(), 3);
  await page.locator('#network-map button[data-edge="node-a-node-b-public"]').click();
  assert.match(await page.locator('#selection-source').textContent(), /inferred.*packets were not captured/i);
  assert.doesNotMatch(await page.locator('#step-result').textContent(), /not happened/);
  publicJourney.id = 'journey-public-bob'; publicJourney.author = 'bob'; publicJourney.receiver = 'alice';
  publicJourney.flow = describeFlow({ visibility: 'public', journey: publicJourney, topology });
  await page.locator('#refresh').click();
  await page.waitForFunction(() => document.querySelector('#step-location').textContent.includes('Bob → Alice'));
  await page.locator('#journey-details > summary').click();
  await page.setViewportSize({ width: 1280, height: 960 });
  await page.waitForFunction(() => {
    const map = document.querySelector('#network-map');
    const positions = ['bob', 'node-b', 'node-a', 'alice'].map(id => map.querySelector('[data-node="' + id + '"]').getBoundingClientRect().x);
    return innerWidth === 1280 && positions.every((position, index) => index === 0 || position > positions[index - 1]);
  });
  const routePositions = await page.locator('#network-map').evaluate(map => ['bob', 'node-b', 'node-a', 'alice'].map(id => map.querySelector('[data-node="' + id + '"]').getBoundingClientRect().x));
  assert.equal(routePositions.every((position, index) => index === 0 || position > routePositions[index - 1]), true, 'Bob uses node B before node A');
  state.journey = journey(3);
  state.journey.id = 'journey-failed';
  state.journey.error = 'The receiving core rejected the encrypted object.';
  state.journey.failed = true; state.journey.nextAction = null;
  state.journey.crypto = describeCryptoJourney(state.journey);
  state.journey.flow.edges = state.journey.flow.edges.map(edge => ({ ...edge, historical: edge.status === 'observed' }));
  state.journey.permissions = permissions(3).map(permission => permission.id === 'alice' ? permission : { ...permission, state: 'unknown', detail: 'This step did not finish; reset before drawing conclusions about this copy.' });
  await page.getByRole('button', { name: 'Refresh evidence', exact: true }).click();
  await page.getByRole('alert').filter({ hasText: 'receiving core rejected' }).waitFor();
  assert.match(await page.locator('#readability').textContent(), /Not verified/);
  assert.match(await page.locator('#journey-recovery').textContent(), /Reset starts a fresh demonstration/);
  assert.equal(await page.getByRole('button', { name: 'Reset and start again', exact: true }).isVisible(), true);
  assert.equal(await page.locator('#next').isDisabled(), true);
  assert.equal(await page.locator('#prepare').isDisabled(), true);
  assert.match(await page.locator('#network-map button[data-edge="store-a"]').textContent(), /Captured before failure/);
  state.journey = { ...journey(4), id: 'failed-after-recovery', failed: true, error: 'The final observation failed.', nextAction: null };
  state.journey.crypto = describeCryptoJourney(state.journey);
  await page.locator('#refresh').click();
  await page.waitForFunction(() => document.querySelector('#journey-error').textContent.includes('final observation failed'));
  assert.equal(await page.locator('#message-comparison').isHidden(), true);
  assert.equal(await page.locator('#app-preview').isHidden(), true);
  assert.deepEqual(errors, []);
});

async function recoveryFixture(t, { status = 'ready', delayFailureRefresh = false, delayReset = false, recoverable = false } = {}) {
  let state = { status, phase: status === 'error' ? 'Startup failed.' : 'Ready', busy: false, session: 'recovery-session', journey: status === 'ready' ? journey(0) : null, services: [] };
  if (recoverable) Object.assign(state.journey, { failed: false, recoverable: true, error: 'This exact message is already present. Edit the title or message and prepare it again.' });
  let disconnected = false, failedMutation = false, delayed = false, recoveryStarted, resetStarted, releaseRecovery, releaseReset;
  const recoveryRequested = new Promise(resolve => { recoveryStarted = resolve; });
  const resetRequested = new Promise(resolve => { resetStarted = resolve; });
  const requests = [];
  const server = createServer(async (req, res) => {
    const json = (code, value) => { res.writeHead(code, { 'content-type': 'application/json' }); res.end(JSON.stringify(value)); };
    if (req.url === '/api/state') {
      if (disconnected) return json(503, { error: 'The local snapshot is temporarily unavailable.' });
      if (delayFailureRefresh && failedMutation && !delayed) {
        delayed = true;
        const snapshot = structuredClone(state);
        releaseRecovery = () => json(200, snapshot);
        recoveryStarted(); return;
      }
      return json(200, state);
    }
    if (req.url === '/api/journey/next' || req.url === '/api/journey/start' || req.url === '/api/reset') {
      let body = ''; for await (const part of req) body += part;
      const payload = JSON.parse(body);
      requests.push({ path: req.url, payload, control: req.headers['x-sandbox-control'] });
      if (req.url === '/api/journey/start') {
        state.journey = { ...journey(0, payload), id: 'journey-replacement' };
        return json(200, state);
      }
      if (req.url === '/api/reset') {
        releaseReset = () => {
          state = { ...state, status: 'ready', busy: false, phase: 'Fresh demonstration ready.', journey: null };
          json(200, state);
        };
        resetStarted();
        if (delayReset) { state.busy = true; return; }
        return releaseReset();
      }
      failedMutation = true;
      state.journey.failed = true; state.journey.nextAction = null;
      state.journey.error = 'This operation may have partially completed. Reset before continuing.';
      state.journey.permissions = permissions(0).map(permission => permission.id === 'alice' ? permission : { ...permission, state: 'unknown', detail: 'This copy has not been verified.' });
      return json(500, { error: 'The operation failed.' });
    }
    const paths = { '/': 'public/walkthrough.html', '/walkthrough.mjs': 'public/walkthrough.mjs', '/walkthrough-player.mjs': 'public/walkthrough-player.mjs', '/walkthrough-story.mjs': 'public/walkthrough-story.mjs', '/walkthrough.css': 'public/walkthrough.css', '/shared/tokens.css': '../packages/tokens/generated/tokens.css', '/shared/site.css': '../apps/site/src/styles/site.css' };
    if (!paths[req.url]) { res.writeHead(404).end(); return; }
    res.setHeader('content-type', req.url.endsWith('.mjs') ? 'text/javascript' : req.url.endsWith('.css') ? 'text/css' : 'text/html');
    res.end(await readFile(new URL(paths[req.url], import.meta.url)));
  });
  await new Promise(done => server.listen(0, '127.0.0.1', done));
  t.after(() => new Promise(done => { server.closeAllConnections(); server.close(done); }));
  const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
  t.after(() => browser.close());
  const page = await browser.newPage({ viewport: { width: 1280, height: 960 }, reducedMotion: 'reduce' });
  await page.goto('http://127.0.0.1:' + server.address().port + '/');
  await page.waitForFunction(expected => document.querySelector('#network-status').dataset.state === expected, status);
  if (status === 'ready') await page.locator('#journey-details > summary').click();
  return { page, requests, recoveryRequested, resetRequested, releaseRecovery() { releaseRecovery(); }, releaseReset() { releaseReset(); }, setDisconnected(value) { disconnected = value; } };
}

test('a failed operation leaves visible recovery available even while its evidence refresh is delayed', async t => {
  const { page, requests, recoveryRequested } = await recoveryFixture(t, { delayFailureRefresh: true });
  await page.getByRole('button', { name: actions[0], exact: true }).click();
  await recoveryRequested;
  await page.waitForFunction(() => !document.querySelector('#refresh').disabled, null, { timeout: 2000 });
  assert.equal(await page.locator('#reset').isEnabled(), true, 'reset does not inherit a failed request lock');
  await page.getByRole('button', { name: 'Refresh evidence', exact: true }).click();
  await page.locator('#recovery-reset').waitFor({ state: 'visible', timeout: 2000 });
  assert.equal(await page.locator('#next').isDisabled(), true);
  assert.equal(await page.locator('#prepare').isDisabled(), true);
  assert.doesNotMatch(await page.locator('#step-result').textContent(), /The message has not been sent/);
  await page.locator('#recovery-reset').click();
  await page.locator('#reset-dialog').getByRole('button', { name: 'Reset demonstration', exact: true }).click();
  await page.waitForFunction(() => !document.querySelector('#prepare').disabled);
  assert.equal(await page.locator('#recovery-reset').isHidden(), true);
  assert.equal(await page.locator('#journey-error').isHidden(), true);
  assert.equal(await page.locator('#post-title').isEnabled(), true);
  assert.deepEqual(requests.map(request => request.path), ['/api/journey/next', '/api/reset']);
  assert.deepEqual(requests[1], { path: '/api/reset', payload: {}, control: '1' });
});

test('a connected network error can be reset from the visible header control', async t => {
  const { page, requests } = await recoveryFixture(t, { status: 'error' });
  assert.equal(await page.locator('#reset').isVisible(), true);
  assert.equal(await page.locator('#reset').isEnabled(), true);
  assert.equal(await page.locator('#prepare').isDisabled(), true);
  await page.locator('#reset').click();
  await page.locator('#reset-dialog').getByRole('button', { name: 'Reset demonstration', exact: true }).click();
  await page.waitForFunction(() => !document.querySelector('#prepare').disabled);
  assert.equal(requests.length, 1);
  assert.equal(requests[0].path, '/api/reset');
});

test('refresh can reconnect after snapshot failure without enabling a stale next action', async t => {
  const fixture = await recoveryFixture(t);
  const { page } = fixture;
  fixture.setDisconnected(true);
  await page.locator('#refresh').click();
  await page.waitForFunction(() => document.querySelector('#network-status').textContent === 'Disconnected');
  assert.equal(await page.locator('#refresh').isEnabled(), true);
  assert.equal(await page.locator('#next').isDisabled(), true);
  fixture.setDisconnected(false);
  await page.locator('#refresh').click();
  await page.waitForFunction(() => document.querySelector('#network-status').textContent === 'Network ready');
  assert.equal(await page.locator('#next').isEnabled(), true);
});

test('a recoverable duplicate draft remains editable and can be replaced without reset', async t => {
  const { page, requests } = await recoveryFixture(t, { recoverable: true });
  assert.match(await page.locator('#journey-error').textContent(), /already present/);
  assert.match(await page.locator('#next-explanation').textContent(), /Edit the title or message/);
  assert.equal(await page.locator('#journey-recovery').isHidden(), true);
  for (const selector of ['#post-title', '#post-text', '#prepare']) assert.equal(await page.locator(selector).isEnabled(), true);
  assert.equal(await page.locator('#next').isHidden(), true, 'prepare the edited draft before attempting to publish again');
  await page.locator('#post-title').fill('A changed meeting');
  await page.locator('#post-text').fill('Meet by the red door at eight.');
  await page.locator('#prepare').click();
  await page.waitForFunction(() => document.querySelector('#journey-error').hidden);
  assert.deepEqual(requests, [{ path: '/api/journey/start', payload: { author: 'alice', visibility: 'private', title: 'A changed meeting', text: 'Meet by the red door at eight.' }, control: '1' }]);
  assert.match(await page.locator('#step-input').textContent(), /A changed meeting/);
  assert.match(await page.locator('#step-input').textContent(), /Meet by the red door at eight/);
  assert.equal(await page.locator('#next').isEnabled(), true);
});

test('an earlier delayed observation cannot unlock a newer reset operation', async t => {
  const fixture = await recoveryFixture(t, { delayFailureRefresh: true, delayReset: true });
  const { page, recoveryRequested, resetRequested } = fixture;
  await page.locator('#next').click();
  await recoveryRequested;
  await page.locator('#refresh').click();
  await page.locator('#recovery-reset').waitFor({ state: 'visible' });
  await page.locator('#recovery-reset').click();
  await page.locator('#reset-dialog').getByRole('button', { name: 'Reset demonstration', exact: true }).click();
  await resetRequested;
  const staleResponse = page.waitForResponse(response => response.url().endsWith('/api/state'));
  fixture.releaseRecovery();
  await (await staleResponse).finished();
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(resolve)));
  assert.equal(await page.locator('#reset').isDisabled(), true, 'the old failed operation cannot clear the active reset lock');
  assert.equal(await page.locator('#recovery-reset').isDisabled(), true);
  assert.equal(await page.locator('#prepare').isDisabled(), true);
  assert.equal(await page.locator('#refresh').isEnabled(), true);
  fixture.releaseReset();
  await page.waitForFunction(() => !document.querySelector('#prepare').disabled);
  assert.equal(await page.locator('#journey-recovery').isHidden(), true);
  assert.equal(await page.locator('#journey-error').isHidden(), true);
});
