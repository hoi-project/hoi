// Mobile UI policy regression using explicit mock transport state. Real relay transport
// remains covered by scripts/e2e-public-network.mjs; no home-node pairing is used here.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { existsSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { consentAccepted } from '../../../scripts/browser-vault.mjs';

const web = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const port = 5900 + Math.floor(Math.random() * 100);
const base = `http://127.0.0.1:${port}`;
const server = spawn(process.execPath, [resolve(web, 'node_modules/vite/bin/vite.js'), '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
  cwd: web, env: { ...process.env, VITE_HOI_BACKEND: 'mock', VITE_HOI_REGISTRY_URL: '' }, stdio: ['ignore', 'pipe', 'pipe'],
});
let output = '', browser;
server.stdout.on('data', b => { output += b; });
server.stderr.on('data', b => { output += b; });
try {
  let ready = false;
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(base)).ok) { ready = true; break; } } catch { /* Starting. */ }
    if (server.exitCode !== null) break;
    await new Promise(r => setTimeout(r, 100));
  }
  assert(ready, output);
  const chrome = 'C:/Program Files/Google/Chrome/Application/chrome.exe';
  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || (existsSync(chrome) ? chrome : undefined) });
  const page = await browser.newPage({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true, reducedMotion: 'reduce' });
  await consentAccepted(page.context());
  page.setDefaultTimeout(15_000);
  const failures = [];
  page.on('pageerror', error => failures.push(error.message));
  await page.goto(`${base}/?me=lena#/`);
  await page.getByRole('heading', { name: 'Hey lena.' }).waitFor();
  const community = await page.evaluate(async () => {
    const { app } = await import('/src/lib/app.svelte.ts');
    const id = app.joined[0].id;
    window.readingFixture = { id, calls: 0, replies: 0, net: { mode: 'local', label: 'local', peers: 0, rtc: [], relaying: false, communities: [] } };
    app.core.net = async () => window.readingFixture.net;
    app.net = window.readingFixture.net;
    const create = app.core.createPost.bind(app.core), reply = app.core.reply.bind(app.core);
    app.core.createPost = async draft => { window.readingFixture.calls++; return create(draft); };
    app.core.reply = async (...args) => { window.readingFixture.replies++; return reply(...args); };
    location.hash = `#/c/${id}`;
    return id;
  });
  await page.locator('#c-h').waitFor();
  assert(await page.locator('.rows .row').count() > 0 || await page.locator('.postrow').count() > 0 || await page.locator('a[href^="#/p/"]').count() > 0, 'Saved community posts are readable without a node');
  await page.evaluate(async id => (await import('/src/lib/app.svelte.ts')).app.openComposer(id), community);
  const form = page.getByRole('form', { name: 'New post' });
  await form.getByLabel('Title', { exact: true }).fill('A mobile draft that survives a disconnect');
  await form.getByLabel('Text', { exact: true }).fill('Keep this text until a matching community connection exists.');
  const send = form.getByRole('button', { name: 'Sign & send', exact: true });
  assert(await send.isDisabled());
  async function connection(matching, coreOnly = false) {
    await page.evaluate(async ({ matching, coreOnly }) => {
      const { app } = await import('/src/lib/app.svelte.ts');
      const f = window.readingFixture;
      f.net = { ...f.net, peers: matching === 'offline' ? 0 : 1, communities: matching === 'offline' ? [] : [{ community: matching === 'mine' ? f.id : 'f'.repeat(64), connectedPeers: 1 }] };
      if (!coreOnly) app.net = f.net;
    }, { matching, coreOnly });
  }
  await connection('other');
  assert(await send.isDisabled(), 'An unrelated peer must not unlock this community');
  await connection('mine');
  await page.waitForFunction(() => !document.querySelector('form[aria-label="New post"] button[type="submit"]').disabled);
  // Cached UI says connected; live core status changes before the actual click.
  await connection('offline', true);
  await send.click();
  await page.locator('.toast').filter({ hasText: 'before posting' }).waitFor();
  assert.equal(await page.evaluate(() => window.readingFixture.calls), 0);
  assert.equal(await form.getByLabel('Title', { exact: true }).inputValue(), 'A mobile draft that survives a disconnect');
  assert.equal(await form.getByLabel('Text', { exact: true }).inputValue(), 'Keep this text until a matching community connection exists.');
  assert(await send.isDisabled());

  // Private communities retain their separate encrypted outbox path and do not call
  // the public transport preflight. The mock does not implement MLS itself.
  await page.evaluate(async id => {
    const { app } = await import('/src/lib/app.svelte.ts');
    app.privateGroups = [{ community: id, active: true, members: [], isOwner: false }];
    app.core.net = async () => { throw new Error('Public net must not be queried for a private post'); };
    await app.requirePostingConnection(id);
  }, community);
  await page.waitForFunction(() => !document.querySelector('form[aria-label="New post"] button[type="submit"]').disabled);
  await page.evaluate(async () => {
    const { app } = await import('/src/lib/app.svelte.ts');
    app.privateGroups = [];
    app.core.net = async () => window.readingFixture.net;
  });
  await connection('mine');
  await send.click();
  // No home node here: before the first post the app says, once, who can read it.
  const notice = page.getByRole('dialog', { name: 'Nobody can read posts from this browser' });
  await notice.getByText(/1 node connected now/).waitFor();
  await notice.getByRole('button', { name: 'Not now' }).click();
  await page.locator('.toast').filter({ hasText: 'Not posted. Your text stays in the editor.' }).waitFor();
  assert.equal(await page.evaluate(() => window.readingFixture.calls), 0, 'Not now signs nothing');
  assert.equal(await form.getByLabel('Title', { exact: true }).inputValue(), 'A mobile draft that survives a disconnect');
  await send.click();
  await notice.getByRole('button', { name: 'Post through them' }).click();
  await form.waitFor({ state: 'detached' });
  assert.equal(await page.evaluate(() => window.readingFixture.calls), 1);
  assert(await page.evaluate(async () => (await import('/src/lib/app.svelte.ts')).app.settings.postingNoticeSeen), 'asked once: the answer is kept');
  const post = await page.evaluate(async id => {
    const { app } = await import('/src/lib/app.svelte.ts');
    const posts = await app.core.feed({ community: id, sort: 'fresh', limit: 60 });
    return posts.find(p => p.title === 'A mobile draft that survives a disconnect').id;
  }, community);
  await page.evaluate(id => { location.hash = `#/p/${id}`; }, post);
  const reply = page.locator('form.reply').first();
  await reply.getByLabel('Reply', { exact: true }).fill('This reply also waits for its community.');
  await connection('offline');
  assert(await reply.getByRole('button', { name: 'Sign & send →', exact: true }).isDisabled());
  assert.equal(await reply.getByLabel('Reply', { exact: true }).inputValue(), 'This reply also waits for its community.');
  await connection('mine');
  await reply.getByRole('button', { name: 'Sign & send →', exact: true }).click();
  await page.waitForFunction(() => window.readingFixture.replies === 1);
  await page.getByText('This reply also waits for its community.', { exact: true }).first().waitFor();
  assert(!await page.evaluate(() => window.readingFixture.net.homeNode));
  assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'Mobile view has no horizontal overflow');
  const shots = resolve(web, '.shots/public-reading'); mkdirSync(shots, { recursive: true });
  await page.screenshot({ path: resolve(shots, 'phone-connected.png'), fullPage: true });
  assert.deepEqual(failures, []);
  console.log('PASS: mobile cached reading, exact-community post/reply gates, live preflight, retained text, and unchanged private flow; no home node.');
} finally {
  await browser?.close();
  server.kill();
}
