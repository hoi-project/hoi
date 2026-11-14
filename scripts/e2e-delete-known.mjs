// The You screen's "Delete known posts and comments" and "Clear local data" with an offline mock
// core: deletions made one at a time, confirmation, resumed status, no second list on reopening,
// the clear refused while deletions need this browser, and 1280/320 px layout.
// Run: node scripts/e2e-delete-known.mjs
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
        <div id="post-fixture" class="gutter"></div>
        <script type="module">
          import { mount, unmount } from 'svelte';
          import You from '/src/routes/You.svelte';
          import Post from '/src/routes/Post.svelte';
          import { app } from '/src/lib/app.svelte.ts';
          import '@hoi/tokens/fonts.css';
          import '@hoi/tokens/tokens.css';
          import '/src/app.css';
          await app.start();
          if (app.phase !== 'ready') throw new Error('Mock app did not start.');
          app.refresh = async () => {};
          app.refreshWarmth = async () => {};
          const community = (await app.core.communities()).find((c) => c.joined) ?? (await app.core.communities())[0];
          const [first] = await Promise.all(['First post to delete', 'Second post to delete'].map((title) => app.core.createPost({ community: community.id, title, text: 'Example text.' })));
          // deleted one at a time, before any list: a post, and a reply under a post that stays for the list
          const alone = await app.core.createPost({ community: community.id, title: 'Deleted on its own', text: 'Example text.' });
          const reply = await app.core.reply(first.id, 'A reply deleted on its own.');
          await app.core.deleteOwn(alone.id);
          await app.core.deleteOwn(reply.id);
          let shown;
          const mountPost = (id) => {
            if (shown) unmount(shown);
            shown = mount(Post, { target: document.querySelector('#post-fixture'), props: { id } });
          };
          window.deleteFixture = { app, mountPost, community: community.name, alone: alone.id, thread: first.id };
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

  // deletions made from a post or a comment, each linked to where it was
  const fixture = await page.evaluate(() => { const { app: _, mountPost: __, ...rest } = window.deleteFixture; return rest; });
  const singles = dialog.getByRole('region', { name: 'Deleted one at a time' });
  await singles.waitFor();
  const byName = ([a], [b]) => a.localeCompare(b);
  const singleLinks = async () => (await singles.getByRole('link').evaluateAll((links) => links.map((a) => [a.textContent, a.getAttribute('href')]))).sort(byName);
  const expectedSingles = [[`Post in c/${fixture.community}`, `#/p/${fixture.alone}`], [`Comment in c/${fixture.community}`, `#/p/${fixture.thread}`]].sort(byName);
  assert.deepEqual(await singleLinks(), expectedSingles, 'a post links to itself, a reply to its thread');
  const singleText = await singles.innerText();
  assert.match(singleText, /Signed, not sent to the community yet\./, 'signed is not the same as sent');
  assert.match(singleText, /Still needs this browser/);
  assert.doesNotMatch(singleText, /\b(everywhere|erased|tombstone)\b/i, 'no line claims what peers did');
  await dialog.getByRole('heading', { name: 'Clear this browser afterwards' }).waitFor();
  assert.equal(await dialog.getByRole('button', { name: 'Clear local data…' }).count(), 0, 'single deletions block the combined clear too');
  assert.deepEqual(await audit(dialog), [], 'single deletions are accessible');
  await dialog.screenshot({ path: resolve(shots, 'single-1280.png') });
  await dialog.getByRole('button', { name: 'Review known posts and comments' }).click();
  await dialog.getByText('Not on this list', { exact: true }).waitFor();
  assert.match(await dialog.innerText(), /This list can't prove it's complete/, 'the preview never claims a complete history');
  assert.equal(await batches(), 0, 'reviewing changes nothing');
  await dialog.screenshot({ path: resolve(shots, 'review-1280.png') });
  assert.deepEqual(await audit(dialog), [], 'the review is accessible');
  await dialog.getByRole('button', { name: /^Delete these \d+$/ }).click();
  assert.match(await dialog.innerText(), /There's no undo\. Your key stays on this browser until the work is done; it isn't burned\./);
  await dialog.getByRole('button', { name: 'Yes, delete them' }).click();
  // the saved list itself: single deletions already show the section below
  await dialog.getByRole('region', { name: /^List of \d+ from / }).waitFor();
  const status = await dialog.innerText();
  assert.match(status, /deletions? signed\. 0 sent to the community/, 'signed is not the same as sent');
  assert.match(status, /still needs? this browser/, 'the core says the deletions still need this browser');
  assert.equal(await dialog.getByRole('button', { name: 'Clear local data…' }).count(), 0, 'no combined clear while work remains');
  assert.equal(await batches(), 1);
  assert.deepEqual(await singleLinks(), expectedSingles, 'deletions in a list are not shown again as single ones');
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

  // the deleted post's own page shows its deletion: the mock's "Poof." view, then the core's "not found"
  await page.setViewportSize({ width: 1280, height: 900 });
  const postPage = page.locator('#post-fixture');
  const own = postPage.getByRole('region', { name: 'Your deletion' });
  await page.evaluate((id) => window.deleteFixture.mountPost(id), fixture.alone);
  await postPage.getByRole('heading', { name: 'Poof.' }).waitFor();
  await own.waitFor();
  assert.match(await own.innerText(), /Signed, not sent to the community yet\.[\s\S]*Still needs this browser/);
  assert.equal(await own.getByRole('link', { name: 'You' }).getAttribute('href'), '#/you');
  assert.deepEqual(await audit(postPage), [], 'the deleted post page is accessible');
  await postPage.screenshot({ path: resolve(shots, 'post-1280.png') });
  await page.evaluate((id) => {
    const core = window.deleteFixture.app.core;
    core.post = async () => { throw Object.assign(new Error('Poof. Removed by its author: deleted by the author.'), { code: 'not-found' }); };
    window.deleteFixture.mountPost(id);
  }, fixture.alone);
  await postPage.getByRole('heading', { name: "That post isn't here." }).waitFor();
  await own.waitFor();
  assert.match(await own.innerText(), /Signed, not sent to the community yet\./, 'the core answers "not found" for a deleted post; its deletion still shows');
  await page.setViewportSize({ width: 320, height: 900 });
  assert.ok(await postPage.evaluate((el) => el.scrollWidth <= el.clientWidth + 1), 'the deleted post page fits 320 px');

  assert.deepEqual(externalRequests, [], 'nothing here makes an external request');
  assert.deepEqual(errors, []);
  await context.close();
  console.log('PASS: delete known posts and comments (single deletions, their links and the deleted post page, review, confirm, resumed status, no second list), the guarded and the ordinary clear, keyboard, axe on each dialog and 1280/320 px layout (offline mock core).');
} finally {
  await browser?.close();
  await server.close();
}
