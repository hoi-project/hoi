// Verify that every user-facing join waits for the VPN warning, including dismissal and retry.
// Uses the existing mock backend; no VPN or external peer is contacted.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { existsSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { consentAccepted } from '../../../scripts/browser-vault.mjs';

const web = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const port = 6100 + Math.floor(Math.random() * 100);
const base = `http://127.0.0.1:${port}`;
const server = spawn(process.execPath, [resolve(web, 'node_modules/vite/bin/vite.js'), '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
  cwd: web,
  env: { ...process.env, VITE_HOI_BACKEND: 'mock', VITE_HOI_REGISTRY_URL: '', VITE_HOI_RELAY_URLS: '["/relay"]' },
  stdio: ['ignore', 'pipe', 'pipe'],
});
let output = '', browser;
server.stdout.on('data', b => { output += b; });
server.stderr.on('data', b => { output += b; });
try {
  let ready = false, lastResponse = '';
  for (let i = 0; i < 100; i++) {
    try {
      const response = await fetch(base);
      if (response.ok) { ready = true; break; }
      lastResponse = `HTTP ${response.status}: ${(await response.text()).slice(0, 1000)}`;
    } catch (error) { lastResponse = String(error.cause ?? error); }
    if (server.exitCode !== null) break;
    await new Promise(r => setTimeout(r, 100));
  }
  assert(ready, `${output}\n${lastResponse}`);
  const chrome = 'C:/Program Files/Google/Chrome/Application/chrome.exe';
  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || (existsSync(chrome) ? chrome : undefined) });
  const shots = resolve(web, '.shots/join-warning');
  mkdirSync(shots, { recursive: true });
  for (const width of [1280, 390]) {
    const context = await browser.newContext({ viewport: { width, height: 844 }, reducedMotion: 'reduce' });
    await consentAccepted(context);
    const page = await context.newPage();
    page.setDefaultTimeout(10_000);
    const failures = [];
    page.on('pageerror', e => failures.push(e.message));
    await page.goto(`${base}/?me=lena#/`);
    await page.getByRole('heading', { name: 'Hey lena.' }).waitFor();
    const fixture = await page.evaluate(async () => {
      const { app } = await import('/src/lib/app.svelte.ts');
      const community = app.joined[0];
      const invite = await app.core.invite(community.id);
      await app.core.leave(community.id);
      window.joinCalls = { join: 0, publicJoin: 0, privateJoin: 0 };
      for (const method of Object.keys(window.joinCalls)) {
        const original = app.core[method].bind(app.core);
        app.core[method] = async (...args) => { window.joinCalls[method]++; return original(...args); };
      }
      location.hash = `#/join?${invite.uri.split('?')[1]}`;
      return { id: community.id, name: community.name };
    });
    const warning = page.getByRole('dialog', { name: 'Turn on your VPN before joining' });
    const count = method => page.evaluate(method => window.joinCalls[method], method);
    async function dismissThenContinue(trigger, method, before, shot = false) {
      await trigger.click();
      await warning.waitFor();
      assert.equal(await count(method), before, 'Opening the warning must not join');
      await warning.getByRole('button', { name: 'Not now' }).click();
      await warning.waitFor({ state: 'hidden' });
      assert.equal(await count(method), before, 'Not now must not join');
      await trigger.click();
      await warning.waitFor();
      await page.keyboard.press('Escape');
      await warning.waitFor({ state: 'hidden' });
      assert.equal(await count(method), before, 'Escape must not join');
      await trigger.click();
      await warning.getByRole('button', { name: 'Close', exact: true }).click();
      await warning.waitFor({ state: 'hidden' });
      assert.equal(await count(method), before, 'Closing must not join');
      await trigger.click();
      await warning.waitFor();
      assert(await warning.getByText(/activate your VPN/).isVisible());
      if (shot) {
        assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'Warning must fit the screen');
        await page.screenshot({ path: resolve(shots, `${width}.png`) });
      }
      await warning.getByRole('button', { name: 'Continue joining' }).click();
      await page.waitForFunction(({ method, total }) => window.joinCalls[method] === total, { method, total: before + 1 });
    }

    await dismissThenContinue(page.getByRole('button', { name: `Join c/${fixture.name}`, exact: true }), 'join', 0, true);
    await page.getByRole('button', { name: '✓ Joined', exact: true }).click();
    assert.equal(await warning.isVisible(), false, 'Leaving needs no join warning');
    await dismissThenContinue(page.getByRole('button', { name: 'Join', exact: true }), 'join', 1);
    await page.getByRole('button', { name: '✓ Joined', exact: true }).waitFor();

    await page.evaluate(async id => {
      const { app } = await import('/src/lib/app.svelte.ts');
      await app.core.leave(id);
      location.hash = `#/public?c=${id}`;
    }, fixture.id);
    await dismissThenContinue(page.getByRole('button', { name: 'Join public community', exact: true }), 'publicJoin', 0);
    await page.getByRole('link', { name: 'Open community', exact: true }).waitFor();

    await page.evaluate(() => { location.hash = '#/private'; });
    await page.getByLabel('Your private invitation', { exact: true }).fill('mock invitation');
    await dismissThenContinue(page.getByRole('button', { name: 'Open private invitation', exact: true }), 'privateJoin', 0);
    // The mock rejects MLS invitations. Confirm the error is handled and the user can retry.
    await page.locator('.toast.err').filter({ hasText: 'Private communities require the secure core.' }).waitFor();
    assert.equal(await page.getByLabel('Your private invitation', { exact: true }).inputValue(), 'mock invitation');
    await page.getByRole('button', { name: 'Open private invitation', exact: true }).click();
    await warning.waitFor();
    assert.equal(await count('privateJoin'), 1, 'A failed attempt must show the warning again');
    await warning.getByRole('button', { name: 'Not now' }).click();
    assert.deepEqual(failures, []);
    await context.close();
  }
  console.log('PASS: all four join flows wait for confirmation; cancel, Escape, close and failed-join retry are safe at desktop and phone widths.');
} finally {
  await browser?.close();
  server.kill();
}
