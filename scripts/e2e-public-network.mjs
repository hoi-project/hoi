// Real independent browser keys -> public registry -> WebRTC carrier, founder offline.
// Nothing is configured by hand: the registry starts with no public nodes, the carrier is
// started with --index and announces itself, the founder's browser never pairs with it, and
// the joining browser gets no invite, pairing credential or local WebSocket.
// Requires pnpm core:wasm and cargo build -p hoi-node (in core/).
// Run: node scripts/e2e-public-network.mjs (CHROMIUM_PATH may select Chromium).
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { createSocket } from 'node:dgram';
import { once } from 'node:events';
import { mkdtemp, readdir, unlink, rmdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { DatabaseSync } from 'node:sqlite';
import { launch } from './e2e-browser.mjs';
import { createServer as createViteServer } from '../apps/web/node_modules/vite/dist/node/index.js';
import { createRegistryServer } from '../registry/server.mjs';
import { consentAccepted } from './browser-vault.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const temp = await mkdtemp(join(tmpdir(), 'hoi-public-network-'));
const listen = (server) => new Promise((done, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', done); });
const closeServer = (server) => new Promise((done) => {
  if (!server?.listening) return done();
  server.closeAllConnections(); server.close(done);
});
const probe = createServer();
await listen(probe);
const appPort = probe.address().port;
await closeServer(probe);
const origin = `http://127.0.0.1:${appPort}`;
const udp = createSocket('udp4');
await new Promise((done, reject) => { udp.once('error', reject); udp.bind(0, '127.0.0.1', done); });
const rtcPort = udp.address().port;
await new Promise((done) => udp.close(done));
let carrier;
let carrierOutput = '';
let registry;
let vite;
let browser;
const pageErrors = [];

async function startCarrier(registryUrl) {
  const command = join(root, 'core/target/debug', process.platform === 'win32' ? 'hoi-node.exe' : 'hoi-node');
  carrier = spawn(command, ['--data', temp, '--listen', '127.0.0.1:0', '--name', 'public carrier',
    '--rtc-port', String(rtcPort), '--rtc-advertise', `127.0.0.1:${rtcPort}`, '--relaying', '--index', registryUrl, '--carry-limit', '8', '--no-port-mapping'],
  { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
  await new Promise((done, reject) => {
    const timeout = setTimeout(() => reject(new Error(`Carrier did not start: ${carrierOutput}`)), 30_000);
    const finish = (error) => { clearTimeout(timeout); error ? reject(error) : done(); };
    carrier.once('error', finish);
    carrier.once('exit', (code) => finish(new Error(`Carrier exited ${code}: ${carrierOutput}`)));
    const read = (bytes) => {
      carrierOutput = (carrierOutput + bytes).slice(-64_000);
      if (carrierOutput.includes(`public index ${registryUrl}: announcing`)) finish();
    };
    carrier.stdout.on('data', read); carrier.stderr.on('data', read);
  });
  const routes = [...carrierOutput.matchAll(/rtc:[A-Za-z0-9_-]{43}@[^\s,]+/g)].map(match => match[0]);
  const route = routes.find(value => value.endsWith(`@127.0.0.1:${rtcPort}`));
  assert.ok(route, 'the node must advertise its real RTC certificate fingerprint');
  return route;
}

async function seed(context, name) {
  const page = await context.newPage();
  page.setDefaultTimeout(45_000);
  page.on('pageerror', error => pageErrors.push(`${name}: ${error.message}`));
  await page.goto(origin);
  await page.waitForFunction(async () => (await import('/src/lib/app.svelte.ts')).app.phase !== 'booting', null, { timeout: 30_000 });
  await page.evaluate(async name => {
    const { app } = await import('/src/lib/app.svelte.ts');
    const deadline = Date.now() + 30_000;
    while (app.phase === 'booting' && Date.now() < deadline) await new Promise(done => setTimeout(done, 50));
    if (app.phase === 'locked') await app.unlock('public network browser test passphrase');
    if (app.phase !== 'ready') throw new Error(`App did not become ready: ${app.bootError}`);
    await app.identityChanged(await app.core.createIdentity(name));
    // The once-only notice before a first post through others' nodes is e2e-node-link's to test.
    await app.setSettings({ motion: 'reduced', postingNoticeSeen: true });
  }, name);
  return page;
}

async function until(check, message, timeout = 30_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await check()) return;
    await new Promise(done => setTimeout(done, 100));
  }
  throw new Error(message);
}

try {
  // The registry knows no public node until one announces itself. Everything runs on this
  // machine, so the carrier announces loopback routes: the registry takes them only because this
  // test says so (a production registry, and the Worker, never do).
  registry = createRegistryServer({ filename: join(temp, 'registry.db'), origins: [origin], publicNodes: [], appUrl: `${origin}/`, allowPrivateRoutesForTests: true });
  await listen(registry);
  const registryUrl = `http://127.0.0.1:${registry.address().port}`;
  assert.deepEqual((await (await fetch(`${registryUrl}/v1/meta`)).json()).publicNodes, []);
  const route = await startCarrier(registryUrl);
  await until(async () => (await (await fetch(`${registryUrl}/v1/meta`)).json()).publicNodes.includes(route),
    `The carrier did not announce ${route} to the registry: ${carrierOutput}`, 30_000);
  assert.match(carrierOutput, /public index: announced \d+ routes? and 0 communities/);
  console.log('The carrier announced its own WebRTC route to the registry; nobody configured HOI_PUBLIC_NODES.');

  process.env.VITE_HOI_BACKEND = 'wasm';
  process.env.VITE_HOI_REGISTRY_URL = registryUrl;
  process.env.HOI_SANDBOX = '0';
  vite = await createViteServer({ root: join(root, 'apps/web'), server: { host: '127.0.0.1', port: appPort, strictPort: true, hmr: false }, logLevel: 'error' });
  await vite.listen();
  browser = await launch();
  const founder = await browser.newContext();
  await consentAccepted(founder);
  const newcomer = await browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
  await consentAccepted(newcomer);
  const robin = await seed(founder, 'Robin');
  const lena = await seed(newcomer, 'Lena');
  const roots = await Promise.all([robin, lena].map(page => page.evaluate(async () => (await import('/src/lib/app.svelte.ts')).app.me.rootKey)));
  assert.notEqual(roots[0], roots[1]);
  const sockets = [];
  robin.on('websocket', socket => sockets.push(socket.url()));
  lena.on('websocket', socket => sockets.push(socket.url()));

  // Robin makes a public community in a browser and reserves its name. The app's own
  // discovery then connects to the announced carrier; the carrier checks the registry and
  // takes the community from Robin's browser. No pairing link changes hands.
  const created = await robin.evaluate(async () => {
    const { app } = await import('/src/lib/app.svelte.ts');
    const { publishName } = await import('/src/lib/registry.ts');
    const community = await app.core.createCommunity({ name: 'public-network-test', description: 'Offline author delivery', color: 'mint', limits: { powBits: 12 } });
    const listing = await publishName(await app.core.communityPublication(community.id));
    await app.refresh();
    return { community: community.id, listing };
  });
  assert.equal(created.listing.indexed, true);
  // Either way is the carrier's own choice: its index confirms the key, or the owner's browser
  // asks it to carry the community, whichever comes first.
  const carries = () => robin.evaluate(async id => {
    const net = await (await import('/src/lib/app.svelte.ts')).app.core.net();
    return net.connections.some(c => c.keeps && c.kind === 'public' && c.communities.includes(id));
  }, created.community);
  await until(async () => carrierOutput.includes('public index: now carrying c/public-network-test') || await carries(),
    `The carrier did not take the published community from the founder's browser: ${carrierOutput}`, 60_000);
  await robin.waitForFunction(async id => {
    const net = await (await import('/src/lib/app.svelte.ts')).app.core.net();
    return net.communities?.some(item => item.community === id && item.connectedPeers > 0) === true;
  }, created.community, { timeout: 45_000 });
  const post = await robin.evaluate(async id => {
    const { app } = await import('/src/lib/app.svelte.ts');
    await app.requirePostingConnection(id);
    const post = await app.core.createPost({ community: id, title: 'Robin is offline', text: 'Lena receives this from the carrier over WebRTC.' });
    return post.id;
  }, created.community);
  const storage = new DatabaseSync(join(temp, 'hoi.db'), { readOnly: true });
  try {
    const hasPost = storage.prepare('SELECT id FROM records WHERE id = ?');
    await until(() => !!hasPost.get(Buffer.from(post, 'hex')), 'Carrier did not durably save Robin’s signed post.');
  } finally { storage.close(); }
  // The carrier re-announces with the community listed; its routes then lead the per-community answer.
  await until(async () => {
    const nodes = (await (await fetch(`${registryUrl}/v2/nodes?community=${created.community}`)).json()).publicNodes;
    return nodes.includes(route) && nodes[0].startsWith(route.split('@')[0]);
  }, 'The carrier did not announce that it carries the community.', 30_000);
  await founder.close();
  assert.ok(robin.isClosed(), 'founder must be offline before the newcomer joins');
  console.log('Robin published a public community and post from a browser; the carrier took and saved them; Robin is offline.');

  await lena.evaluate(() => { location.hash = '#/registry'; });
  const entry = lena.locator('.registry-card').filter({ hasText: 'c/public-network-test' });
  await entry.getByRole('link', { name: 'Review and join →', exact: true }).click();
  const target = new URLSearchParams(new URL(lena.url()).hash.split('?')[1]);
  assert.equal(target.get('c'), created.community);
  assert.deepEqual([...target.keys()].sort(), ['c', 'name', 'p'], 'public link contains no pairing token, genesis or invite');
  await lena.getByText('those carrying this community first').waitFor();
  await lena.getByRole('button', { name: 'Join public community', exact: true }).click();
  await lena.getByRole('dialog', { name: 'Turn on your VPN before joining' }).getByRole('button', { name: 'Continue joining' }).click();
  await lena.getByRole('link', { name: 'Open community', exact: true }).waitFor({ timeout: 75_000 });
  assert.equal(await lena.evaluate(async id => (await (await import('/src/lib/app.svelte.ts')).app.core.publicJoinStatus(id)).status, created.community), 'joined');
  await lena.getByRole('link', { name: 'Open community', exact: true }).click();
  await lena.getByText('Robin is offline', { exact: true }).first().waitFor({ timeout: 45_000 });
  const received = await lena.evaluate(async post => {
    const { app } = await import('/src/lib/app.svelte.ts');
    return { net: await app.core.net(), traffic: await app.core.traffic(), post: await app.core.post(post) };
  }, post);
  assert.ok(!received.net.homeNode, 'Lena must not be paired with the node');
  assert.ok(received.traffic.connections.some(connection => connection.role === 'member' && connection.transport === 'webrtc' && connection.recordsIn > 0));
  assert.ok(received.traffic.connections.every(connection => connection.transport !== 'websocket'), 'no sync may use a local WebSocket');
  assert.equal(sockets.length, 0, 'neither browser opens a WebSocket');
  assert.equal(received.post.title, 'Robin is offline');
  assert.equal(received.post.author.name, 'Robin');
  await lena.evaluate(() => { location.hash = '#/you'; });
  await lena.getByText('encrypted · WebRTC, DTLS', { exact: true }).first().waitFor();
  assert.deepEqual(pageErrors, []);
  assert.doesNotMatch(carrierOutput, /127\.0\.0\.1:\d+ (connected|joined|dialed)/, 'the carrier prints its own status, never a peer');
  assert.doesNotMatch(carrierOutput, /hoi:\/\/pair\?|#\/pair\?/, 'the pairing link stays out of the output unless asked for (--show-pairing)');
  console.log('PASS: the carrier announced itself, took Robin’s community from a browser without pairing, and served Lena over real WebRTC while Robin was offline; no invite, pairing, HOI_PUBLIC_NODES or local WebSocket.');
} catch (error) {
  for (const context of browser?.contexts() ?? []) for (const page of context.pages()) {
    console.error('Browser state:', page.url(), await page.locator('.public-card, [role="alert"]').allTextContents());
    console.error('Network:', await page.evaluate(async () => (await import('/src/lib/app.svelte.ts')).app.core.net()).catch(() => null));
  }
  console.error('Carrier output:', carrierOutput);
  throw error;
} finally {
  await browser?.close();
  await vite?.close();
  await closeServer(registry);
  if (carrier && carrier.exitCode === null && carrier.signalCode === null) { const stopped = once(carrier, 'exit'); carrier.kill(); await stopped; }
  // Only remove this test's newly-created flat temporary directory after the node exits.
  for (const file of await readdir(temp, { withFileTypes: true })) {
    if (!file.isFile()) throw new Error(`Unexpected directory in test cleanup: ${file.name}`);
    await unlink(join(temp, file.name));
  }
  await rmdir(temp);
}
