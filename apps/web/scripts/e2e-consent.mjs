// Informed before connected: the Network & Safety notice blocks the app until both
// boxes are ticked, stays acknowledged across reloads, comes back when its version changes,
// and an operator's notice from a node the browser met cannot be dismissed, only answered.
// Uses the mock backend; no network is contacted.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { existsSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { CONSENT_KEY, NETWORK_NOTICE_VERSION } from '../src/lib/legal/version.mjs';

const web = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const port = 6300 + Math.floor(Math.random() * 100);
const base = `http://127.0.0.1:${port}`;
const server = spawn(process.execPath, [resolve(web, 'node_modules/vite/bin/vite.js'), '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
  cwd: web,
  env: { ...process.env, VITE_HOI_BACKEND: 'mock', VITE_HOI_REGISTRY_URL: '', VITE_HOI_RELAY_URLS: '[]' },
  stdio: ['ignore', 'pipe', 'pipe'],
});
let output = '', browser;
server.stdout.on('data', b => { output += b; });
server.stderr.on('data', b => { output += b; });
const read = (page) => page.evaluate((key) => localStorage.getItem(key), CONSENT_KEY);
try {
  let ready = false, lastResponse = '';
  for (let i = 0; i < 100; i++) {
    try {
      const response = await fetch(base);
      if (response.ok) { ready = true; break; }
      lastResponse = `HTTP ${response.status}`;
    } catch (error) { lastResponse = String(error.cause ?? error); }
    if (server.exitCode !== null) break;
    await new Promise(r => setTimeout(r, 100));
  }
  assert(ready, `${output}\n${lastResponse}`);
  const chrome = 'C:/Program Files/Google/Chrome/Application/chrome.exe';
  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || (existsSync(chrome) ? chrome : undefined) });
  const shots = resolve(web, '.shots/consent');
  mkdirSync(shots, { recursive: true });
  for (const width of [1280, 390]) {
    const context = await browser.newContext({ viewport: { width, height: 844 }, reducedMotion: 'reduce' });
    const page = await context.newPage();
    page.setDefaultTimeout(15_000);
    const failures = [];
    page.on('pageerror', e => failures.push(e.message));

    // 1. First start: the notice, and nothing of the app, until both boxes are ticked.
    await page.goto(`${base}/?me=lena#/`);
    await page.getByRole('heading', { name: 'Network & Safety' }).waitFor();
    assert.equal(await page.getByRole('heading', { name: 'Hey lena.' }).count(), 0, 'the app must not start behind the notice');
    const go = page.getByRole('button', { name: 'Continue', exact: true });
    assert(await go.isDisabled(), 'Continue waits for both boxes');
    await page.locator('#safety-ack-ip').check();
    assert(await go.isDisabled(), 'one box is not enough');
    await page.locator('#safety-ack-vpn').check();
    assert(!(await go.isDisabled()));
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'the notice must fit the screen');
    await page.screenshot({ path: resolve(shots, `safety-${width}.png`), fullPage: true });
    assert.equal(await read(page), null, 'nothing is stored before Continue');
    await go.click();
    await page.getByRole('heading', { name: 'Hey lena.' }).waitFor();
    const stored = JSON.parse(await read(page));
    assert.equal(stored.version, NETWORK_NOTICE_VERSION);
    assert.match(stored.date, /^\d{4}-\d{2}-\d{2}$/);

    // 2. Acknowledged: a reload goes straight in; the privacy page says when.
    await page.reload();
    await page.getByRole('heading', { name: 'Hey lena.' }).waitFor();
    assert.equal(await page.getByRole('heading', { name: 'Network & Safety' }).count(), 0);
    await page.getByRole('link', { name: 'Privacy', exact: true }).first().click();
    await page.getByRole('heading', { name: /Privacy Notice/ }).waitFor();
    await page.getByText(`Acknowledged on this browser on ${stored.date}`).waitFor();
    await page.getByText('Data stored on your device.').waitFor();
    await page.screenshot({ path: resolve(shots, `privacy-${width}.png`), fullPage: true });

    // 3. A changed version asks again, and says so.
    await page.evaluate((key) => localStorage.setItem(key, JSON.stringify({ version: '0.1', date: '2026-01-01' })), CONSENT_KEY);
    await page.goto(`${base}/?me=lena#/`);
    await page.reload();
    await page.getByRole('heading', { name: 'Network & Safety' }).waitFor();
    await page.getByText('This notice changed since you last read it').waitFor();
    await page.locator('#safety-ack-ip').check();
    await page.locator('#safety-ack-vpn').check();
    await page.getByRole('button', { name: 'Continue', exact: true }).click();
    await page.getByRole('heading', { name: 'Hey lena.' }).waitFor();

    // 4. A node the browser met asks to be read: the dialog cannot be dismissed, only answered.
    await page.goto(`${base}/?me=lena&notice#/`);
    await page.getByRole('heading', { name: 'Hey lena.' }).waitFor();
    const dialog = page.getByRole('dialog', { name: 'Before this node serves you' });
    await dialog.waitFor();
    await dialog.getByText('This node is operated by Lena. Contact: lena at example dot org.').waitFor();
    await page.keyboard.press('Escape');
    await page.waitForTimeout(300);
    assert(await dialog.isVisible(), 'Escape must not dismiss an operator notice');
    assert.equal(await dialog.getByRole('button', { name: 'Close', exact: true }).count(), 0, 'no close button');
    await page.screenshot({ path: resolve(shots, `operator-${width}.png`) });
    await dialog.getByRole('button', { name: 'Decline' }).click();
    await dialog.waitFor({ state: 'hidden' });
    // Same query from here on: a changed query reloads the page, and the mock keeps nothing across reloads.
    await page.goto(`${base}/?me=lena&notice#/you`);
    await page.getByText('declined', { exact: true }).first().waitFor();
    await page.getByRole('button', { name: 'Accept', exact: true }).click();
    await page.getByText('accepted', { exact: true }).first().waitFor();
    await page.getByRole('button', { name: 'Forget', exact: true }).click();
    await page.getByText('no notices yet').waitFor();

    // 5. A node link that carries a notice shows it before connecting; one without warns.
    const fp = 'q'.repeat(43);
    const endpoint = 'ab'.repeat(32);
    await page.goto(`${base}/?me=lena&notice#/node?w=${fp}@203.0.113.9:7844&p=${endpoint}@203.0.113.9:7843&nt=eJyrVkpUsgIA&ns=AAAA`);
    await page.getByRole('heading', { name: 'A public node.' }).waitFor();
    await page.getByText('Join notice – sleepy-walrus-48213').waitFor();
    await page.getByText("The operator's notice, signed by the node this link names").waitFor();
    await page.screenshot({ path: resolve(shots, `node-link-${width}.png`), fullPage: true });
    await page.getByRole('button', { name: 'Decline' }).click();
    await page.waitForURL(/#\/communities/);
    await page.goto(`${base}/?me=lena&notice#/node?w=${fp}@203.0.113.9:7844&p=${endpoint}@203.0.113.9:7843&nt=eJyrVkpUsgIA&ns=AAAA`);
    await page.getByRole('button', { name: 'Accept and connect' }).click();
    await page.waitForURL(/#\/communities/);
    await page.goto(`${base}/?me=lena&notice#/node?w=${'r'.repeat(43)}@203.0.113.10:7844`);
    await page.getByText('This node has not published an operator notice.').waitFor();
    const connect = page.getByRole('button', { name: 'Connect and look around' });
    assert(await connect.isDisabled(), 'a link without a notice waits for the extra acknowledgement');
    await page.locator('#node-ack-nonotice').check();
    assert(!(await connect.isDisabled()));
    await connect.click();
    await page.waitForURL(/#\/communities/);
    assert.deepEqual(failures, []);
    await context.close();
  }
  console.log('PASS: Network & Safety gates the app until acknowledged, returns on a new version, operator notices are answered not dismissed, and node links show the notice first, at desktop and phone widths.');
} finally {
  await browser?.close();
  server.kill();
}
