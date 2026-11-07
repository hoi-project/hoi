// The wiki and rules in a real browser: Loro inside the wasm core, a page written,
// saved, read back after a reload, and a rules change proposed and in force.
import assert from 'node:assert/strict';
import { resolve } from 'node:path';
import { chromium } from 'playwright';
import { createServer } from '../apps/web/node_modules/vite/dist/node/index.js';
import { acceptNetworkNotice, unlockBrowser } from './browser-vault.mjs';

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

async function phase(page, expected) {
  const until = Date.now() + 60_000;
  while (Date.now() < until) {
    try {
      if ((await page.evaluate(async () => (await import('/src/lib/app.svelte.ts')).app.phase)) === expected) return;
    } catch (e) {
      if (!/Execution context was destroyed|Cannot find context/.test(e.message)) throw e;
    }
    await new Promise((r) => setTimeout(r, 50));
  }
  throw new Error(`expected ${expected}`);
}

try {
  const context = await browser.newContext({ reducedMotion: 'reduce' });
  const page = await context.newPage();
  page.on('pageerror', (e) => errors.push(e.message));
  await page.goto(origin);
  await acceptNetworkNotice(page);
  await phase(page, 'locked');
  await unlockBrowser(page);
  await phase(page, 'ready');
  const c = await page.evaluate(async () => {
    const { app } = await import('/src/lib/app.svelte.ts');
    await app.identityChanged(await app.core.createIdentity('Wiki writer'));
    const c = await app.core.createCommunity({ name: 'wiki-check', description: 'Wiki in the browser', color: 'mint', limits: { powBits: 12 }, rules: '1. Be kind.' });
    await app.refresh();
    return c.id;
  });
  // the rules came through Loro: a change, and then the rules shown
  const rules = await page.evaluate(async (id) => {
    const { app } = await import('/src/lib/app.svelte.ts');
    await app.core.setRules(id, '1. Be kind.\n2. Credit your sources.');
    return (await app.core.community(id)).rules?.text;
  }, c);
  assert.equal(rules, '1. Be kind.\n2. Credit your sources.');
  console.log('✓ a rules change made in the browser is in force, merged by Loro in the wasm core');
  // a page through the screen
  await page.evaluate((h) => { location.hash = h; }, `#/c/${c}/wiki`);
  await page.locator('#wk-new').fill('getting-started');
  await page.getByRole('button', { name: 'Start it', exact: true }).click();
  await page.locator('#wk-text').fill('Welcome.\n\nRead the rules first.');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await page.getByText('Read the rules first.', { exact: true }).waitFor();
  console.log('✓ a page written and saved from the wiki screen');
  // read back after a reload, from the encrypted store
  await page.reload();
  await phase(page, 'locked');
  await unlockBrowser(page);
  await phase(page, 'ready');
  const text = await page.evaluate(async (id) => (await (await import('/src/lib/app.svelte.ts')).app.core.wikiPage(id, 'getting-started')).text, c);
  assert.equal(text, 'Welcome.\n\nRead the rules first.');
  console.log('✓ the page is there after a reload');
  assert.deepEqual(errors, []);
  console.log('PASS: wiki pages and rules changes as Loro documents in a real browser.');
} finally {
  await browser.close();
  await server.close();
}
