// Real WASM crypto and browser storage: fresh setup, reload, bad password, lock,
// competing tabs, ciphertext corruption and migration from the plaintext format.
import assert from 'node:assert/strict';
import { mkdir, readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { chromium } from 'playwright';
import { createServer } from '../apps/web/node_modules/vite/dist/node/index.js';
import { TEST_PASSPHRASE, acceptNetworkNotice, consentAccepted, unlockBrowser } from './browser-vault.mjs';
import { editStore, rawStore, storeHelpers } from './browser-store.mjs';

const root = resolve(import.meta.dirname, '..');
process.env.VITE_HOI_BACKEND = 'wasm';
process.env.VITE_HOI_REGISTRY_URL = '';
process.env.VITE_HOI_RELAY_URLS = '[]';
process.env.HOI_SANDBOX = '0';
const server = await createServer({ root: resolve(root, 'apps/web'), logLevel: 'error', server: { host: '127.0.0.1', port: 0, hmr: false } });
await server.listen();
const origin = `http://127.0.0.1:${server.httpServer.address().port}`;
const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
const errors = [];
const output = resolve(root, 'test-results/vault');
const axePath = createRequire(import.meta.url).resolve('axe-core/axe.min.js');
await mkdir(output, { recursive: true });

const raw = (page) => rawStore(page);
/** Where the app keeps this browser's store: OPFS files, or an IndexedDB database. */
const where = (page) => page.evaluate(async () => ({ opfs: !!(await window.__hoiStoreFiles('hoi')), idb: (await indexedDB.databases()).some((d) => d.name === 'hoi') }));

async function phase(page, expected) {
  // Keep module access in the main world; the mounted Svelte app owns the state.
  const until = Date.now() + 60_000;
  let state;
  while (Date.now() < until) {
    try {
      state = await page.evaluate(async () => {
        const { app } = await import('/src/lib/app.svelte.ts');
        return { phase: app.phase, error: app.bootError };
      });
      if (state.phase === expected) return;
    } catch (error) {
      if (!/Execution context was destroyed|Cannot find context/.test(error.message)) throw error;
    }
    await new Promise(done => setTimeout(done, 50));
  }
  throw new Error(`Expected ${expected}: ${JSON.stringify(state)}`);
}

// Some password managers set DOM values without firing input/change events. Exercise that
// path through the real form, not Svelte bindings or a mocked core call.
async function managerSubmit(page, formId, values) {
  await page.evaluate(({ formId, values }) => {
    const form = document.getElementById(formId);
    for (const [name, value] of Object.entries(values)) form.elements.namedItem(name).value = value;
    form.requestSubmit();
  }, { formId, values });
}

async function accessible(page, label) {
  await page.addScriptTag({ path: axePath });
  const failures = await page.evaluate(async () => (await axe.run(document, {
    runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa', 'best-practice'] },
  })).violations.filter(v => ['serious', 'critical'].includes(v.impact)).map(v => ({ id: v.id, targets: v.nodes.map(n => n.target) })));
  assert.deepEqual(failures, [], `Accessibility: ${label}`);
}

try {
  const context = await browser.newContext({ reducedMotion: 'reduce', viewport: { width: 1100, height: 850 } });
  await storeHelpers(context);
  const page = await context.newPage();
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(origin);
  await acceptNetworkNotice(page);
  await page.getByRole('heading', { name: 'Protect this device', exact: true }).waitFor();
  await phase(page, 'locked');
  const initial = await page.evaluate(async () => ({ phase: (await import('/src/lib/app.svelte.ts')).app.phase, state: (await import('/src/lib/app.svelte.ts')).app.vaultState, status: await (await import('/src/lib/core/index.ts')).getVaultStatus(), body: document.body.innerText }));
  assert.equal(initial.state, 'create', JSON.stringify(initial));
  assert.equal(await page.locator('#vault-username').getAttribute('autocomplete'), 'username');
  assert.equal(await page.locator('#vault-passphrase').getAttribute('autocomplete'), 'new-password');
  assert.equal(await page.evaluate(async () => {
    const { app } = await import('/src/lib/app.svelte.ts');
    try { void app.core; return false; } catch { return true; }
  }), true, 'no identity APIs are accessible before unlock');
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: resolve(output, 'protect-device-mobile.png'), fullPage: true });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await accessible(page, 'create vault');
  const vaultEntry = await page.locator('#vault-username').inputValue();
  await managerSubmit(page, 'vault-login', { username: vaultEntry, password: TEST_PASSPHRASE, 'password-confirmation': TEST_PASSPHRASE });
  await phase(page, 'ready');
  const identity = await page.evaluate(async () => {
    const { app } = await import('/src/lib/app.svelte.ts');
    const me = await app.core.createIdentity('Vault test identity');
    await app.identityChanged(me);
    const community = await app.core.createPrivateCommunity({ name: 'vault-private', description: 'Private local storage test', color: 'mint', limits: { powBits: 12 } });
    const post = await app.core.createPost({ community: community.id, title: 'private-vault-marker-98172', text: 'private-vault-message-37651' });
    return { root: me.rootKey, work: me.workKey, device: me.device, community: community.id, post: post.id };
  });
  await page.goto(`${origin}/#/you`);
  await page.getByRole('button', { name: 'Back up my identity', exact: true }).click();
  await page.getByRole('button', { name: 'Show my recovery key', exact: true }).click();
  const recoveryKey = await page.locator('#kb-key').inputValue();
  const recoveryEntry = await page.locator('#kb-login').inputValue();
  assert.notEqual(recoveryEntry, vaultEntry);
  assert.equal(await page.getByRole('button', { name: 'Forget the recovery key here', exact: true }).count(), 0);
  await page.getByRole('button', { name: 'Verify saved key', exact: true }).click();
  await accessible(page, 'verify a saved recovery key');
  assert.equal(await page.locator('#save-recovery').count(), 0, 'only the retrieval form is active');
  await managerSubmit(page, 'verify-recovery', { username: recoveryEntry, password: 'wrong saved credential' });
  await page.getByRole('alert').filter({ hasText: 'not this identity' }).waitFor();
  assert.equal(await page.locator('#kb-check-key').inputValue(), '', 'verification clears the submitted secret, including autofill without events');
  assert.equal(await page.getByRole('button', { name: 'Forget the recovery key here', exact: true }).count(), 0);
  await managerSubmit(page, 'verify-recovery', { username: recoveryEntry, password: recoveryKey.toUpperCase().replaceAll('-', ' ') });
  await page.getByRole('button', { name: 'Forget the recovery key here', exact: true }).waitFor();
  assert.equal(await page.evaluate(async () => (await import('/src/lib/app.svelte.ts')).app.me.workKey), identity.work, 'verifying a saved key cannot rotate the identity');
  await page.getByRole('button', { name: 'Forget the recovery key here', exact: true }).click();
  await page.waitForFunction(async () => !(await import('/src/lib/app.svelte.ts')).app.me.holdsLine);
  await page.getByRole('button', { name: 'Back up my identity', exact: true }).click();
  await page.getByRole('button', { name: 'Encrypted file', exact: true }).click();
  await accessible(page, 'export a backup file');
  assert.equal(await page.locator('#save-recovery').count(), 0);
  const fileEntry = await page.locator('#kb-file-login').inputValue();
  const downloadPromise = page.waitForEvent('download');
  await managerSubmit(page, 'export-backup', { username: fileEntry, password: 'vault backup passphrase', 'password-confirmation': 'vault backup passphrase' });
  const download = await downloadPromise;
  const backup = await readFile(await download.path());
  assert.equal(fileEntry, `hoi key file · ${download.suggestedFilename()}`);
  assert.match(await page.evaluate(async () => {
    try { await (await import('/src/lib/core/worker-core.ts')).workerVaultCall('__resetVault', ['RESET']); return ''; }
    catch (error) { return error.message; }
  }), /Lock hoi/, 'recovery cannot erase an unlocked runtime');
  const saved = await raw(page);
  const diskText = [...saved.records.map(([, r]) => r.wire), ...saved.blobs.map(([, value]) => value)].map(value => Buffer.from(value).toString('utf8')).join('\n');
  assert.doesNotMatch(diskText, /private-vault-marker|private-vault-message|Vault test identity/);
  assert.ok(saved.blobs.some(([key]) => key === 'v:keys'));
  assert.ok(saved.blobs.some(([key]) => key === 'v:mls'));
  assert.ok(!saved.blobs.some(([key]) => ['keys', 'mls', 'state'].includes(key)));
  assert.deepEqual(await where(page), { opfs: true, idb: false }, 'a browser with sync access handles keeps its store in OPFS, and makes no database');

  await page.reload();
  await phase(page, 'locked');
  assert.equal(await page.locator('#vault-username').inputValue(), vaultEntry, 'the manager entry survives reload');
  assert.equal(await page.locator('#vault-passphrase').getAttribute('autocomplete'), 'current-password');
  const beforeWrong = await raw(page);
  await page.locator('#vault-passphrase').fill('an incorrect long passphrase');
  await page.getByRole('button', { name: 'Unlock', exact: true }).click();
  await page.getByRole('alert').waitFor();
  assert.deepEqual(await raw(page), beforeWrong, 'wrong password cannot rewrite data or reset identity');
  await managerSubmit(page, 'vault-login', { username: vaultEntry, password: TEST_PASSPHRASE });
  await phase(page, 'ready');
  assert.equal(await page.evaluate(async () => (await import('/src/lib/app.svelte.ts')).app.me.rootKey), identity.root);
  assert.equal(await page.evaluate(async id => (await import('/src/lib/app.svelte.ts')).app.core.privateGroup(id).then(group => group.active), identity.community), true);

  const second = await context.newPage();
  second.on('pageerror', error => errors.push(error.message));
  await second.goto(origin);
  await phase(second, 'blocked');
  assert.match(await second.evaluate(async () => {
    try { await (await import('/src/lib/core/worker-core.ts')).workerVaultCall('__resetVault', ['RESET']); return ''; }
    catch (error) { return error.message; }
  }), /Open hoi in this tab/, 'another tab cannot clear the active writer storage');
  await second.getByRole('button', { name: 'Use it here', exact: true }).click();
  await phase(second, 'locked');
  await phase(page, 'blocked');
  await page.close();
  await unlockBrowser(second);
  await phase(second, 'ready');
  await second.getByRole('button', { name: 'Lock', exact: true }).click();
  await phase(second, 'locked');
  await second.screenshot({ path: resolve(output, 'locked.png'), fullPage: true });

  const mls = (await raw(second)).blobs.find(([key]) => key === 'v:mls')[1];
  mls[mls.length - 1] ^= 1;
  await editStore(second, [{ op: 'blob', key: 'v:mls', value: mls }]);
  const corrupted = await raw(second);
  await second.locator('#vault-passphrase').fill(TEST_PASSPHRASE);
  await second.getByRole('button', { name: 'Unlock', exact: true }).click();
  await second.getByRole('alert').waitFor();
  assert.deepEqual(await raw(second), corrupted, 'tampering fails closed without replacing encrypted state');

  await second.getByText("Can't unlock? Recover or start over", { exact: true }).click();
  await second.setViewportSize({ width: 390, height: 844 });
  await second.screenshot({ path: resolve(output, 'recovery-mobile.png'), fullPage: true });
  assert.equal(await second.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await accessible(second, 'locked recovery');
  const clear = second.getByRole('button', { name: 'Clear local data', exact: true });
  assert.equal(await clear.isDisabled(), true);
  await second.getByRole('checkbox', { name: /I have a key backup/ }).check();
  await second.locator('#vault-reset-confirm').fill('reset');
  assert.equal(await clear.isDisabled(), true, 'confirmation is exact and deliberate');
  await second.locator('#vault-reset-confirm').fill('RESET');
  // clearing reloads the page without the notice's consent: it asks again
  await Promise.all([second.waitForEvent('load'), clear.click()]);
  await acceptNetworkNotice(second);
  await second.getByRole('heading', { name: 'Protect this device', exact: true }).waitFor();
  assert.deepEqual(await raw(second), { records: [], blobs: [] });
  await unlockBrowser(second);
  await second.getByRole('heading', { name: 'Restore my identity', exact: true }).waitFor();
  await second.screenshot({ path: resolve(output, 'existing-key-mobile.png'), fullPage: true });
  assert.equal(await second.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await second.getByRole('button', { name: 'Use backup file', exact: true }).click();
  await accessible(second, 'restore a backup file');
  await second.locator('#fr-key-file').setInputFiles({ name: download.suggestedFilename(), mimeType: 'application/octet-stream', buffer: backup });
  assert.equal(await second.locator('#fr-file-entry').inputValue(), fileEntry);
  await second.locator('#fr-pass').fill('wrong backup passphrase');
  const beforeBadImport = await raw(second);
  await second.getByRole('button', { name: 'Restore backup file', exact: true }).click();
  await second.waitForFunction(() => document.querySelector('#fr-pass')?.value === '');
  assert.equal(await second.evaluate(async () => (await import('/src/lib/app.svelte.ts')).app.me), null);
  assert.deepEqual(await raw(second), beforeBadImport, 'a failed key import cannot change local data');
  // A damaged file also leaves the existing empty vault intact.
  await second.locator('#fr-key-file').setInputFiles({ name: download.suggestedFilename(), mimeType: 'application/octet-stream', buffer: Buffer.from('not a key backup') });
  await managerSubmit(second, 'restore-file', { username: fileEntry, password: 'vault backup passphrase' });
  await second.getByRole('alert').waitFor();
  await second.waitForFunction(() => document.querySelector('#fr-pass')?.value === '');
  assert.deepEqual(await raw(second), beforeBadImport);
  await second.locator('#fr-key-file').setInputFiles({ name: download.suggestedFilename(), mimeType: 'application/octet-stream', buffer: backup });
  await managerSubmit(second, 'restore-file', { username: fileEntry, password: 'vault backup passphrase' });
  await second.getByRole('heading', { name: 'Restore my identity', exact: true }).waitFor({ state: 'detached' });
  assert.equal(await second.evaluate(async () => (await import('/src/lib/app.svelte.ts')).app.me.rootKey), identity.root);
  await second.reload();
  await phase(second, 'locked');
  await unlockBrowser(second);
  await phase(second, 'ready');
  assert.equal(await second.evaluate(async () => (await import('/src/lib/app.svelte.ts')).app.me.rootKey), identity.root, 'import keeps the local vault passphrase');
  await second.getByRole('button', { name: 'Lock', exact: true }).click();
  await phase(second, 'locked');
  await editStore(second, [{ op: 'blob', key: 'vault', value: null }]);
  await second.reload();
  await phase(second, 'error');
  await second.getByText("Can't unlock? Recover or start over", { exact: true }).click();
  await second.getByRole('checkbox', { name: /I have a key backup/ }).check();
  await second.locator('#vault-reset-confirm').fill('RESET');
  await Promise.all([second.waitForEvent('load'), second.getByRole('button', { name: 'Clear local data', exact: true }).click()]);
  await acceptNetworkNotice(second);
  await second.getByRole('heading', { name: 'Protect this device', exact: true }).waitFor();
  assert.deepEqual(await raw(second), { records: [], blobs: [] }, 'recovery also works when damaged metadata prevents the unlock form');
  await context.close();

  const restoredContext = await browser.newContext({ reducedMotion: 'reduce' });
  await consentAccepted(restoredContext);
  await storeHelpers(restoredContext);
  const restoredPage = await restoredContext.newPage();
  restoredPage.on('pageerror', error => errors.push(error.message));
  await restoredPage.goto(origin);
  await unlockBrowser(restoredPage);
  await restoredPage.getByRole('heading', { name: 'Restore my identity', exact: true }).waitFor();
  await accessible(restoredPage, 'restore a recovery key');
  const beforeRestore = await raw(restoredPage);
  await managerSubmit(restoredPage, 'restore-recovery', { username: recoveryEntry, password: TEST_PASSPHRASE, 'display-name': 'Vault test identity' });
  await restoredPage.getByRole('alert').filter({ hasText: 'Choose your recovery-key entry' }).waitFor();
  assert.deepEqual(await raw(restoredPage), beforeRestore);
  await managerSubmit(restoredPage, 'restore-recovery', { username: recoveryEntry, password: recoveryKey, 'display-name': 'Vault test identity' });
  await restoredPage.getByRole('heading', { name: 'Restore my identity', exact: true }).waitFor({ state: 'detached', timeout: 60_000 });
  const restored = await restoredPage.evaluate(async () => (await import('/src/lib/app.svelte.ts')).app.me);
  assert.equal(restored.rootKey, identity.root);
  assert.notEqual(restored.device, identity.device);
  assert.notEqual(restored.workKey, identity.work);
  assert.equal(restored.restoring, true, 'network history is still needed after local restoration');
  await restoredPage.reload();
  await unlockBrowser(restoredPage);
  assert.equal(await restoredPage.evaluate(async () => (await import('/src/lib/app.svelte.ts')).app.me.rootKey), identity.root);
  await restoredContext.close();

  const legacyContext = await browser.newContext();
  await storeHelpers(legacyContext);
  const legacy = await legacyContext.newPage();
  legacy.on('pageerror', error => errors.push(error.message));
  await legacy.route('**/migration-fixture', route => route.fulfill({ contentType: 'text/html', body: '<!doctype html><title>Local test fixture</title>' }));
  await legacy.goto(`${origin}/migration-fixture`);
  const legacyRoot = await legacy.evaluate(async () => {
    const { default: init, WasmNode } = await import('/src/lib/core/wasm/pkg/hoi_wasm.js');
    const { openIdb } = await import('/src/lib/core/idb.ts');
    await init();
    const node = new WasmNode('web', location.origin + '/', [], []);
    const me = JSON.parse(node.call('createIdentity', JSON.stringify(['Legacy vault identity'])));
    node.call('createPrivateCommunity', JSON.stringify([{ name: 'legacy-private', description: 'Retain this group', color: 'mint', limits: { powBits: 12 } }]));
    const changes = node.take_changes();
    const db = await openIdb();
    await new Promise((resolve, reject) => {
      const tx = db.transaction(['records', 'blobs'], 'readwrite');
      for (const c of changes) {
        if (c.op === 'put') tx.objectStore('records').put({ wire: c.wire, community: c.community, expiresAt: c.expiresAt }, c.id);
        else if (c.op === 'blob') { if (c.value) tx.objectStore('blobs').put(c.value, c.key); else tx.objectStore('blobs').delete(c.key); }
        else if (c.op === 'delete') tx.objectStore('records').delete(c.id);
      }
      tx.oncomplete = resolve;
      tx.onerror = () => reject(tx.error);
    });
    db.close(); node.free(); return me.rootKey;
  });
  await legacy.goto(origin);
  await acceptNetworkNotice(legacy);
  await phase(legacy, 'locked');
  assert.equal(await legacy.evaluate(async () => (await import('/src/lib/app.svelte.ts')).app.vaultState), 'migrate');
  await unlockBrowser(legacy);
  await phase(legacy, 'ready');
  assert.equal(await legacy.evaluate(async () => (await import('/src/lib/app.svelte.ts')).app.me.rootKey), legacyRoot);
  assert.equal(await legacy.evaluate(async () => (await import('/src/lib/app.svelte.ts')).app.core.privateGroups().then(groups => groups.length)), 1);
  const migrated = await raw(legacy);
  assert.ok(migrated.blobs.some(([key]) => key === 'v:mls'));
  assert.ok(!migrated.blobs.some(([key]) => key === 'keys' || key === 'mls'));
  assert.deepEqual(await where(legacy), { opfs: false, idb: true }, 'a store already in IndexedDB stays there');
  await legacy.reload(); await phase(legacy, 'locked'); await unlockBrowser(legacy); await phase(legacy, 'ready');
  assert.equal(await legacy.evaluate(async () => (await import('/src/lib/app.svelte.ts')).app.me.rootKey), legacyRoot);
  await legacyContext.close();

  // Fail the first core save AFTER migration has installed passphrase A. The retry
  // must not silently accept B while leaving A on disk, nor start networking early.
  const recoveryContext = await browser.newContext();
  await consentAccepted(recoveryContext);
  await storeHelpers(recoveryContext);
  await recoveryContext.route('**/src/lib/core/idb.ts', async route => {
    const response = await route.fetch();
    const original = await response.text();
    await route.fulfill({ response, body: original + '\nconst actualApply = apply; let testWrites = 0; apply = async (...args) => { if (++testWrites === 2) throw new Error("simulated first-flush failure"); return actualApply(...args); };\n' });
  });
  const recovery = await recoveryContext.newPage();
  await recovery.goto(origin);
  await recovery.getByRole('heading', { name: 'Protect this device', exact: true }).waitFor();
  await recovery.locator('#vault-passphrase').fill(TEST_PASSPHRASE);
  await recovery.locator('#vault-confirm').fill(TEST_PASSPHRASE);
  await recovery.getByRole('button', { name: 'Protect and continue', exact: true }).click();
  await recovery.getByRole('alert').waitFor();
  await recovery.getByRole('heading', { name: 'Unlock this device', exact: true }).waitFor();
  assert.equal(await recovery.locator('#vault-confirm').count(), 0, 'a committed vault must not offer a new password');
  const interrupted = await raw(recovery);
  await recovery.locator('#vault-passphrase').fill('a different retry passphrase');
  await recovery.getByRole('button', { name: 'Unlock', exact: true }).click();
  await recovery.getByRole('alert').filter({ hasText: /incorrect|damaged/i }).waitFor();
  await phase(recovery, 'locked');
  assert.deepEqual(await raw(recovery), interrupted, 'a wrong retry password must not commit pending writes');
  await unlockBrowser(recovery);
  await phase(recovery, 'ready');
  await recoveryContext.unroute('**/src/lib/core/idb.ts');
  await recovery.reload();
  await phase(recovery, 'locked');
  await unlockBrowser(recovery);
  await phase(recovery, 'ready');
  await recoveryContext.close();
  assert.deepEqual(errors, []);
  console.log('PASS: encrypted storage, migration, wrong/damaged backups, lock ownership, recovery-key verification without rotation, file export/restore, recovery restoration, manager autofill without input events and mobile layout.');
} finally {
  await browser.close();
  await server.close();
}
