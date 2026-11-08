// Direct links between browsers, end to end in real browsers:
//
//   the pi (hoi-node): a public node on this machine, with nothing configured
//   mira (browser): opens the pi's node link, starts c/balcony, which the pi takes on
//   kim (browser): opens the same link and joins c/balcony from mira's invite
//
// Both turned on "Direct links with members' browsers". The pi lists each to the other and passes
// one offer and one answer between them; its WebRTC port answers their STUN. Their browsers open
// a data channel to each other. Then the pi stops, and mira's next post still reaches kim: it went
// browser to browser.
//
// Needs `pnpm core:wasm` and `cargo build -p hoi-node` (in core/).
// Run: node scripts/e2e-direct.mjs (CHROMIUM_PATH picks a browser, HOI_E2E_BROWSER another engine).
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { createSocket } from 'node:dgram';
import { once } from 'node:events';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { BROWSER, launch } from './e2e-browser.mjs';
import { createServer as createViteServer } from '../apps/web/node_modules/vite/dist/node/index.js';
import { consentAccepted } from './browser-vault.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const temp = await mkdtemp(join(tmpdir(), 'hoi-direct-'));
const listen = (server) => new Promise((done, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', done); });
const probe = createServer();
await listen(probe);
const appPort = probe.address().port;
await new Promise((done) => probe.close(done));
const origin = `http://127.0.0.1:${appPort}`;
const udp = createSocket('udp4');
await new Promise((done, reject) => { udp.once('error', reject); udp.bind(0, '127.0.0.1', done); });
const rtcPort = udp.address().port;
await new Promise((done) => udp.close(done));

let pi;
let piOutput = '';
let vite;
let browser;
const pageErrors = [];

async function until(check, message, timeout = 30_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await check()) return;
    await new Promise((done) => setTimeout(done, 200));
  }
  throw new Error(message);
}

/** The node links a hoi-node printed, each with what it holds (the line under it): the addresses
 * themselves are packed into the link's code. */
const linksIn = (out) => [...out.matchAll(/https:\/\/\S+#\/node\?(\S+)\s+it holds (\d+) WebRTC routes? and (\d+) native routes?(, and the operator's notice, signed)?/g)]
  .map((m) => ({ query: m[1], rtc: Number(m[2]), native: Number(m[3]), notice: !!m[4] }));

async function startPi() {
  const command = join(root, 'core/target/debug', process.platform === 'win32' ? 'hoi-node.exe' : 'hoi-node');
  pi = spawn(command, ['--data', temp, '--listen', '127.0.0.1:0', '--name', 'the pi', '--rtc-port', String(rtcPort), '--rtc-advertise', `127.0.0.1:${rtcPort}`, '--no-port-mapping', '--no-dht'],
    { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
  const link = await new Promise((done, reject) => {
    const timeout = setTimeout(() => reject(new Error(`The pi did not start: ${piOutput}`)), 30_000);
    pi.once('exit', (code) => { clearTimeout(timeout); reject(new Error(`The pi exited ${code}: ${piOutput}`)); });
    const read = (bytes) => {
      piOutput = (piOutput + bytes).slice(-64_000);
      const found = linksIn(piOutput).filter((l) => l.rtc).pop()?.query;
      if (found) { clearTimeout(timeout); done(found); }
    };
    pi.stdout.on('data', read);
    pi.stderr.on('data', read);
  });
  return link;
}

/** A browser with a key, direct links on, connected to the pi through its node link. */
async function person(name, nodeLink) {
  const context = await browser.newContext();
  await consentAccepted(context);
  const page = await context.newPage();
  page.setDefaultTimeout(45_000);
  page.on('pageerror', (error) => pageErrors.push(`${name}: ${error.message}`));
  await page.goto(origin);
  await page.evaluate(async ({ name, nodeLink }) => {
    const { app } = await import('/src/lib/app.svelte.ts');
    const deadline = Date.now() + 30_000;
    while (app.phase === 'booting' && Date.now() < deadline) await new Promise((done) => setTimeout(done, 50));
    if (app.phase === 'locked') await app.unlock('direct links browser test passphrase');
    if (app.phase !== 'ready') throw new Error(`App did not become ready: ${app.bootError}`);
    await app.identityChanged(await app.core.createIdentity(name));
    await app.setSettings({ motion: 'reduced', postingNoticeSeen: true, directLinks: true });
    await app.core.openNodeLink(`https://hoi.sh/app/#/node?${nodeLink}`);
  }, { name, nodeLink });
  await until(() => page.evaluate(async () => {
    const net = await (await import('/src/lib/app.svelte.ts')).app.core.net();
    return net.connections?.some((c) => c.connected && c.kind === 'public') === true;
  }), `${name} did not reach the pi: ${piOutput}`, 60_000);
  return page;
}

const directLinks = (page) => page.evaluate(async () => {
  const net = await (await import('/src/lib/app.svelte.ts')).app.core.net();
  return (net.connections ?? []).filter((c) => c.connected && c.direct);
});

try {
  const nodeLink = await startPi();
  console.log('✓ the pi is up as a public node, nothing configured');
  process.env.VITE_HOI_BACKEND = 'wasm';
  process.env.VITE_HOI_REGISTRY_URL = '';
  process.env.HOI_SANDBOX = '0';
  vite = await createViteServer({ root: join(root, 'apps/web'), server: { host: '127.0.0.1', port: appPort, strictPort: true, hmr: false }, logLevel: 'error' });
  await vite.listen();
  // Chromium hides a page's own addresses behind mDNS names unless told otherwise; Firefox's
  // launcher prefs do the same (e2e-browser.mjs).
  browser = await launch(BROWSER === 'chromium' ? { args: ['--disable-features=WebRtcHideLocalIpsWithMdns'] } : {});

  const mira = await person('mira', nodeLink);
  const community = await mira.evaluate(async () => {
    const { app } = await import('/src/lib/app.svelte.ts');
    const c = await app.core.createCommunity({ name: 'balcony', description: 'What grows on one.', color: 'mint', limits: { powBits: 12 } });
    await app.refresh();
    return c.id;
  });
  // her browser asks the pi to carry what she founded
  await until(() => mira.evaluate(async (id) => {
    const net = await (await import('/src/lib/app.svelte.ts')).app.core.net();
    return net.connections?.some((c) => c.connected && c.kind === 'public' && c.communities?.includes(id)) === true;
  }, community), `the pi did not take c/balcony on: ${piOutput}`, 60_000);
  const invite = await mira.evaluate(async (id) => (await (await import('/src/lib/app.svelte.ts')).app.core.invite(id)).web, community);
  console.log('✓ mira started c/balcony in her browser and the pi carries it');

  const kim = await person('kim', nodeLink);
  await kim.evaluate(async (invite) => {
    const { app } = await import('/src/lib/app.svelte.ts');
    await app.core.join(invite);
    await app.refresh();
  }, invite);
  const first = await mira.evaluate(async (id) => (await (await import('/src/lib/app.svelte.ts')).app.core.createPost({ community: id, title: 'Through the pi', text: 'Before the direct link.' })).id, community);
  await until(() => kim.evaluate(async (p) => !!(await (await import('/src/lib/app.svelte.ts')).app.core.post(p).catch(() => null)), first), "kim didn't get mira's first post through the pi", 60_000);
  console.log('✓ kim joined c/balcony and reads mira through the pi');

  // the pi introduces them; one offers, one answers; the channel opens between the browsers
  await until(async () => (await directLinks(mira)).length === 1 && (await directLinks(kim)).length === 1,
    'no direct link opened between the two browsers', 60_000);
  const [m, k] = [(await directLinks(mira))[0], (await directLinks(kim))[0]];
  assert.equal(m.transport, 'webrtc');
  assert.equal(m.kind, 'browser');
  assert.notEqual(m.incoming, k.incoming, 'one side offered, the other answered');
  await until(async () => (await directLinks(kim))[0]?.communities?.includes(community), 'the direct link does not sync c/balcony', 30_000);
  console.log(`✓ mira's and kim's browsers opened a direct link (${m.incoming ? 'kim' : 'mira'} offered), introduced by the pi`);

  // the pi goes away; the direct link stays, and carries mira's next post to kim
  const stopped = once(pi, 'exit');
  pi.kill();
  await stopped;
  // With the node gone, ICE consent can take 30 seconds to expire (RFC 7675).
  // Leave time for the browser to report the failure and update the app's network state.
  await until(async () => !(await mira.evaluate(async () => {
    const net = await (await import('/src/lib/app.svelte.ts')).app.core.net();
    return net.connections?.some((c) => c.connected && c.kind === 'public');
  })), 'mira still sees the pi', 60_000);
  assert.equal((await directLinks(mira)).length, 1, 'the direct link outlives the pi');
  const second = await mira.evaluate(async (id) => (await (await import('/src/lib/app.svelte.ts')).app.core.createPost({ community: id, title: 'Straight across', text: 'The pi is off.' })).id, community);
  await until(() => kim.evaluate(async (p) => !!(await (await import('/src/lib/app.svelte.ts')).app.core.post(p).catch(() => null)), second), "mira's post didn't reach kim with the pi off", 30_000);
  const got = await kim.evaluate(async (p) => (await (await import('/src/lib/app.svelte.ts')).app.core.post(p)), second);
  assert.equal(got.title, 'Straight across');
  assert.equal(got.author.name, 'mira');
  assert.deepEqual(pageErrors, []);
  console.log(`PASS (${BROWSER}): the pi passed one offer and one answer, then mira's post went from her browser to kim's with the pi off.`);
} catch (error) {
  for (const context of browser?.contexts() ?? []) for (const page of context.pages()) {
    console.error('Network:', JSON.stringify(await page.evaluate(async () => (await import('/src/lib/app.svelte.ts')).app.core.net()).catch(() => null)));
  }
  console.error('The pi said:', piOutput.slice(-4000));
  throw error;
} finally {
  await browser?.close();
  await vite?.close();
  if (pi && pi.exitCode === null && pi.signalCode === null) { const stopped = once(pi, 'exit'); pi.kill(); await stopped; }
  await rm(temp, { recursive: true, force: true });
}
