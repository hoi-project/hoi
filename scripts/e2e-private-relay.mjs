// Real browser/WASM private groups and IndexedDB, through an explicitly local opaque relay.
// Build the WASM core, start Wrangler locally, then:
// HOI_PRIVATE_RELAY_TEST_URL=http://127.0.0.1:8787 node scripts/e2e-private-relay.mjs
// Never point this test at a public relay: it creates encrypted test objects.
import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { createServer as createViteServer } from '../apps/web/node_modules/vite/dist/node/index.js';
import { isPaddedSize } from '../relay/protocol.mjs';
import { consentAccepted } from './browser-vault.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const target = new URL(process.env.HOI_PRIVATE_RELAY_TEST_URL || 'http://127.0.0.1:8787');
assert.ok(['127.0.0.1', 'localhost', '[::1]'].includes(target.hostname), 'Private relay tests only accept loopback destinations.');
assert.ok(['http:', 'https:'].includes(target.protocol) && !target.username && !target.password
  && target.pathname === '/' && !target.search && !target.hash, 'Provide only the local Worker origin.');
const relayOrigin = target.origin;
const output = resolve(root, 'test-results/private-relay');
await mkdir(output, { recursive: true });

const readyUntil = Date.now() + 60_000;
let ready = false;
while (Date.now() < readyUntil) {
  try {
    const response = await fetch(`${relayOrigin}/relay/v2/meta`, { redirect: 'error', signal: AbortSignal.timeout(2000) });
    ready = response.ok && (await response.json()).apiVersion === 2;
    if (ready) break;
  } catch { /* Wrangler may still be starting. */ }
  await new Promise(done => setTimeout(done, 500));
}
assert.equal(ready, true, 'The local encrypted relay did not become ready.');

// Synchronization is explicitly stepped so background polling cannot consume a commit
// during the tamper/removal assertions. All crypto and client persistence remain real.
process.env.VITE_HOI_BACKEND = 'wasm';
process.env.VITE_HOI_REGISTRY_URL = '';
process.env.HOI_SANDBOX = '0';
const servers = [];
async function startApp(relays) {
  process.env.VITE_HOI_RELAY_URLS = JSON.stringify(relays);
  const server = await createViteServer({
    root: resolve(root, 'apps/web'), logLevel: 'error',
    server: { host: '127.0.0.1', port: 0, hmr: false, proxy: {
      '/relay': { target: relayOrigin, changeOrigin: true, configure(proxy) {
        // This is a local same-origin test gateway, not an external browser CORS request.
        proxy.on('proxyReq', request => request.removeHeader('origin'));
      } },
    } },
  });
  servers.push(server);
  await server.listen();
  return `http://127.0.0.1:${server.httpServer.address().port}`;
}
const origin = await startApp([]);
const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
const errors = [];
const pages = [];

async function call(page, method, ...args) {
  return page.evaluate(async ({ method, args }) => {
    const { app } = await import('/src/lib/app.svelte.ts');
    if (method === 'createPost' && args[0].media) args[0].media = args[0].media.map(item => ({ ...item, data: new Uint8Array(item.data) }));
    const result = await app.core[method](...args);
    if (method === 'media') return { ...result, data: Array.from(result.data) };
    return result;
  }, { method, args });
}

async function boot(page) {
  // Run in the page's main world: its Svelte store is the one mounted by App.svelte.
  await page.evaluate(async () => {
    const { app } = await import('/src/lib/app.svelte.ts');
    const deadline = Date.now() + 30_000;
    while (app.phase === 'booting' && Date.now() < deadline) await new Promise(done => setTimeout(done, 50));
    if (app.phase === 'locked') await app.unlock('private relay browser test passphrase');
    if (app.phase !== 'ready') throw new Error(`App did not become ready: ${app.phase}: ${app.bootError}`);
  });
}

async function person(name, appOrigin = origin) {
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, reducedMotion: 'reduce' });
  await consentAccepted(context);
  const page = await context.newPage();
  pages.push(page);
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(appOrigin);
  await boot(page);
  await page.evaluate(async name => {
    const { app } = await import('/src/lib/app.svelte.ts');
    await app.identityChanged(await app.core.createIdentity(name));
    await app.setSettings({ motion: 'reduced' });
  }, name);
  return page;
}

async function sync(page, community, removed = false) {
  const status = await page.evaluate(async community => {
    const { app } = await import('/src/lib/app.svelte.ts');
    const { syncPrivateGroup } = await import('/src/lib/private-relay.ts');
    return syncPrivateGroup(app.core, community, ['/relay']);
  }, community);
  assert.equal(status.replicas, 1, JSON.stringify(status));
  if (removed) assert.match(status.error, /device was removed/i);
  else assert.equal(status.error, undefined, JSON.stringify(status));
  return status;
}

// Reads go straight to the Worker with signatures a member's core made for exactly that request.
async function signed(page, community, target) {
  const { room, reads: [read] } = await call(page, 'privateRead', community, [target]);
  const headers = { 'X-Hoi-Read-Time': String(read.time), 'X-Hoi-Read-Nonce': read.nonce, 'X-Hoi-Read-Signature': read.signature };
  return { room, url: `${relayOrigin}/relay/v2/rooms/${room}/${target}`, headers };
}

async function objects(page, community) {
  const result = [];
  let cursor = 0;
  for (let index = 0; index < 64; index++) {
    const { url, headers } = await signed(page, community, `objects?after=${cursor}`);
    const response = await fetch(url, { redirect: 'error', headers });
    assert.equal(response.status, 200);
    const listed = await response.json();
    for (const item of listed.objects) assert.deepEqual(Object.keys(item).sort(), ['id', 'sequence', 'size'], 'readers see no writer, grant, signature or expiry');
    result.push(...listed.objects);
    if (listed.nextCursor === undefined) return result;
    assert.ok(listed.nextCursor > cursor);
    cursor = listed.nextCursor;
  }
  throw new Error('Unexpectedly large private test history.');
}

async function ciphertext(page, community, id) {
  const { url, headers } = await signed(page, community, `objects/${id}`);
  const response = await fetch(url, { redirect: 'error', headers });
  assert.equal(response.status, 200);
  assert.equal(response.headers.get('Content-Type'), 'application/octet-stream');
  assert.equal(response.headers.get('X-Hoi-Author'), null);
  return Buffer.from(await response.arrayBuffer());
}

function pngPixel() {
  // Same 1x1 RGB fixture as the core's media validation tests, including valid CRCs.
  const parts = [Buffer.from([137, 80, 78, 71, 13, 10, 26, 10])];
  const crc32 = data => {
    let crc = 0xffffffff;
    for (const byte of data) { crc ^= byte; for (let bit = 0; bit < 8; bit++) crc = crc & 1 ? 0xedb88320 ^ (crc >>> 1) : crc >>> 1; }
    return (~crc) >>> 0;
  };
  const chunk = (name, bytes) => {
    const type = Buffer.from(name);
    const data = Buffer.from(bytes);
    const length = Buffer.alloc(4); length.writeUInt32BE(data.length);
    const crc = Buffer.alloc(4); crc.writeUInt32BE(crc32(Buffer.concat([type, data])));
    parts.push(length, type, data, crc);
  };
  chunk('IHDR', [0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]);
  chunk('IDAT', [0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0, 0, 3, 1, 1, 0]);
  chunk('IEND', []);
  return Buffer.concat(parts);
}

try {
  const founder = await person('Private founder');
  const member = await person('Private member');
  const outsider = await person('Unrelated device');
  const founderMe = await call(founder, 'me');
  const memberMe = await call(member, 'me');
  assert.notEqual(founderMe.rootKey, memberMe.rootKey);
  const community = await call(founder, 'createPrivateCommunity', { name: 'private-relay-test', description: 'Only invited devices can read this.', color: 'mint', limits: { powBits: 12 } });
  const initial = await call(founder, 'privateGroup', community.id);
  assert.equal(initial.active, true);
  assert.equal(initial.isOwner, true);
  const request = await call(member, 'privateJoinRequest');
  const invite = await call(founder, 'privateInvite', community.id, request);
  await assert.rejects(call(outsider, 'privateJoin', invite), 'an intercepted welcome cannot add an unrelated device');
  const joined = await call(member, 'privateJoin', invite);
  assert.equal(joined.id, community.id);
  await sync(founder, community.id);
  await sync(member, community.id);
  const group = await call(founder, 'privateGroup', community.id);
  assert.equal(group.members.some(item => item.device === memberMe.device), true);
  assert.equal(group.members.some(item => item.root === memberMe.rootKey), true);
  assert.equal((await call(member, 'privateGroup', community.id)).room, group.room);
  console.log('Real browser identities joined one private group using the encrypted welcome.');

  const title = 'Only the invited devices should read this message';
  const text = 'Operator-blind relay integration: the secret content stays inside MLS ciphertext.';
  const png = pngPixel();
  const post = await call(founder, 'createPost', { community: community.id, title, text, media: [{ data: [...png], alt: 'A private red pixel' }] });
  assert.ok(post.media?.length, 'a real PNG attachment must be included');
  const pending = await call(founder, 'privateOutbox', community.id);
  assert.ok(pending.length, 'a private post must queue encrypted relay objects');
  const corrupt = Buffer.from(pending.at(-1).body, 'base64url');
  corrupt[corrupt.length - 1] ^= 1;
  const beforeTamper = await call(member, 'privateGroup', community.id);
  await assert.rejects(call(member, 'privateReceive', community.id, corrupt.toString('base64url')), 'modified ciphertext must not be accepted');
  const afterTamper = await call(member, 'privateGroup', community.id);
  assert.equal(afterTamper.epoch, beforeTamper.epoch);
  assert.deepEqual(afterTamper.received, beforeTamper.received);
  await assert.rejects(call(outsider, 'privateReceive', community.id, pending.at(-1).body), 'an unrelated device cannot decrypt the group message');
  await sync(founder, community.id);
  await sync(member, community.id);
  const received = await call(member, 'post', post.id);
  assert.equal(received.title, title);
  assert.equal(received.text, text);
  const media = await call(member, 'media', post.media[0].hash);
  assert.equal(media.mime, 'image/png');
  assert.deepEqual(Buffer.from(media.data), Buffer.from((await call(founder, 'media', post.media[0].hash)).data));

  const stored = await objects(member, community.id);
  assert.ok(stored.length > 0);
  for (const item of stored) {
    const bytes = await ciphertext(member, community.id, item.id);
    for (const plaintext of [title, text, 'private-relay-test', 'A private red pixel']) assert.equal(bytes.includes(Buffer.from(plaintext)), false, 'relay body contains plaintext');
    assert.equal(bytes.includes(png), false, 'relay body contains raw attachment bytes');
    for (const key of [community.id, founderMe.device, memberMe.device]) assert.equal(bytes.includes(Buffer.from(key, 'hex')), false, 'the MLS header and member keys are sealed');
    assert.ok(isPaddedSize(bytes.length), `object size ${bytes.length} is a padded bucket`);
  }
  assert.doesNotMatch(JSON.stringify(stored), /Private founder|Private member|private-relay-test|Only the invited/);
  // A stranger who knows the room, or replays a member's read, learns nothing.
  const listing = await signed(founder, community.id, 'objects?after=0');
  assert.equal(listing.room, group.room);
  assert.notEqual(group.room, community.id, 'the room is not the community key');
  assert.equal((await fetch(listing.url, { redirect: 'error' })).status, 401, 'unsigned listing');
  assert.equal((await fetch(listing.url, { redirect: 'error', headers: listing.headers })).status, 200);
  assert.equal((await fetch(listing.url, { redirect: 'error', headers: listing.headers })).status, 401, 'replayed read');
  assert.equal((await fetch(`${relayOrigin}/relay/v1/rooms/${group.room}/objects`, { redirect: 'error' })).status, 410, 'protocol v1 is gone');
  await assert.rejects(call(outsider, 'privateRead', community.id, ['objects?after=0']), 'an unrelated device cannot sign reads');
  console.log('Encrypted text and PNG arrived through the relay; reads were signed by the room key, objects padded and sealed; tampering, strangers and replays were refused.');

  await member.reload();
  await boot(member);
  assert.equal((await call(member, 'post', post.id)).text, text, 'private records survive browser reload');
  assert.equal((await call(member, 'privateGroup', community.id)).room, group.room, 'MLS state survives browser reload');
  assert.deepEqual(Buffer.from((await call(member, 'media', post.media[0].hash)).data), Buffer.from(media.data), 'encrypted media was persisted locally after validation');

  const previousEpoch = (await call(founder, 'privateGroup', community.id)).epoch;
  const removed = await call(founder, 'privateRemove', community.id, memberMe.device);
  assert.ok(removed.epoch > previousEpoch, 'removing a member rotates the MLS epoch');
  assert.notEqual(removed.room, group.room, 'a new epoch has a new room');
  await sync(founder, community.id);
  await sync(member, community.id, true);
  const gone = await call(member, 'privateGroup', community.id);
  assert.equal(gone.active, false);
  assert.equal(gone.room, '', 'a removed device cannot derive the next room');
  await assert.rejects(call(member, 'privateRead', community.id, ['objects?after=0']), 'a removed device cannot sign reads');
  const later = await call(founder, 'createPost', { community: community.id, title: 'After removal', text: 'This uses the new membership epoch.' });
  const newObjects = await call(founder, 'privateOutbox', community.id);
  assert.ok(newObjects.length);
  for (const item of newObjects) await assert.rejects(call(member, 'privateReceive', community.id, item.body), 'a removed device must not receive new-epoch plaintext');
  await sync(founder, community.id);
  await assert.rejects(call(member, 'post', later.id));

  // Removal is reversible only through a fresh recipient-bound invitation. The
  // server still contains the old epochs, so synchronization must handle that history.
  const renewedRequest = await call(member, 'privateJoinRequest');
  const renewedInvite = await call(founder, 'privateInvite', community.id, renewedRequest);
  await sync(founder, community.id);
  await call(member, 'privateJoin', renewedInvite);
  assert.equal((await call(member, 'privateGroup', community.id)).active, true);
  const afterRejoinText = 'The fresh invitation restored access to this new epoch.';
  const afterRejoin = await call(founder, 'createPost', { community: community.id, title: 'After an approved rejoin', text: afterRejoinText });
  await sync(founder, community.id);
  await sync(member, community.id);
  assert.equal((await call(member, 'post', afterRejoin.id)).text, afterRejoinText);
  console.log('A removed member rejoined with a fresh request and received a new-epoch message through the retained relay history.');

  await founder.goto(`${origin}/#/private`);
  await boot(founder);
  await founder.screenshot({ path: resolve(output, 'private-groups.png'), fullPage: true });
  console.log('Deterministic ciphertext, persistence and removal checks passed; checking the configured UI and automatic synchronization.');

  // A fresh Vite instance has the actual application relay configuration. From here
  // on, membership and posting use rendered controls, and no coordinator is called.
  const uiOrigin = await startApp(['/relay']);
  const ownerUi = await person('Form owner', uiOrigin);
  const memberUi = await person('Form member', uiOrigin);
  await ownerUi.goto(`${uiOrigin}/#/new/private`);
  await boot(ownerUi);
  await ownerUi.getByRole('heading', { name: 'Start a community', exact: true }).waitFor();
  await ownerUi.getByLabel('Name', { exact: true }).fill('private-ui-test');
  await ownerUi.getByLabel('What it’s about', { exact: true }).fill('Created through the real private community form.');
  const create = ownerUi.getByRole('button', { name: 'Create private community', exact: true });
  assert.equal(await create.isEnabled(), true, 'a configured private relay enables creation');
  await create.click();
  await ownerUi.getByRole('heading', { name: 'private-ui-test', exact: true }).waitFor({ timeout: 30_000 });
  const uiGroups = await call(ownerUi, 'privateGroups');
  assert.equal(uiGroups.length, 1);
  const uiCommunity = uiGroups[0].community;

  await memberUi.goto(`${uiOrigin}/#/private`);
  await boot(memberUi);
  await memberUi.getByRole('button', { name: 'Create invitation request', exact: true }).click();
  const requestField = memberUi.getByLabel('Send this request to the owner', { exact: true });
  await requestField.waitFor();
  await ownerUi.getByText('Invite someone or manage devices', { exact: true }).click();
  await ownerUi.getByLabel('Their invitation request', { exact: true }).fill(await requestField.inputValue());
  await ownerUi.getByRole('button', { name: 'Check invitation request', exact: true }).click();
  const expectedMember = await call(memberUi, 'me');
  const verifiedKey = ownerUi.getByLabel('Verified person key', { exact: true });
  await verifiedKey.waitFor();
  assert.equal(await verifiedKey.inputValue(), expectedMember.rootKey);
  await ownerUi.getByRole('button', { name: 'Approve this device', exact: true }).click();
  const invitation = ownerUi.getByLabel('Send this private invitation directly to your friend', { exact: true });
  await invitation.waitFor({ timeout: 30_000 });
  await memberUi.getByLabel('Your private invitation', { exact: true }).fill(await invitation.inputValue());
  await memberUi.getByRole('button', { name: 'Open private invitation', exact: true }).click();
  await memberUi.getByRole('dialog', { name: 'Turn on your VPN before joining' }).getByRole('button', { name: 'Continue joining' }).click();
  await memberUi.getByRole('heading', { name: 'private-ui-test', exact: true }).waitFor({ timeout: 30_000 });
  assert.equal((await call(memberUi, 'privateGroup', uiCommunity)).active, true);

  const uiTitle = 'Delivered automatically from the private form';
  await ownerUi.getByRole('button', { name: '+ Post in c/private-ui-test', exact: true }).click();
  const composer = ownerUi.getByRole('form', { name: 'New post', exact: true });
  await composer.getByLabel('Title', { exact: true }).fill(uiTitle);
  await composer.getByLabel('Text', { exact: true }).fill('The app sent this without a manual synchronization call.');
  await composer.getByRole('button', { name: 'Sign & send', exact: true }).click();
  await memberUi.getByText(uiTitle, { exact: true }).waitFor({ timeout: 45_000 });
  await memberUi.waitForFunction(async community => {
    const { app } = await import('/src/lib/app.svelte.ts');
    return !!app.privateSync[community]?.lastSync && app.privateSync[community].replicas === 1;
  }, uiCommunity, { timeout: 30_000 });
  const uiStored = await objects(memberUi, uiCommunity);
  assert.ok(uiStored.length > 0, 'automatic synchronization persisted ciphertext');
  for (const item of uiStored) assert.equal((await ciphertext(memberUi, uiCommunity, item.id)).includes(Buffer.from(uiTitle)), false);
  await memberUi.getByTitle('Private relay checked', { exact: true }).waitFor();
  assert.doesNotMatch(await memberUi.locator('body').innerText(), /only in this browser|needs the desktop app|updates cannot be sent or received yet/);
  await memberUi.screenshot({ path: resolve(output, 'private-community-automatic-sync.png'), fullPage: true });
  await ownerUi.goto(`${uiOrigin}/#/private`);
  await boot(ownerUi);
  await ownerUi.getByRole('heading', { name: 'Private communities', exact: true }).waitFor();
  assert.doesNotMatch(await ownerUi.locator('body').innerText(), /only in this browser|needs the desktop app|updates cannot be sent or received yet/,
    'private communities must not display misleading public-transport setup warnings');
  await ownerUi.screenshot({ path: resolve(output, 'private-communities-form.png'), fullPage: true });
  assert.deepEqual(errors, [], 'no uncaught browser errors');
  console.log(`PASS: real browser private membership, relay v2 signed reads and sealed padded objects, encrypted media, tamper and replay resistance, reload persistence, removal/rejoin epoch rooms, rendered forms and automatic synchronization. Screenshots: ${output}`);
} catch (error) {
  for (const page of pages) console.error('Private browser state:', page.url(), await page.locator('body').innerText().catch(() => 'unavailable'));
  console.error('Browser errors:', errors);
  throw error;
} finally {
  await browser.close();
  for (const server of servers) await server.close();
}
