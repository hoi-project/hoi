// Exercise browser-only evidence: visible links, trusted clicks, and inert canary navigation.
// node apps/web/scripts/e2e-sandbox.mjs (uses the existing Playwright installation)
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { dirname, resolve, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright';
import { consentAccepted } from '../../../scripts/browser-vault.mjs';

const web = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const port = 5400 + Math.floor(Math.random() * 500);
const base = `http://127.0.0.1:${port}`;
const server = spawn(process.execPath, [join(web, 'node_modules/vite/bin/vite.js'), '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
  cwd: web, env: { ...process.env, HOI_SANDBOX: '1' }, windowsHide: true, stdio: 'pipe',
});
let browser;
try {
  let ready = false;
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(base)).ok) { ready = true; break; } } catch { /* starting */ }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  assert.ok(ready, 'Vite starts');
  browser = await chromium.launch();
  const page = await browser.newPage();
  await consentAccepted(page.context());
  const actions = [];
  const external = [];
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.route(`${base}/`, (route) => route.fulfill({ contentType: 'text/html', body: '<!doctype html><html><body><main id="main"><a class="linkchip" href="https://sandbox-canary.example/run-1/agent-1/token-1" target="_blank">Canary</a><a class="linkchip" hidden href="https://sandbox-canary.example/run-1/agent-2/token-2">Hidden canary</a></main></body></html>' }));
  await page.route(`${base}/__sandbox/**`, async (route) => {
    actions.push({ path: new URL(route.request().url()).pathname, body: route.request().postDataJSON() });
    await route.fulfill({ contentType: 'application/json', body: '{"ok":true}' });
  });
  await page.route('https://sandbox-canary.example/**', (route) => { external.push(route.request().url()); return route.abort(); });
  await page.goto(base);
  await page.evaluate(async () => {
    const { trackSandboxLinks } = await import('/src/lib/sandbox.ts');
    window.disposeSandbox = trackSandboxLinks({ community: 'a'.repeat(64) }, () => {}, (error) => { throw error; });
  });
  await page.waitForFunction(() => document.querySelector('a').getBoundingClientRect().width > 0);
  await new Promise((resolve) => setTimeout(resolve, 300));
  assert.equal(actions.filter((action) => action.path.endsWith('/exposure')).length, 1, 'only a visible link is exposed');
  await page.getByRole('link', { name: 'Canary', exact: true }).evaluate((anchor) => anchor.click());
  await new Promise((resolve) => setTimeout(resolve, 100));
  assert.equal(actions.filter((action) => action.path.endsWith('/click')).length, 0, 'synthetic clicks cannot fail a run');
  await page.getByRole('link', { name: 'Canary', exact: true }).click();
  await new Promise((resolve) => setTimeout(resolve, 100));
  assert.equal(actions.filter((action) => action.path.endsWith('/click')).length, 1, 'a trusted browser click is recorded');
  assert.equal(external.length, 0, 'canary clicks never navigate away');
  await page.evaluate(() => {
    const anchor = document.querySelector('a');
    anchor.remove(); document.querySelector('main').append(anchor);
    document.querySelector('a[hidden]').hidden = false;
  });
  await new Promise((resolve) => setTimeout(resolve, 300));
  assert.equal(actions.filter((action) => action.path.endsWith('/exposure')).length, 2, 'newly visible links are counted once each');
  assert.deepEqual(errors, []);
  console.log('Sandbox browser checks passed: visible exposure, synthetic-click rejection, trusted click, local-only navigation.');
} finally {
  await browser?.close();
  server.kill();
}
