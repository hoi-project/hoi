import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { createHash } from 'node:crypto';
import { chromium } from 'playwright';
import { describeCryptoJourney } from './walkthrough-crypto-view.mjs';
import { describeFlow } from './walkthrough-flow.mjs';

const stages = ['draft', 'seal', 'store', 'download', 'open'];
const text = 'Meet beside the blue door.';
const object = { alias: 'E1', id: createHash('sha256').update(Buffer.alloc(4096, 0xab)).digest('hex'), size: 4096, hex: 'ab'.repeat(4096), expires: 1800000000,
  segments: [{ name: 'Nonce', offset: 0, bytes: 24 }, { name: 'Encrypted bytes', offset: 24, bytes: 4056 }, { name: 'Authentication tag', offset: 4080, bytes: 16 }] };
const connections = { private: { communityId: 'c'.repeat(64), name: 'Our group', sameCommunity: true, sameRoom: true, sameEpoch: true,
  clients: ['alice', 'bob'].map(id => ({ id, active: true, communityId: 'c'.repeat(64), epoch: 1, memberCount: 2 })),
  invitation: { method: 'local-handoff', completed: true }, relays: ['relay-a', 'relay-b'] } };
const availability = ['relay-a', 'relay-b'].map(service => ({ service, actor: 'alice', path: '/relay/v2/meta', status: 200, apiVersion: 2 }));

function snapshot(index, message = text) {
  const journey = { id: 'example', author: 'alice', receiver: 'bob', visibility: 'private', stage: stages[index], complete: index === 4,
    message: { title: 'A private plan', text: message }, community: { id: connections.private.communityId, name: 'Our group' }, objects: [], downloads: [], stored: [], uploads: [] };
  if (index >= 1) {
    journey.objects = [object];
    journey.post = { alias: 'P1', id: 'e'.repeat(64), title: journey.message.title, text: message, checks: { signature: 'valid' } };
  }
  if (index >= 2) for (const service of ['relay-a', 'relay-b']) {
    journey.stored.push({ service, objectId: object.id, identical: true });
    journey.uploads.push({ service, objectId: object.id, sha256: object.id, status: 201, receipt: { id: object.id, size: object.size, expires: object.expires } });
  }
  if (index >= 3) journey.downloads.push({ service: 'relay-a', objectAlias: 'E1', objectId: object.id, sha256: object.id, size: object.size, status: 200, identical: true, readProofUsed: true, hex: object.hex });
  if (index === 4) journey.recovered = { ...journey.post, samePost: true, sameTitle: true, sameText: true };
  journey.crypto = describeCryptoJourney(journey);
  journey.flow = describeFlow({ visibility: 'private', journey, availability });
  return { status: 'ready', session: 'session-one', journey, connections, flows: { private: describeFlow({ visibility: 'private', availability }) } };
}

async function fixture(t, { stage = 0, motion = 'reduce', message = text } = {}) {
  const states = stages.map((_, index) => snapshot(index, message));
  const script = `import { createPlayer } from '/walkthrough-player.mjs';
    const states = ${JSON.stringify(states)};
    let state = structuredClone(states[${stage}]), hold = false, release, busy = false;
    const requests = [];
    const player = createPlayer({ host: document.querySelector('#player'), getState: () => state, getScenario: () => 'private', canRun: () => !busy,
      onAdvance: async () => {
        const before = state.journey.stage; requests.push(before);
        if (hold) { hold = false; await new Promise(done => { release = done; }); }
        const index = ['draft','seal','store','download','open'].indexOf(before);
        state = { ...structuredClone(states[index + 1]), session: state.session, journey: { ...structuredClone(states[index + 1].journey), id: state.journey.id } };
        player.update(); return true;
      }
    });
    window.playerTest = { requests, hold() { hold = true; }, release() { release(); }, getState() { return state; },
      reset() { state = structuredClone(states[0]); state.session = 'session-two'; state.journey.id = 'new-example'; player.update(); },
      fail() { state.journey.failed = true; player.update(); },
      hide() { Object.defineProperty(document, 'hidden', { configurable: true, value: true }); document.dispatchEvent(new Event('visibilitychange')); },
      busy(value) { busy = value; player.update(); }
    };
    player.update();`;
  const paths = { '/walkthrough-player.mjs': 'public/walkthrough-player.mjs', '/walkthrough-story.mjs': 'public/walkthrough-story.mjs', '/walkthrough.css': 'public/walkthrough.css', '/tokens.css': '../packages/tokens/generated/tokens.css', '/site.css': '../apps/site/src/styles/site.css' };
  const server = createServer(async (req, res) => {
    if (req.url === '/favicon.ico') { res.writeHead(204).end(); return; }
    if (req.url === '/') {
      res.setHeader('content-type', 'text/html');
      res.end('<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"><link rel="stylesheet" href="/tokens.css"><link rel="stylesheet" href="/site.css"><link rel="stylesheet" href="/walkthrough.css"></head><body><main class="wrap"><section id="player" class="story-player"></section></main><script type="module" src="/fixture.mjs"></script></body></html>'); return;
    }
    if (req.url === '/fixture.mjs') { res.setHeader('content-type', 'text/javascript'); res.end(script); return; }
    const path = paths[req.url];
    if (!path) { res.writeHead(404).end(); return; }
    res.setHeader('content-type', path.endsWith('.mjs') ? 'text/javascript' : 'text/css'); res.end(await readFile(new URL(path, import.meta.url)));
  });
  await new Promise(done => server.listen(0, '127.0.0.1', done));
  t.after(() => new Promise(done => { server.closeAllConnections(); server.close(done); }));
  const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
  t.after(() => browser.close());
  const page = await browser.newPage({ viewport: { width: 1280, height: 960 }, reducedMotion: motion });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
  await page.goto('http://127.0.0.1:' + server.address().port);
  await page.locator('#story-next').waitFor();
  const frame = () => page.locator('#player').getAttribute('data-frame');
  const step = async expected => { await page.locator('#story-next').click(); await page.waitForFunction(id => document.querySelector('#player').dataset.frame === id, expected); };
  const requests = () => page.evaluate(() => [...window.playerTest.requests]);
  t.after(() => assert.deepEqual(errors, [], 'the player must not raise browser errors'));
  return { page, frame, step, requests };
}

test('Next runs each real private operation once; inner steps and replay inspect captured results', async t => {
  const { page, frame, step, requests } = await fixture(t);
  await step('private-invitation'); await step('private-relays'); await step('message-input');
  assert.deepEqual(await requests(), []);
  await step('protect-signed-post');
  assert.deepEqual(await requests(), ['draft']);
  for (const id of ['protect-mls', 'protect-padding', 'protect-outer']) await step(id);
  assert.deepEqual(await requests(), ['draft'], 'the inner explanation must not rerun encryption');
  await page.locator('[data-byte-part="Nonce"]').click();
  assert.equal(await page.locator('.story-byte-preview').textContent(), 'ab'.repeat(24));
  await step('upload-relay-a'); await step('upload-relay-b');
  assert.deepEqual(await requests(), ['draft', 'seal'], 'both replica frames review one store operation');
  await step('receiver-request'); await step('receiver-download');
  assert.deepEqual(await requests(), ['draft', 'seal', 'store']);
  assert.doesNotMatch(await page.locator('#story-output').innerText(), /Meet beside/);
  for (const id of ['open-outer-open', 'open-unpad', 'open-mls-open', 'open-validate']) {
    await step(id); assert.doesNotMatch(await page.locator('#story-output').innerText(), /Meet beside/);
  }
  await step('open-readable');
  assert.match(await page.locator('#story-output').innerText(), /Meet beside the blue door/);
  assert.deepEqual(await requests(), ['draft', 'seal', 'store', 'download']);
  await page.locator('#story-back').click(); assert.equal(await frame(), 'open-validate');
  await page.locator('#story-replay').click(); assert.equal(await frame(), 'private-community');
  for (const id of ['private-invitation', 'private-relays', 'message-input', 'protect-signed-post']) await step(id);
  assert.deepEqual(await requests(), ['draft', 'seal', 'store', 'download'], 'back and replay must not publish again');
});

test('Pause while a core call is pending retains its result without advancing the animation', async t => {
  const { page, frame, step, requests } = await fixture(t);
  await step('private-invitation'); await step('private-relays'); await step('message-input');
  await page.evaluate(() => window.playerTest.hold());
  await page.locator('#story-play').click();
  await page.waitForFunction(() => window.playerTest.requests.length === 1);
  assert.equal(await page.locator('#story-play').innerText(), 'Pause');
  await page.locator('#story-play').click();
  await page.evaluate(() => window.playerTest.release());
  await page.waitForFunction(() => document.querySelector('#player').dataset.running === 'false');
  assert.equal(await frame(), 'message-input');
  assert.equal(await page.evaluate(() => window.playerTest.getState().journey.stage), 'seal');
  assert.equal(await page.locator('#player').getAttribute('data-playing'), 'false');
  await page.waitForTimeout(4200);
  assert.equal(await frame(), 'message-input', 'a finished request must not restart a paused timer');
  assert.deepEqual(await requests(), ['draft']);
  await step('protect-signed-post');
  assert.deepEqual(await requests(), ['draft'], 'the next click reviews the completed call');
});

test('reset identity and a hidden page stop playback, including a pending call', async t => {
  const { page, frame, step, requests } = await fixture(t);
  await step('private-invitation'); await step('private-relays'); await step('message-input');
  await page.evaluate(() => window.playerTest.hold());
  await page.locator('#story-play').click();
  await page.waitForFunction(() => window.playerTest.requests.length === 1);
  await page.evaluate(() => window.playerTest.reset());
  assert.equal(await frame(), 'private-community');
  assert.equal(await page.locator('#player').getAttribute('data-playing'), 'false');
  await page.evaluate(() => window.playerTest.release());
  await page.waitForFunction(() => document.querySelector('#player').dataset.running === 'false');
  assert.equal(await frame(), 'private-community', 'the previous identity’s callback must not move the new story');
  await page.locator('#story-play').click();
  await page.waitForFunction(() => document.querySelector('#player').dataset.frame === 'private-invitation');
  await page.evaluate(() => window.playerTest.hide());
  assert.equal(await page.locator('#player').getAttribute('data-playing'), 'false');
  assert.equal(await page.locator('#story-play').innerText(), 'Play');
  assert.deepEqual(await requests(), ['draft']);
});

test('a failed final observation removes readable claims and keeps recovery possible', async t => {
  const { page, step } = await fixture(t, { stage: 4 });
  for (const id of ['private-invitation', 'private-relays', 'message-input', 'protect-signed-post', 'protect-mls', 'protect-padding', 'protect-outer', 'upload-relay-a', 'upload-relay-b', 'receiver-request', 'receiver-download', 'open-outer-open', 'open-unpad', 'open-mls-open', 'open-validate', 'open-readable']) await step(id);
  await page.evaluate(() => window.playerTest.fail());
  assert.doesNotMatch(await page.locator('#story-output').innerText(), /Meet beside/);
  assert.doesNotMatch(await page.locator('#story-next').innerText(), /can read/i);
  assert.doesNotMatch(await page.locator('[data-node="bob"]').innerText(), /Can read/);
  assert.match(await page.locator('#story-status').innerText(), /Stopped|Reset/);
  assert.equal(await page.locator('#story-next').isEnabled(), false);
  assert.equal(await page.locator('#story-replay').isEnabled(), true);
});

test('normal motion moves a message marker; reduced motion keeps the route marker static', async t => {
  const moving = await fixture(t, { motion: 'no-preference' });
  await moving.step('private-invitation');
  const start = await moving.page.locator('.story-marker').getAttribute('cx');
  await moving.page.waitForFunction(previous => document.querySelector('.story-marker').getAttribute('cx') !== previous, start);
  const end = await moving.page.locator('.story-route[data-active="true"]').evaluate(path => path.getPointAtLength(path.getTotalLength()).x);
  await moving.page.waitForFunction(target => Math.abs(Number(document.querySelector('.story-marker').getAttribute('cx')) - target) < 1, end);
  const staticView = await fixture(t, { motion: 'reduce' });
  await staticView.step('private-invitation');
  assert.equal(await staticView.page.locator('#player').getAttribute('data-reduced-motion'), 'true');
  const marker = await staticView.page.locator('.story-marker').getAttribute('cx');
  await staticView.page.evaluate(() => new Promise(done => requestAnimationFrame(() => requestAnimationFrame(done))));
  assert.equal(await staticView.page.locator('.story-marker').getAttribute('cx'), marker);
  const expected = await staticView.page.locator('.story-route[data-active="true"]').evaluate(path => path.getPointAtLength(path.getTotalLength()).x);
  assert.ok(Math.abs(Number(marker) - expected) < 1);
});

test('the central scene keeps device cards separated at desktop and phone widths', async t => {
  const { page, step } = await fixture(t, { stage: 4 });
  for (const id of ['private-invitation', 'private-relays', 'message-input', 'protect-signed-post', 'protect-mls', 'protect-padding', 'protect-outer']) await step(id);
  for (const width of [1280, 320]) {
    await page.setViewportSize({ width, height: 960 });
    await page.waitForFunction(narrow => document.querySelector('#story-map').dataset.narrow === String(narrow), width === 320);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, 'the scene must fit the viewport');
    const boxes = await page.locator('.story-device').evaluateAll(items => items.map(item => {
      const { left, right, top, bottom } = item.getBoundingClientRect(); return { left, right, top, bottom };
    }));
    const communityBottom = await page.locator('.story-community').evaluate(item => item.getBoundingClientRect().bottom);
    assert.ok(boxes.every(box => box.top >= communityBottom), 'active device cards must stay below the community banner');
    for (const [index, box] of boxes.entries()) for (const other of boxes.slice(index + 1)) {
      const overlaps = box.left < other.right && box.right > other.left && box.top < other.bottom && box.bottom > other.top;
      assert.equal(overlaps, false, 'device labels must not overlap at ' + width + 'px');
    }
  }
});

test('input and output grow equally without scrollbars for long text and expanded layers', async t => {
  const { page, step } = await fixture(t, { stage: 4, message: 'A longer message stays fully visible. '.repeat(48) });
  const checkPanels = async () => {
    const layout = await page.locator('.story-card').evaluateAll(cards => cards.map(card => {
      const content = card.querySelector(':scope > div');
      return { height: card.getBoundingClientRect().height, contentHeight: content.clientHeight,
        scrollHeight: content.scrollHeight, contentWidth: content.clientWidth, scrollWidth: content.scrollWidth,
        overflow: getComputedStyle(content).overflowY };
    }));
    assert.ok(Math.abs(layout[0].height - layout[1].height) < 1, 'both panels have equal height');
    for (const item of layout) {
      assert.ok(item.scrollHeight <= item.contentHeight + 1, 'all content fits vertically');
      assert.ok(item.scrollWidth <= item.contentWidth + 1, 'all content wraps horizontally');
      assert.equal(item.overflow, 'visible', 'content is not a scroll area');
    }
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  };
  await step('private-invitation'); await step('private-relays'); await step('message-input');
  for (const width of [1280, 320]) {
    await page.setViewportSize({ width, height: 960 });
    await page.waitForFunction(narrow => document.querySelector('#story-map').dataset.narrow === String(narrow), width === 320);
    await checkPanels();
  }
  for (const id of ['protect-signed-post', 'protect-mls', 'protect-padding', 'protect-outer']) await step(id);
  for (const width of [1280, 320]) {
    await page.setViewportSize({ width, height: 960 });
    await page.waitForFunction(narrow => document.querySelector('#story-map').dataset.narrow === String(narrow), width === 320);
    while (await page.locator('#story-input details:not([open]) > summary').count()) await page.locator('#story-input details:not([open]) > summary').first().click();
    await checkPanels();
  }
});
