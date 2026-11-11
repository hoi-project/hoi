// The You screen's "Delete known posts and comments" and "Clear local data" with an offline mock
// core: confirmation, resumed status, no second list on reopening, the clear refused while
// deletions need this browser, and 1280/320 px layout. Run: node scripts/e2e-delete-known.mjs
import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { chromium } from 'playwright';
import { startComponentServer } from './e2e-component-server.mjs';

const root = resolve(import.meta.dirname, '..');
const axePath = createRequire(import.meta.url).resolve('axe-core/axe.min.js');
const shots = resolve(root, 'test-results/36-delete-known');
process.env.VITE_HOI_BACKEND = 'mock';
const { server, origin } = await startComponentServer('/delete-known-test', `<!doctype html><html lang="en"><title>Delete known posts test</title>
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
          const community = (await app.core.communities()).find((c) => c.joined) ?? (await app.core.communities())[0];
          for (const title of ['First post to delete', 'Second post to delete']) await app.core.createPost({ community: community.id, title, text: 'Example text.' });
          window.deleteFixture = { app };
          mount(You, { target: document.querySelector('#fixture') });
        </script></html>`);
let browser;
const errors = [];
const externalRequests = [];
try {
  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
  const context = await browser.newContext({ reducedMotion: 'reduce', viewport: { width: 1280, height: 900 }, serviceWorkers: 'block' });
  await context.route('**/*', (route) => {
    if (new URL(route.request().url()).origin === origin) return route.continue();
    externalRequests.push(route.request().url());
    return route.abort();
  });
  await context.addInitScript({ path: axePath });
  const page = await context.newPage();
  /** Serious or critical axe findings inside an open dialog. */
  const audit = (dialog) => dialog.evaluate(async (el) => {
    const r = await window.axe.run(el, { runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa', 'best-practice'] }, resultTypes: ['violations'] });
    return r.violations.filter((v) => v.impact === 'serious' || v.impact === 'critical').map((v) => v.id);
  });
  page.on('pageerror', (error) => errors.push(error.message));
  page.setDefaultTimeout(15_000);
  await page.goto(`${origin}/delete-known-test?me=Example`);
  await mkdir(shots, { recursive: true });
  const batches = () => page.evaluate(async () => (await window.deleteFixture.app.core.bulkDeletions()).length);

  const row = page.locator('.set').filter({ has: page.getByText('Delete known posts and comments', { exact: true }) });
  await row.waitFor();
  assert.equal(await page.locator('.set').filter({ has: page.getByText('Clear local data', { exact: true }) }).count(), 1, 'clearing is its own, separately named action');
  const review = row.getByRole('button', { name: 'Review…' });
  await review.focus();
  await page.keyboard.press('Enter');
  const dialog = page.getByRole('dialog', { name: 'Delete known posts and comments' });
  await dialog.getByRole('button', { name: 'Review known posts and comments' }).click();
  await dialog.getByText('Not on this list', { exact: true }).waitFor();
  assert.match(await dialog.innerText(), /This list can't prove it's complete/, 'the preview never claims a complete history');
  assert.equal(await batches(), 0, 'reviewing changes nothing');
  await dialog.screenshot({ path: resolve(shots, 'review-1280.png') });
  assert.deepEqual(await audit(dialog), [], 'the review is accessible');
  await dialog.getByRole('button', { name: /^Delete these \d+$/ }).click();
  assert.match(await dialog.innerText(), /There's no undo\. Your key stays on this browser until the work is done; it isn't burned\./);
  await dialog.getByRole('button', { name: 'Yes, delete them' }).click();
  await dialog.getByRole('heading', { name: 'Clear this browser afterwards' }).waitFor();
  const status = await dialog.innerText();
  assert.match(status, /deletions? signed\. 0 sent to the community/, 'signed is not the same as sent');
  assert.match(status, /still needs? this browser/, 'the core says the deletions still need this browser');
  assert.equal(await dialog.getByRole('button', { name: 'Clear local data…' }).count(), 0, 'no combined clear while work remains');
  assert.equal(await batches(), 1);
  await dialog.screenshot({ path: resolve(shots, 'status-1280.png') });
  assert.deepEqual(await audit(dialog), [], 'the status is accessible');

  // reopening shows the saved list and starts nothing new
  await dialog.getByRole('button', { name: 'Done' }).click();
  await row.getByRole('button', { name: 'Review…' }).click();
  await dialog.getByRole('heading', { name: 'Clear this browser afterwards' }).waitFor();
  assert.equal(await batches(), 1, 'reopening never launches a second list');

  for (const width of [1280, 320]) {
    await page.setViewportSize({ width, height: 900 });
    const fit = await dialog.evaluate((el) => ({ right: el.getBoundingClientRect().right, left: el.getBoundingClientRect().left, width: window.innerWidth, scroll: el.scrollWidth, client: el.clientWidth }));
    assert.ok(fit.left >= 0 && fit.right <= fit.width + 1, `the dialog fits ${width} px`);
    assert.ok(fit.scroll <= fit.client + 1, `nothing overflows at ${width} px`);
    await dialog.screenshot({ path: resolve(shots, `status-${width}.png`) });
  }
  await page.setViewportSize({ width: 1280, height: 900 });

  // a screen told "ready" by a stale answer: the core still refuses, and nothing is cleared
  await dialog.getByRole('button', { name: 'Done' }).click();
  await page.evaluate(() => {
    const core = window.deleteFixture.app.core;
    const real = core.cleanupReadiness.bind(core);
    core.cleanupReadiness = async () => ({ ...(await real()), ready: true, blocked: [] });
    window.deleteFixture.restore = () => (core.cleanupReadiness = real);
  });
  await row.getByRole('button', { name: 'Review…' }).click();
  await dialog.getByRole('button', { name: 'Clear local data…' }).click();
  assert.match(await dialog.innerText(), /Exported key backups and your other devices keep theirs\. Deleting files doesn't guarantee the storage is physically erased\./);
  await dialog.getByRole('button', { name: 'Yes, clear it' }).click();
  await dialog.getByRole('alert').filter({ hasText: 'Nothing was cleared' }).waitFor();
  assert.equal(await page.evaluate(async () => (await window.deleteFixture.app.core.me())?.name), 'Example', 'a refused clear keeps the identity');
  await page.evaluate(() => window.deleteFixture.restore());
  await dialog.getByRole('button', { name: 'Done' }).click();

  // the ordinary clear says what it abandons and what it doesn't reach
  await page.locator('.set').filter({ has: page.getByText('Clear local data', { exact: true }) }).getByRole('button', { name: 'Wipe…' }).click();
  const wipe = page.getByRole('dialog', { name: 'Clear local data on this browser?' });
  await wipe.getByText('Unfinished deletions stop here.').waitFor();
  const wipeText = await wipe.innerText();
  assert.match(wipeText, /Not touched: exported key backups, your other devices and copies other members keep\./);
  assert.match(wipeText, /doesn't guarantee the storage is physically erased/);
  assert.match(wipeText, /Your vault passphrase stays/, 'an ordinary clear keeps the vault, unlike locked recovery');
  await wipe.screenshot({ path: resolve(shots, 'wipe-1280.png') });
  assert.deepEqual(await audit(wipe), [], 'the clear dialog is accessible');
  await page.setViewportSize({ width: 320, height: 900 });
  assert.ok(await wipe.evaluate((el) => el.scrollWidth <= el.clientWidth + 1), 'the clear dialog fits 320 px');
  await wipe.getByRole('button', { name: 'Keep everything' }).click();
  assert.equal(await batches(), 1, 'looking at the clear changes nothing');

  assert.deepEqual(externalRequests, [], 'nothing here makes an external request');
  assert.deepEqual(errors, []);
  await context.close();
  console.log('PASS: delete known posts and comments (review, confirm, resumed status, no second list), the guarded and the ordinary clear, keyboard, axe on each dialog and 1280/320 px layout (offline mock core).');
} finally {
  await browser?.close();
  await server.close();
}
