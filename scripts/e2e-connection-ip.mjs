// Main-interface IP evidence and an explicitly requested pre-join check. Offline fixtures;
// the transport's real no-profile/no-community handshake is tested at its own boundary.
import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
import { resolve } from 'node:path';
import { chromium } from 'playwright';
import { startComponentServer } from './e2e-component-server.mjs';

process.env.VITE_HOI_BACKEND = 'mock';
const route = 'wss://first.example.com/';
const { server, origin } = await startComponentServer('/connection-ip-test', `<!doctype html><html lang="en"><title>Connection checks</title>
  <div id="header"></div><main id="check" class="gutter"></main>
  <script type="module">
    import { mount } from 'svelte';
    import TopBar from '/src/lib/ui/TopBar.svelte';
    import ConnectionCheck from '/src/lib/ui/ConnectionCheck.svelte';
    import { app } from '/src/lib/app.svelte.ts';
    import '@hoi/tokens/fonts.css'; import '@hoi/tokens/tokens.css'; import '/src/app.css';
    await app.start(); app.refresh = async () => {}; app.refreshWarmth = async () => {};
    app.net = { mode: 'local', label: 'offline', peers: 0, rtc: [], relaying: false, transport: {endpointId:'fixture', peers:['${route}'], relays:[]} };
    window.ipFixture = { app, calls: [], result: '192.0.2.25', hold: false, joinCalls: 0 };
    for (const method of ['join', 'publicJoin', 'privateJoin']) app.core[method] = async () => { window.ipFixture.joinCalls++; throw new Error('The check must never join.'); };
    mount(TopBar, { target: document.querySelector('#header') });
    mount(ConnectionCheck, { target: document.querySelector('#check'), props: {
      get routes() { return app.net.transport.peers; },
      check: async (target, signal) => {
        const f = window.ipFixture; f.calls.push(target); f.signal = signal;
        if (f.hold) return await new Promise(resolve => { f.finish = resolve; });
        if (f.error) throw new Error('The intended node did not answer.');
        return { address: f.result, transport: 'websocket' };
      }
    } });
  </script></html>`);
let browser;
try {
  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, reducedMotion: 'reduce', serviceWorkers: 'block' });
  const external = [], errors = [];
  await context.route('**/*', request => {
    if (new URL(request.request().url()).origin === origin) return request.continue();
    external.push(request.request().url()); return request.abort();
  });
  const page = await context.newPage();
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(`${origin}/connection-ip-test?me=Example`);
  const check = page.getByRole('region', { name: 'Check connection before joining' });
  const header = page.getByLabel('Connection IP', { exact: true });
  await check.getByText(/reveals your source IP to that node/).waitFor();
  assert.equal(await page.evaluate(() => window.ipFixture.calls.length), 0, 'Opening a join screen never initiates an IP check');
  await header.getByText('No active connection', { exact: true }).waitFor();
  await header.getByRole('button', { name: 'Connection IP No active connection Details', exact: true }).waitFor();
  const profile = page.locator('a.me');
  const initials = (await profile.innerText()).trim();
  assert.ok(initials && (await profile.getAttribute('aria-label')).startsWith(initials), 'The profile link starts with its visible initials');
  await check.getByRole('button', { name: 'Check connection', exact: true }).click();
  await check.getByText('This node sees you as 192.0.2.25', { exact: true }).waitFor();
  assert.match(await check.innerText(), /Reported by first\.example\.com over websocket/);
  assert.deepEqual(await page.evaluate(() => window.ipFixture.calls), [route], 'Only the disclosed intended route is checked');
  assert.equal(await page.evaluate(() => window.ipFixture.joinCalls), 0, 'Checking never changes membership');
  await page.evaluate(() => window.dispatchEvent(new Event('offline')));
  await check.getByText('This node sees you as 192.0.2.25', { exact: true }).waitFor({ state: 'hidden' });
  assert.equal(await page.evaluate(() => window.ipFixture.signal.aborted), true, 'A network change invalidates the earlier check');

  await page.evaluate(() => {
    const { app } = window.ipFixture;
    app.net = { ...app.net, peers: 1, vpn: { interface: 'wg0', up: true, addresses: ['10.2.0.2'] }, connections: [
      { peer: 'first-node', role: 'peer', kind: 'public', route: 'direct', connected: true, transport: 'quic', encryption: 'encrypted', observedAddress: '2001:db8::25' }
    ], exposure: [{address:'198.51.100.90:7844', scope:'internet', fromRouter:true, told:['strangers']}] };
  });
  await header.getByText('2001:db8::25', { exact: true }).waitFor();
  assert.equal((await header.innerText()).includes('10.2.0.2'), false, 'A VPN interface IP is not shown as public egress');
  assert.equal((await header.innerText()).includes('198.51.100.90'), false, 'Listening hints are not shown as source evidence');
  const readout = header.getByRole('button', { name: 'Connection IP 2001:db8::25 Details', exact: true });
  await readout.focus();
  await page.keyboard.press('Enter');
  const details = page.getByRole('dialog', { name: 'Your connection IP' });
  await details.getByText('Node first-node', { exact: true }).waitFor();
  assert.match(await details.innerText(), /does not verify a VPN/);
  assert.ok(await details.evaluate(el => el.contains(document.activeElement)), 'Opening details moves keyboard focus inside');
  await page.keyboard.press('Escape');
  await details.waitFor({ state: 'hidden' });
  assert.ok(await readout.evaluate(el => el === document.activeElement), 'Escape returns to the opener');

  await page.evaluate(() => { window.ipFixture.hold = true; });
  await check.getByRole('button', { name: 'Check connection', exact: true }).click();
  await page.waitForFunction(() => !!window.ipFixture.finish);
  const checking = check.getByRole('button', { name: 'Checking connection…', exact: true });
  assert.ok(await checking.evaluate(el => el === document.activeElement), 'The ongoing check retains keyboard focus');
  assert.equal(await checking.getAttribute('aria-disabled'), 'true');
  await page.keyboard.press('Enter');
  assert.equal(await page.evaluate(() => window.ipFixture.calls.length), 2, 'A second activation cannot start a duplicate check');
  await page.evaluate(() => {
    const f = window.ipFixture;
    f.app.net = { ...f.app.net, transport: { ...f.app.net.transport, peers: ['wss://second.example.com/'] } };
  });
  await page.waitForFunction(() => window.ipFixture.signal.aborted);
  await page.evaluate(() => { window.ipFixture.finish({ address: '192.0.2.99', transport: 'websocket' }); window.ipFixture.hold = false; window.ipFixture.error = true; });
  assert.equal(await check.getByText(/This node sees you as/).count(), 0, 'A stale result cannot name a newly selected node');
  await check.getByRole('button', { name: 'Check connection', exact: true }).click();
  await check.getByRole('alert').filter({ hasText: 'Connection IP unavailable.' }).waitFor();
  assert.equal(await page.evaluate(() => window.ipFixture.joinCalls), 0);

  for (const width of [1280, 320]) {
    await page.setViewportSize({ width, height: 900 });
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `Main IP and check fit ${width} px`);
    assert.ok(await check.getByRole('button', { name: 'Check connection', exact: true }).isVisible());
    await mkdir(resolve(import.meta.dirname, '../test-results/ip-status'), { recursive: true });
    await page.screenshot({ path: resolve(import.meta.dirname, `../test-results/ip-status/${width}.png`) });
  }
  await page.evaluate(() => {
    const f = window.ipFixture;
    f.app.net = { ...f.app.net, connections: [], peers: 0, transport: { ...f.app.net.transport, peers: [] } };
  });
  await header.getByText('No active connection', { exact: true }).waitFor();
  await check.getByRole('button', { name: 'Check connection unavailable' }).waitFor();
  assert.equal(await check.getByText(/This node sees you as/).count(), 0);
  assert.deepEqual(external, [], 'The interface makes no generic external lookup');
  assert.deepEqual(errors, []);
  await context.close();
  console.log('PASS: visible source IP, named intended-node consent, no automatic check or join, stale cancellation, unknown/error and 320 px layout.');
} finally { await browser?.close(); await server.close(); }
