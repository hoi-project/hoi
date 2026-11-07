// Real browser worker -> signed community -> versioned HTTP registry -> durable public join.
// No mocked keys or signatures. Peer delivery/admission is covered by core public-join tests.
// Run after pnpm core:wasm: node scripts/e2e-registry-browser.mjs
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtemp, rmdir, unlink } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { createServer as createViteServer } from '../apps/web/node_modules/vite/dist/node/index.js';
import { createRegistryServer } from '../registry/server.mjs';
import { unlockBrowser, consentAccepted } from './browser-vault.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const temp = await mkdtemp(join(tmpdir(), 'hoi-registry-browser-'));
const filename = join(temp, 'registry.db');
const listen = (server, port = 0) => new Promise((resolveListen, reject) => {
  server.once('error', reject);
  server.listen(port, '127.0.0.1', resolveListen);
});
const close = (server) => new Promise((done) => {
  if (!server?.listening) return done();
  server.closeAllConnections();
  server.close(done);
});
const probe = createServer();
await listen(probe);
const appPort = probe.address().port;
await close(probe);
const origin = `http://127.0.0.1:${appPort}`;
let registry;
let vite;
let browser;
const pageErrors = [];

async function startRegistry(port = 0) {
  registry = createRegistryServer({ filename, origins: [origin] });
  await listen(registry, port);
  return registry.address().port;
}

async function seed(page, name) {
  page.on('pageerror', (e) => pageErrors.push(e.message));
  await page.goto(origin);
  await unlockReady(page);
  await page.evaluate(async (name) => {
    const { app } = await import('/src/lib/app.svelte.ts');
    const deadline = Date.now() + 30_000;
    while (app.phase === 'booting' && Date.now() < deadline) await new Promise((done) => setTimeout(done, 50));
    if (app.phase !== 'ready') throw new Error(`App did not become ready: ${app.bootError}`);
    await app.identityChanged(await app.core.createIdentity(name));
    await app.setSettings({ motion: 'reduced' });
  }, name);
}

async function unlockReady(page) {
  await unlockBrowser(page);
  await page.waitForFunction(async () => (await import('/src/lib/app.svelte.ts')).app.phase !== 'booting', null, { timeout: 30_000 });
  await page.evaluate(async () => {
    const { app } = await import('/src/lib/app.svelte.ts');
    if (app.phase !== 'ready') throw new Error(`App did not become ready: ${app.phase}; vault ${app.vaultState}; ${app.bootError}`);
  });
}

async function publicJoinStatus(page, community) {
  return page.evaluate(async (community) => (await import('/src/lib/app.svelte.ts')).app.core.publicJoinStatus(community), community);
}

async function reloadReady(page) {
  await page.reload();
  await unlockReady(page);
  await page.getByRole('heading', { name: 'Join a public community', exact: true }).waitFor();
  await page.waitForFunction(async () => (await import('/src/lib/app.svelte.ts')).app.phase === 'ready', null, { timeout: 30_000 });
}

try {
  const registryPort = await startRegistry();
  const registryUrl = `http://127.0.0.1:${registryPort}`;
  process.env.VITE_HOI_BACKEND = 'wasm';
  process.env.VITE_HOI_REGISTRY_URL = registryUrl;
  process.env.HOI_SANDBOX = '0';
  vite = await createViteServer({ root: join(root, 'apps/web'), server: { host: '127.0.0.1', port: appPort, strictPort: true, hmr: false }, logLevel: 'error' });
  await vite.listen();
  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
  const a = await browser.newContext();
  await consentAccepted(a);
  const b = await browser.newContext();
  await consentAccepted(b);
  a.setDefaultTimeout(30_000);
  b.setDefaultTimeout(30_000);
  const robin = await a.newPage();
  const lena = await b.newPage();
  await seed(robin, 'Robin');
  await seed(lena, 'Lena');
  await lena.getByRole('status', { name: 'Release status' }).getByText(/Protocol changes may cause data loss/).waitFor();
  const roots = await Promise.all([robin, lena].map((page) => page.evaluate(async () => (await import('/src/lib/app.svelte.ts')).app.me.rootKey)));
  assert.notEqual(roots[0], roots[1]);
  console.log('Real WASM identities ready');

  await robin.evaluate(() => { location.hash = '#/new'; });
  await robin.locator('#n-name').fill('registry-browser-test');
  await robin.getByRole('button', { name: 'Make c/registry-browser-test', exact: true }).click();
  await robin.getByText('Registered public name · c/registry-browser-test', { exact: true }).waitFor();
  const community = await robin.evaluate(() => location.hash.split('/')[2]);
  assert.match(community, /^[a-f0-9]{64}$/);
  const listing = await (await fetch(`${registryUrl}/v2/communities?name=registry-browser-test`)).json();
  assert.deepEqual(listing, { name: 'registry-browser-test', community, protocolVersion: '0.3', indexVersion: 1, compatible: true, indexed: true,
    proof: { version: 2, signature: listing.proof.signature, protocolVersion: '0.3', indexVersion: 1 } });
  assert.match(listing.proof.signature, /^[0-9a-f]{128}$/);
  await robin.getByText('People can find this community by name.', { exact: false }).waitFor();
  await robin.keyboard.press('Escape');
  console.log('Public creation confirmed by UI and registry');

  await lena.evaluate(() => { location.hash = '#/new'; });
  await lena.locator('#n-name').fill('registry-browser-test');
  await lena.getByRole('button', { name: 'Make c/registry-browser-test', exact: true }).click();
  await lena.getByRole('alert').filter({ hasText: 'already registered' }).waitFor();
  assert.equal(await lena.evaluate(async () => (await (await import('/src/lib/app.svelte.ts')).app.core.communities()).length), 0);
  // A distinct local community can exist, but the signed registry claim cannot steal the name.
  const other = await lena.evaluate(async () => {
    const { app } = await import('/src/lib/app.svelte.ts');
    const c = await app.core.createCommunity({ name: 'registry-browser-test', description: '', color: 'mint', limits: { powBits: 12 } });
    const { publishName } = await import('/src/lib/registry.ts');
    try { await publishName(await app.core.communityPublication(c.id)); }
    catch (error) { await app.refresh(); return { id: c.id, conflict: error.name, message: error.message }; }
    throw new Error('A second signed claim unexpectedly succeeded');
  });
  assert.notEqual(other.id, community);
  assert.match(other.message, /already registered to another community/);
  await lena.evaluate((id) => { location.hash = `#/c/${id}`; }, other.id);
  await lena.getByText('Name registered to another community', { exact: true }).waitFor();
  console.log('Conflicting public creation blocked; distinct signed claim rejected');

  // A crafted link cannot reuse a registered name with another key.
  const forgedKey = 'f'.repeat(64);
  await lena.evaluate((id) => { location.hash = `#/public?c=${id}&name=registry-browser-test&p=0.3`; }, forgedKey);
  await lena.getByRole('button', { name: 'Join public community', exact: true }).click();
  await lena.getByRole('dialog', { name: 'Turn on your VPN before joining' }).getByRole('button', { name: 'Continue joining' }).click();
  await lena.getByRole('alert').filter({ hasText: 'does not match the verified community name' }).waitFor();
  assert.equal((await publicJoinStatus(lena, forgedKey)).status, 'unknown', 'a mismatched named link must not start core synchronization');

  // The registry's key can start a join without an invitation or a local connection.
  // With no carrying node configured it must remain visibly pending, never pretend joined.
  await lena.evaluate(() => { location.hash = '#/registry'; });
  await lena.getByRole('heading', { name: 'Community registry', exact: true }).waitFor();
  const registered = lena.locator('.registry-card').filter({ hasText: 'c/registry-browser-test' });
  await registered.getByRole('link', { name: 'Review and join →', exact: true }).click();
  await lena.getByRole('heading', { name: 'Join a public community', exact: true }).waitFor();
  const publicUrl = new URL(lena.url());
  assert.equal(new URLSearchParams(publicUrl.hash.split('?')[1]).get('c'), community);
  await lena.getByText(/No public peer is available from this registry yet/).waitFor();
  await lena.getByRole('button', { name: 'Join public community', exact: true }).click();
  await lena.getByRole('dialog', { name: 'Turn on your VPN before joining' }).getByRole('button', { name: 'Continue joining' }).click();
  await lena.getByText(/Waiting for a node with this community/).waitFor();
  assert.equal((await publicJoinStatus(lena, community)).status, 'pending');
  assert.equal(await lena.getByRole('link', { name: 'Open community', exact: true }).count(), 0);
  await reloadReady(lena);
  await lena.getByText(/The join request is saved on this device/).waitFor();
  assert.equal((await publicJoinStatus(lena, community)).status, 'pending', 'IndexedDB restores pending public joins');
  await lena.getByRole('button', { name: 'Cancel request', exact: true }).click();
  await lena.getByRole('button', { name: 'Join public community', exact: true }).waitFor();
  assert.equal((await publicJoinStatus(lena, community)).status, 'unknown');
  await reloadReady(lena);
  assert.equal((await publicJoinStatus(lena, community)).status, 'unknown', 'cancelled joins stay cancelled after reload');
  assert.equal(await lena.getByRole('button', { name: 'Cancel request', exact: true }).count(), 0);
  await lena.evaluate((community) => { location.hash = `#/public?c=${community}&name=registry-browser-test&p=0.2`; }, community);
  await lena.getByRole('alert').filter({ hasText: 'This link uses protocol 0.2.' }).waitFor();
  assert.equal(await lena.getByRole('button', { name: 'Join public community', exact: true }).count(), 0);
  await lena.getByRole('status', { name: 'Release status' }).getByText(/Protocol changes may cause data loss/).waitFor();
  console.log('Public directory route, no-node guidance, durable pending/cancel, incompatible protocol, and preview checked');

  await close(registry);
  await robin.evaluate(() => { location.hash = '#/'; });
  await robin.getByRole('heading', { name: 'Hey Robin.' }).waitFor();
  await robin.evaluate((id) => { location.hash = `#/c/${id}`; }, community);
  await robin.getByText('Local name · public reservation unconfirmed', { exact: true }).waitFor();
  await robin.locator('[aria-label="Community identity"]').getByText(/directory is unreachable/).waitFor();
  await robin.getByRole('button', { name: 'Register public name', exact: true }).click();
  await robin.locator('[aria-label="Community identity"]').getByText(/directory is unreachable/).waitFor();
  assert.equal(await robin.evaluate(async () => (await (await import('/src/lib/app.svelte.ts')).app.core.communities()).length), 1);
  await startRegistry(registryPort);
  await robin.getByRole('button', { name: 'Register public name', exact: true }).click();
  await robin.getByText('Registered public name · c/registry-browser-test', { exact: true }).waitFor();
  assert.equal(await robin.evaluate(() => location.hash.split('/')[2]), community);
  assert.deepEqual(await (await fetch(`${registryUrl}/v2/communities?name=registry-browser-test`)).json(), listing);
  assert.deepEqual(pageErrors, []);
  console.log('PASS: real browser v2 publication, conflict/outage guidance, same-key retry, public join route, durable pending/cancel, protocol compatibility, and preview');
} catch (error) {
  for (const context of browser?.contexts() ?? []) for (const page of context.pages()) {
    console.error('Browser state:', page.url(), await page.locator('body').innerText());
  }
  throw error;
} finally {
  await browser?.close();
  await vite?.close();
  await close(registry);
  for (const suffix of ['', '-wal', '-shm']) await unlink(filename + suffix).catch((e) => { if (e.code !== 'ENOENT') throw e; });
  await rmdir(temp);
}
