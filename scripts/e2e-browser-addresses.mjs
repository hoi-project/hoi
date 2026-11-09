// The You screen with an offline core and browser-reported connection statistics.
import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
import { resolve } from 'node:path';
import { chromium } from 'playwright';
import { createServer } from '../apps/web/node_modules/vite/dist/node/index.js';

const root = resolve(import.meta.dirname, '..');
process.env.VITE_HOI_BACKEND = 'mock';
const server = await createServer({ root: resolve(root, 'apps/web'), logLevel: 'error', server: { host: '127.0.0.1', port: 0, hmr: false }, plugins: [{
  name: 'browser-address-test-page',
  configureServer(server) {
    server.middlewares.use('/browser-address-test', async (_req, res) => {
      res.setHeader('Content-Type', 'text/html');
      res.end(await server.transformIndexHtml('/browser-address-test', `<!doctype html><html lang="en"><title>Connection address test</title>
        <main id="fixture" class="gutter"></main>
        <script type="module">
          import { mount } from 'svelte';
          import You from '/src/routes/You.svelte';
          import { app } from '/src/lib/app.svelte.ts';
          import '@hoi/tokens/fonts.css';
          import '@hoi/tokens/tokens.css';
          import '/src/app.css';
          await app.start();
          if (app.phase !== 'ready') throw new Error('Mock app did not start.');
          app.refresh = async () => {};
          app.refreshWarmth = async () => {};
          app.net = { ...app.net, connections: [], peers: 0 };
          window.addressFixture = { app };
          mount(You, { target: document.querySelector('#fixture') });
        </script></html>`));
    });
  },
}] });
await server.listen();
const origin = `http://127.0.0.1:${server.httpServer.address().port}`;
let browser;
const errors = [];
const externalRequests = [];
try {
  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
  const context = await browser.newContext({ reducedMotion: 'reduce', viewport: { width: 1280, height: 900 }, serviceWorkers: 'block' });
  await context.route('**/*', route => {
    if (new URL(route.request().url()).origin === origin) return route.continue();
    externalRequests.push(route.request().url());
    return route.abort();
  });
  await context.addInitScript(() => {
    Object.defineProperty(navigator, 'clipboard', { value: { writeText: async text => { window.copiedAddress = text; } } });
  });
  const page = await context.newPage();
  page.on('pageerror', error => errors.push(error.message));
  page.setDefaultTimeout(15_000);
  await page.goto(`${origin}/browser-address-test?me=Example`);
  const panel = page.locator('.set').filter({ has: page.getByText('Your address', { exact: true }) });
  await panel.waitFor();
  assert.equal((await panel.innerText()).includes('publishes none'), false, 'the browser address panel must not imply that a missing listening address hides its connection IP');
  await panel.getByText('No active connections.', { exact: true }).waitFor();
  await mkdir(resolve(root, 'test-results/49-addresses'), { recursive: true });
  await panel.screenshot({ path: resolve(root, 'test-results/49-addresses/disconnected.png') });

  await page.evaluate(() => {
    const { app } = window.addressFixture;
    app.net = { ...app.net, connections: [{ peer: 'example-node', role: 'peer', route: 'direct', connected: true, transport: 'websocket', encryption: 'encrypted' }] };
  });
  await panel.getByText('Connection IP unavailable.', { exact: true }).waitFor();
  assert.match(await panel.innerText(), /other device can still see it/i, 'unavailable is not a claim of hidden network metadata');

  await page.evaluate(async () => {
    const { rtcAddresses } = await import('/src/lib/core/rtc-addresses.ts');
    class Peer extends EventTarget {
      connectionState = 'connected';
      iceConnectionState = 'connected';
      iceGatheringState = 'complete';
      address = '192.0.2.25';
      candidateType = 'srflx';
      sctp = { transport: { iceTransport: new EventTarget() } };
      async getStats() {
        return new Map([
          ['transport', { id: 'transport', type: 'transport', selectedCandidatePairId: 'selected' }],
          ['selected', { id: 'selected', type: 'candidate-pair', state: 'succeeded', localCandidateId: 'local', remoteCandidateId: 'remote' }],
          ['local', { id: 'local', type: 'local-candidate', candidateType: this.candidateType, address: this.address }],
          ['remote', { id: 'remote', type: 'remote-candidate', candidateType: 'host', address: '198.51.100.30' }],
        ]);
      }
    }
    window.addressFixture.pc = new Peer();
    window.addressFixture.stop = rtcAddresses.track(window.addressFixture.pc, 'node');
    window.addressFixture.Peer = Peer;
  });
  await panel.getByText('192.0.2.25', { exact: true }).waitFor();
  assert.equal((await panel.innerText()).includes('198.51.100.30'), false, 'the readout never substitutes the remote node address');
  assert.match(await panel.innerText(), /WebRTC reports/i, 'the display identifies the browser report rather than promising one universal outgoing IP');
  assert.equal(await panel.getByText('Connection IP unavailable.', { exact: true }).count(), 1, 'a new RTC observation must not mask an existing WebSocket with the same aggregate count');
  await page.evaluate(() => {
    const { app } = window.addressFixture;
    app.net = { ...app.net, connections: app.net.connections.map(connection => ({ ...connection, transport: 'webrtc' })) };
  });
  const copy = panel.getByRole('button', { name: /Copy IP address/ }).first();
  await copy.focus();
  await page.keyboard.press('Enter');
  assert.equal(await page.evaluate(() => window.copiedAddress), '192.0.2.25', 'the address is copyable with the keyboard');
  await page.evaluate(() => {
    const { app } = window.addressFixture;
    app.net = { ...app.net, connections: [...app.net.connections, { ...app.net.connections[0], peer: 'another-example-node' }] };
  });
  await panel.getByText('Connection IP unavailable.', { exact: true }).waitFor();
  assert.match(await panel.innerText(), /cannot determine the source IP for every connection/, 'one known address must not claim to cover another connection');
  assert.equal(await panel.getByText('192.0.2.25', { exact: true }).count(), 1, 'a partial observation preserves the known address');
  await page.evaluate(() => {
    const { app } = window.addressFixture;
    app.net = { ...app.net, connections: app.net.connections.slice(0, 1) };
  });

  for (const width of [1280, 320]) {
    await page.setViewportSize({ width, height: 900 });
    await panel.scrollIntoViewIfNeeded();
    const geometry = await panel.evaluate(el => {
      const bounds = el.getBoundingClientRect();
      return { left: bounds.left, right: bounds.right, width: window.innerWidth, scroll: el.scrollWidth, client: el.clientWidth };
    });
    assert.ok(geometry.left >= 0 && geometry.right <= geometry.width + 1, `address panel fits ${width} px viewport`);
    assert.ok(geometry.scroll <= geometry.client + 1, `address content does not overflow at ${width} px`);
    await panel.screenshot({ path: resolve(root, `test-results/49-addresses/known-${width}.png`) });
  }

  await page.evaluate(() => {
    const { pc } = window.addressFixture;
    pc.address = '2001:db8:abcd:ffff:1234:5678:90ab:cdef';
    pc.candidateType = 'prflx';
    pc.sctp.transport.iceTransport.dispatchEvent(new Event('selectedcandidatepairchange'));
  });
  await panel.getByText('2001:db8:abcd:ffff:1234:5678:90ab:cdef', { exact: true }).waitFor();
  assert.equal(await panel.getByText('192.0.2.25', { exact: true }).count(), 0, 'network changes remove the old readout');
  assert.ok(await panel.evaluate(el => el.scrollWidth <= el.clientWidth + 1), 'a full IPv6 address wraps at 320 px');
  await panel.screenshot({ path: resolve(root, 'test-results/49-addresses/ipv6-320.png') });

  await page.evaluate(() => {
    const { pc } = window.addressFixture;
    pc.address = '192.0.2.26';
    pc.candidateType = 'host';
    pc.sctp.transport.iceTransport.dispatchEvent(new Event('selectedcandidatepairchange'));
  });
  await panel.getByText('Connection IP unavailable.', { exact: true }).waitFor();
  assert.equal(await panel.getByText('2001:db8:abcd:ffff:1234:5678:90ab:cdef', { exact: true }).count(), 0, 'an unavailable result replaces the previous known IP');
  assert.equal(await panel.getByText('192.0.2.26', { exact: true }).count(), 0, 'a host/interface candidate is not presented as a peer-observed external address');

  await page.evaluate(() => {
    const fixture = window.addressFixture;
    fixture.stop();
    fixture.pc = new fixture.Peer();
    fixture.pc.address = '2001:db8::30';
  });
  await page.evaluate(async () => {
    const { rtcAddresses } = await import('/src/lib/core/rtc-addresses.ts');
    window.addressFixture.stop = rtcAddresses.track(window.addressFixture.pc, 'browser');
  });
  await panel.getByText('2001:db8::30', { exact: true }).waitFor();
  assert.match(await panel.innerText(), /Browser connection/, 'direct browser links retain their own source context');
  await page.evaluate(() => {
    const { app, pc } = window.addressFixture;
    app.net = { ...app.net, connections: [] };
    pc.connectionState = 'disconnected';
    pc.iceConnectionState = 'disconnected';
    pc.dispatchEvent(new Event('connectionstatechange'));
  });
  await panel.getByText('No active connections.', { exact: true }).waitFor();
  assert.equal(await panel.getByText('2001:db8::30', { exact: true }).count(), 0, 'disconnection removes the address without waiting for polling');
  await page.evaluate(() => window.addressFixture.stop());
  assert.deepEqual(externalRequests, [], 'looking at connection addresses makes no external request');
  assert.deepEqual(errors, []);
  await context.close();
  console.log('PASS: You connection address states, source context, keyboard copy, stale clearing and 1280/320 px layout (offline browser statistics).');
} finally {
  await browser?.close();
  await server.close();
}
