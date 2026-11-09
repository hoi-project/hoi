// Update controls and rendered spacing, with browser/native boundaries stubbed offline.
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
import { chromium } from 'playwright';
import { createServer } from '../apps/web/node_modules/vite/dist/node/index.js';

const root = resolve(import.meta.dirname, '..');
const axe = createRequire(import.meta.url).resolve('axe-core/axe.min.js');
const shots = resolve(root, 'test-results/49-updates');
mkdirSync(shots, { recursive: true });
const server = await createServer({ root: resolve(root, 'apps/web'), logLevel: 'error', server: { host: '127.0.0.1', port: 0, hmr: false }, plugins: [{
  name: 'updates-test-page',
  configureServer(server) {
    server.middlewares.use('/updates-test', async (_req, res) => {
      res.setHeader('Content-Type', 'text/html');
      res.end(await server.transformIndexHtml('/updates-test', `<!doctype html><html lang="en"><title>Update controls</title>
        <main class="gutter"><h1>Device settings</h1><div id="component"></div></main>
        <script type="module">
          import { mount } from 'svelte';
          import Updates from '/src/lib/ui/Updates.svelte';
          import { PUBLICATION } from '/src/lib/preview.ts';
          import { sw } from '/src/lib/sw.svelte.ts';
          import '@hoi/tokens/fonts.css';
          import '@hoi/tokens/tokens.css';
          import '/src/app.css';
          PUBLICATION.downloads = '';
          window.testUpdates.sw = sw;
          mount(Updates, { target: document.querySelector('#component') });
        </script></html>`));
    });
  },
}] });
await server.listen();
const origin = `http://127.0.0.1:${server.httpServer.address().port}`;
let browser;
const errors = [];
try {
  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
  for (const width of [1280, 320]) for (const desktop of [false, true]) {
    const context = await browser.newContext({ viewport: { width, height: 900 }, reducedMotion: 'reduce', serviceWorkers: 'block' });
    await context.addInitScript(({ desktop }) => {
      const state = window.testUpdates = { result: 'current', failDownload: false, calls: [], pending: null };
      if (!desktop) return;
      window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
      window.__TAURI_INTERNALS__ = { transformCallback: () => 42, invoke: async command => {
        state.calls.push(command);
        if (command.startsWith('plugin:event|')) return 1;
        if (command === 'hoi_update_check') {
          await new Promise(resolve => { state.pending = resolve; });
          return { state: state.result, version: '2.0.0', message: 'Release information is ready.' };
        }
        if (command === 'hoi_update_download') {
          await new Promise(resolve => { state.pending = resolve; });
          if (state.failDownload) throw 'The update signature did not verify.';
          return;
        }
        throw new Error(`Unexpected native command: ${command}`);
      } };
    }, { desktop });
    const page = await context.newPage();
    page.on('pageerror', error => errors.push(error.message));
    await page.route('**/*', route => new URL(route.request().url()).origin === origin ? route.continue() : route.abort());
    await page.goto(`${origin}/updates-test`);
    const panel = page.getByRole('region', { name: 'App updates' });
    const check = panel.getByRole('button', { name: 'Check for updates', exact: true });
    const status = panel.getByRole('status');
    await check.waitFor();
    assert.deepEqual(await page.evaluate(() => window.testUpdates.calls), [], 'no automatic update request');
    const gap = async (before, after, minimum, label) => {
      const first = await before.boundingBox();
      const second = await after.boundingBox();
      assert.ok(first && second && second.y - first.y - first.height >= minimum - 0.5, `${label}: at least ${minimum}px of separation at ${width}px`);
    };
    const layout = async () => {
      const heading = panel.getByRole('heading', { name: 'App updates' });
      const installed = panel.locator('p').filter({ hasText: /^Installed:/ });
      await gap(heading, installed, 8, 'heading and version');
      const help = panel.locator('p').filter({ hasText: desktop ? /^Checking contacts/ : /^The browser keeps/ });
      await gap(installed, help, 8, 'version and explanation');
      await gap(help, panel.locator('.actions'), 12, 'explanation and actions');
      if ((await status.textContent()).trim()) await gap(panel.locator('.actions'), status, 12, 'actions and status');
      const alert = panel.getByRole('alert');
      if (await alert.count()) await gap(status, alert, 12, 'status and error');
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, 'no horizontal overflow');
      for (const button of await panel.getByRole('button').all()) {
        const bounds = await button.boundingBox();
        assert.ok(bounds && bounds.height >= 44 && bounds.x >= 0 && bounds.x + bounds.width <= width, 'touch targets stay within the viewport');
      }
      assert.equal(await status.getAttribute('aria-live'), 'polite');
    };
    await layout();
    await page.keyboard.press('Tab');
    assert.equal(await check.evaluate(el => el === document.activeElement), true, 'check is keyboard reachable');
    await page.keyboard.press('Enter');
    if (desktop) {
      await status.getByText('Checking for updates…', { exact: true }).waitFor();
      assert.equal(await check.isDisabled(), true);
      await layout();
      await page.evaluate(() => window.testUpdates.pending());
      await status.getByText(/This app is up to date/).waitFor();
      await layout();
      await page.evaluate(() => window.testUpdates.result = 'available');
      await check.click();
      await status.getByText('Checking for updates…', { exact: true }).waitFor();
      await page.evaluate(() => window.testUpdates.pending());
      const download = panel.getByRole('button', { name: 'Download and verify 2.0.0', exact: true });
      await download.waitFor();
      await layout();
      await page.evaluate(() => window.testUpdates.failDownload = true);
      await download.click();
      await panel.getByRole('progressbar', { name: 'Update download' }).waitFor();
      await layout();
      await page.waitForFunction(() => window.testUpdates.calls.includes('hoi_update_download'));
      await page.evaluate(() => window.testUpdates.pending());
      await panel.getByRole('alert').filter({ hasText: 'signature did not verify' }).waitFor();
      await layout();
      assert.equal(await panel.getByRole('button', { name: 'Install and restart' }).count(), 0);
      await page.evaluate(() => { window.testUpdates.failDownload = false; window.testUpdates.pending = null; });
      await download.click();
      await page.waitForFunction(() => window.testUpdates.pending !== null);
      await page.evaluate(() => window.testUpdates.pending());
      await panel.getByRole('button', { name: 'Install and restart' }).waitFor();
      await layout();
      assert.equal(await page.evaluate(() => window.testUpdates.calls.includes('hoi_update_install')), false, 'verified downloads never install automatically');
    } else {
      await status.getByText('Signed release checks are unavailable in this development build.').waitFor();
      await layout();
      await page.evaluate(() => {
        const state = window.testUpdates;
        state.sw.waiting = { postMessage: message => state.calls.push(message.type) };
        state.sw.error = 'The browser update check failed. Your cached app is unchanged; reconnect and retry.';
      });
      await panel.getByRole('button', { name: 'Reload to update' }).waitFor();
      await status.getByText(/cached app is unchanged/).waitFor();
      await layout();
      await panel.getByRole('button', { name: 'Reload to update' }).click();
      assert.deepEqual(await page.evaluate(() => window.testUpdates.calls), ['skip-waiting']);
    }
    await page.addScriptTag({ path: axe });
    assert.deepEqual(await page.evaluate(async () => (await axe.run(document)).violations.filter(v => ['serious', 'critical'].includes(v.impact)).map(v => v.id)), []);
    await panel.screenshot({ path: resolve(shots, `${desktop ? 'desktop' : 'browser'}-${width}.png`) });
    await context.close();
  }
  assert.deepEqual(errors, []);
  console.log('PASS: update layout at 1280/320px, browser reload and errors, desktop update states, keyboard and accessibility.');
} finally {
  await browser?.close();
  await server.close();
}
