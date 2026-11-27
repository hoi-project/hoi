// Main-interface IP evidence and an explicitly requested pre-join check. Offline fixtures;
// the transport's real no-profile/no-community handshake is tested at its own boundary.
import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
import { resolve } from 'node:path';
import { chromium } from 'playwright';
import { startComponentServer } from './e2e-component-server.mjs';
import { buildWorldMap, encodeTable, parseRanges } from './geo-assets.mjs';

// Country data fixtures replace the bundled tables, so results do not depend on real data. XC and XY are
// ISO 3166-1 user-assigned codes without display names; the browser runs in UTC, which the fixture maps to XC.
const square = (code, lon, lat, size) => ({ type: 'Feature', properties: { ISO_A2_EH: code, LABEL_X: lon + size / 2, LABEL_Y: lat + size / 2 },
  geometry: { type: 'Polygon', coordinates: [[[lon, lat], [lon + size, lat], [lon + size, lat + size], [lon, lat + size], [lon, lat]]] } });
const GEO = {
  'ip-country-v4': { type: 'application/octet-stream', body: encodeTable(parseRanges('1.0.0.0,1.255.255.255,XC\n', 4), 4).bytes },
  'ip-country-v6': { type: 'application/octet-stream', body: encodeTable(parseRanges('2606::,2606:ffff:ffff:ffff:ffff:ffff:ffff:ffff,XY\n', 6), 6).bytes },
  'tz-country': { type: 'application/json', body: JSON.stringify({ UTC: 'XC' }) },
  'world-map': { type: 'application/json', body: JSON.stringify(buildWorldMap({ features: [square('XC', -80, 0, 40), square('XY', 50, 10, 30)] })) },
};

process.env.VITE_HOI_BACKEND = 'mock';
const route = 'wss://first.example.com/';
const { server, origin } = await startComponentServer('/connection-ip-test', `<!doctype html><html lang="en"><title>Connection checks</title>
  <div id="header"></div><main id="check" class="gutter"></main>
  <script type="module">
    import { mount } from 'svelte';
    import TopBar from '/src/lib/ui/TopBar.svelte';
    import ConnectionCheck from '/src/lib/ui/ConnectionCheck.svelte';
    import { app } from '/src/lib/app.svelte.ts';
    import { watchBrowserIpRoute } from '/src/lib/browser-ip.svelte.ts';
    import '@hoi/tokens/fonts.css'; import '@hoi/tokens/tokens.css'; import '/src/app.css';
    await app.start(); app.refresh = async () => {}; app.refreshWarmth = async () => {};
    app.net = { mode: 'local', label: 'offline', peers: 0, rtc: [], relaying: false, transport: {endpointId:'fixture', peers:['${route}'], relays:[]} };
    watchBrowserIpRoute();
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
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, reducedMotion: 'reduce', serviceWorkers: 'block', timezoneId: 'UTC' });
  const external = [], errors = [], lookups = [], geo = [];
  const provider = 'https://api64.ipify.org/';
  // Well-known public resolver addresses stand in for the fixture's HTTPS reply.
  // The browser never connects to a public resolver or provider.
  let result = '1.1.1.1';
  const publicV4 = result; // In the fixture table's XC range.
  await context.route('**/*', request => {
    const url = new URL(request.request().url());
    const data = url.origin === origin && request.request().resourceType() === 'fetch' && Object.keys(GEO).find(name => url.pathname.includes(`/${name}`));
    if (data) {
      geo.push(data);
      assert.equal(request.request().headers().cookie, undefined, 'Country data requests carry no cookies');
      return request.fulfill({ status: 200, contentType: GEO[data].type, body: GEO[data].body });
    }
    if (url.origin === origin) return request.continue();
    if (request.request().url() === provider) {
      lookups.push(request.request());
      return request.fulfill({
        status: 200, contentType: 'text/plain', headers: { 'Access-Control-Allow-Origin': '*' }, body: result,
      });
    }
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
  await header.getByRole('button', { name: 'IP not checked. Details', exact: true }).waitFor();
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
  // A documentation address is special-use: it has no country and loads no country data.
  const readout = header.getByRole('button', { name: 'Local route, 2001:db8::25. Details', exact: true });
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
  await check.getByRole('alert').filter({ hasText: 'Peer IP check failed.' }).waitFor();
  assert.equal(await page.evaluate(() => window.ipFixture.joinCalls), 0);

  assert.equal(lookups.length, 0, 'A failed peer check never contacts the public IP service');
  assert.deepEqual(geo, [], 'Nothing loads country data before a public check');
  const publicIp = check.getByRole('region', { name: 'Browser public IP' });
  await publicIp.getByText('api64.ipify.org', { exact: true }).waitFor();
  await publicIp.getByRole('button', { name: 'Check public IP', exact: true }).click();
  await publicIp.locator('output').filter({ hasText: result }).waitFor();
  const sameCountry = `XC, your time zone, ${result}. Details`;
  await header.getByRole('button', { name: sameCountry, exact: true }).waitFor();
  const card = publicIp.getByRole('region', { name: `Where ${result} is listed` });
  await card.getByText('Same country as your time zone', { exact: true }).waitFor();
  await card.getByRole('img', { name: /^World map\. XC highlighted/ }).waitFor();
  assert.match(await card.innerText(), /This does not show whether a VPN is on\./);
  assert.deepEqual([...geo].sort((a, b) => a.localeCompare(b)), ['ip-country-v4', 'tz-country', 'world-map'], 'The first check loads the IPv4 table, zones and map once');
  await header.getByText('VPN route: wg0', { exact: true }).waitFor();
  await page.evaluate(address => {
    const { app } = window.ipFixture;
    app.net = { ...app.net, connections: app.net.connections.map(c => ({ ...c, observedAddress: address })) };
  }, result);
  const browserOnly = header.getByRole('button', { name: sameCountry, exact: true });
  await browserOnly.waitFor(); // Two observations of the same address need only one headline.
  await page.evaluate(() => {
    const { app } = window.ipFixture;
    app.net = { ...app.net, vpn: undefined, connections: app.net.connections.map(c => ({ ...c, observedAddress: undefined })) };
  });
  await browserOnly.click();
  await details.getByText(/^No peer address reported\./).waitFor();
  assert.doesNotMatch(await header.innerText(), /VPN not verified|unavailable/i);
  await page.keyboard.press('Escape');
  assert.equal(await check.getByText(/This node sees you as/).count(), 0, 'An HTTPS exit IP never fills a peer observation');
  assert.equal(lookups.length, 1);
  assert.equal(lookups[0].method(), 'GET');
  const headers = await lookups[0].allHeaders();
  assert.equal(headers.referer, undefined);
  assert.equal(headers.cookie, undefined);
  await page.evaluate(() => window.dispatchEvent(new Event('offline')));
  await publicIp.locator('output').waitFor({ state: 'detached' });
  await header.getByRole('button', { name: 'IP not checked. Details', exact: true }).waitFor();
  await header.getByText('No peer address reported', { exact: true }).waitFor();
  await card.waitFor({ state: 'detached' });
  assert.equal(lookups.length, 1, 'A route change clears the sample without a fresh lookup');
  result = '2606:4700:4700::1111';
  await publicIp.getByRole('button', { name: 'Check public IP', exact: true }).click();
  await publicIp.locator('output').filter({ hasText: result }).waitFor();
  assert.equal(lookups.length, 2);
  const otherCountry = `XY, not your time zone, ${result}. Details`;
  await header.getByRole('button', { name: otherCountry, exact: true }).waitFor();
  assert.deepEqual([...geo].sort((a, b) => a.localeCompare(b)), ['ip-country-v4', 'ip-country-v6', 'tz-country', 'world-map'], 'IPv6 loads only its own table');

  // A pre-join result in another country than the browser's HTTPS exit says that the routes differ.
  await page.evaluate(address => { window.ipFixture.error = false; window.ipFixture.result = address; }, publicV4);
  await check.getByRole('button', { name: 'Check connection', exact: true }).click();
  await check.getByRole('region', { name: `Where ${publicV4} is listed` }).getByText('Routes differ', { exact: true }).waitFor();
  assert.equal(geo.length, 4, 'Tables already loaded are not fetched again');

  for (const width of [1280, 320]) {
    await page.setViewportSize({ width, height: 900 });
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `Main IP and check fit ${width} px`);
    assert.ok(await check.getByRole('button', { name: 'Check connection', exact: true }).first().isVisible());
    await mkdir(resolve(import.meta.dirname, '../test-results/ip-status'), { recursive: true });
    await page.screenshot({ path: resolve(import.meta.dirname, `../test-results/ip-status/${width}.png`) });
  }
  await page.evaluate(() => {
    const f = window.ipFixture;
    f.app.net = { ...f.app.net, connections: [], peers: 0, transport: { ...f.app.net.transport, peers: [] } };
  });
  await header.getByRole('button', { name: otherCountry, exact: true }).waitFor();
  await page.evaluate(() => window.dispatchEvent(new Event('offline')));
  await header.getByText('No active connection', { exact: true }).waitFor();
  await check.getByRole('button', { name: 'Check connection unavailable' }).waitFor();
  assert.equal(await check.getByText(/This node sees you as/).count(), 0);
  assert.equal(lookups.length, 2, 'Only the two explicit public IP clicks contacted the named provider');
  assert.deepEqual(external, [], 'No other external requests were made');
  assert.deepEqual(errors, []);
  await context.close();

  // With motion on, the card animates and then rests: no pin or dot is left mid-way.
  const moving = await browser.newContext({ viewport: { width: 1280, height: 900 }, reducedMotion: 'no-preference', serviceWorkers: 'block', timezoneId: 'UTC' });
  await moving.route('**/*', request => {
    const url = new URL(request.request().url());
    const data = url.origin === origin && request.request().resourceType() === 'fetch' && Object.keys(GEO).find(name => url.pathname.includes(`/${name}`));
    if (data) return request.fulfill({ status: 200, contentType: GEO[data].type, body: GEO[data].body });
    if (url.origin === origin) return request.continue();
    if (request.request().url() === provider) return request.fulfill({ status: 200, contentType: 'text/plain', headers: { 'Access-Control-Allow-Origin': '*' }, body: publicV4 });
    return request.abort();
  });
  const animated = await moving.newPage();
  animated.on('pageerror', error => errors.push(error.message));
  await animated.goto(`${origin}/connection-ip-test?me=Example`);
  await animated.getByRole('button', { name: 'Check public IP', exact: true }).click();
  await animated.getByRole('img', { name: /^World map\. XC highlighted/ }).waitFor();
  await animated.waitForTimeout(2500);
  const rest = await animated.evaluate(() => ({
    head: document.querySelector('.pin .head')?.getAttribute('style') ?? '',
    radii: [...document.querySelectorAll('.lit circle')].map(c => c.getAttribute('r')),
  }));
  assert.equal(rest.head.includes('transform') || rest.head.includes('opacity'), false, 'The pin rests in place');
  assert.ok(rest.radii.length > 0 && rest.radii.every(r => r === rest.radii[0] && Number(r) > 0), 'Every country dot is drawn');
  await animated.screenshot({ path: resolve(import.meta.dirname, '../test-results/ip-status/motion.png') });
  assert.deepEqual(errors, []);
  await moving.close();
  console.log('PASS: visible source IP and country, named intended-node consent, no automatic check or join, country data only after a check, stale cancellation, unknown/error, motion at rest and 320 px layout.');
} finally { await browser?.close(); await server.close(); }
