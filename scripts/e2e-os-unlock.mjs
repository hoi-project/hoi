// Desktop UI contract with a stubbed Tauri boundary. Native keystore/database tests live in
// src-tauri; this exercises autofill, cancellation and retry without touching a real keychain.
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { chromium } from 'playwright';
import { createServer } from '../apps/web/node_modules/vite/dist/node/index.js';

const root = resolve(import.meta.dirname, '..');
const axe = createRequire(import.meta.url).resolve('axe-core/axe.min.js');
process.env.VITE_HOI_BACKEND = 'wasm';
const server = await createServer({ root: resolve(root, 'apps/web'), logLevel: 'error', server: { host: '127.0.0.1', port: 0, hmr: false }, plugins: [{
  name: 'os-unlock-test-page',
  configureServer(server) {
    server.middlewares.use('/os-unlock-test', async (req, res) => {
      res.setHeader('Content-Type', 'text/html');
      res.end(await server.transformIndexHtml('/os-unlock-test', `<!doctype html><html lang="en"><title>OS unlock test</title>
        <main id="fixture" class="gutter"><h1>Device settings</h1><div id="component"></div></main>
        <script type="module">
          import { mount } from 'svelte';
          import OsUnlock from '/src/lib/ui/OsUnlock.svelte';
          import Unlock from '/src/routes/Unlock.svelte';
          import Updates from '/src/lib/ui/Updates.svelte';
          import { app } from '/src/lib/app.svelte.ts';
          import '@hoi/tokens/fonts.css';
          import '@hoi/tokens/tokens.css';
          import '/src/app.css';
          app.vaultState = 'locked';
          if (location.search === '?locked') document.querySelector('#fixture').replaceWith(document.createElement('div'));
          mount(location.search === '?updates' ? Updates : location.search === '?locked' ? Unlock : OsUnlock, { target: document.querySelector('#component') || document.body });
        </script></html>`));
    });
  },
}] });
await server.listen();
const origin = `http://127.0.0.1:${server.httpServer.address().port}`;
const browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
const errors = [];
try {
  const context = await browser.newContext({ reducedMotion: 'reduce', viewport: { width: 390, height: 844 } });
  await context.addInitScript(() => {
    window.testOs = { enabled: location.search === '?locked', configured: location.search === '?locked', fail: true, calls: [] };
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
    window.__TAURI_INTERNALS__ = { transformCallback: () => 42, invoke: async (command, args) => {
      const state = window.testOs;
      state.calls.push(command);
      if (command.startsWith('plugin:event|')) return 1;
      if (command === 'hoi_update_check') return { state: 'available', version: '2.0.0', message: 'A signed release is available.' };
      if (command === 'hoi_update_download') { if (state.fail) throw 'The update signature did not verify.'; return; }
      if (command === 'hoi_update_install') throw 'Installation test stopped before replacing any files.';
      if (command === 'hoi_os_unlock_status') return { enabled: state.enabled, configured: state.configured };
      if (state.fail) throw { code: 'internal', message: 'Keystore cancelled. Use your vault password or retry.' };
      if (command === 'hoi_os_unlock_enable') {
        if (args.passphrase !== 'password supplied by manager') throw new Error('The form lost the autofilled password.');
        state.enabled = state.configured = true;
      } else if (command === 'hoi_os_unlock_disable') state.enabled = state.configured = false;
      else throw new Error(`Unexpected native command: ${command}`);
    } };
  });
  const page = await context.newPage();
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(`${origin}/os-unlock-test`);
  const enable = page.getByRole('button', { name: 'Enable OS unlock', exact: true });
  await enable.waitFor();
  assert.deepEqual(await page.evaluate(() => window.testOs.calls), ['hoi_os_unlock_status'], 'settings never reads the keystore or unlocks automatically');
  const submit = () => page.evaluate(() => {
    const form = document.querySelector('#enable-os-unlock');
    form.elements.namedItem('password').value = 'password supplied by manager';
    form.requestSubmit();
  });
  await submit();
  await page.getByRole('alert').filter({ hasText: 'cancelled' }).waitFor();
  assert.equal(await page.locator('#os-password').inputValue(), '');
  await page.evaluate(() => window.testOs.fail = false);
  await submit();
  const remove = page.getByRole('button', { name: 'Remove OS unlock', exact: true });
  await remove.waitFor();
  assert.equal(await page.locator('#os-password').count(), 0);
  await page.addScriptTag({ path: axe });
  assert.deepEqual(await page.evaluate(async () => (await axe.run(document)).violations.filter(v => ['serious', 'critical'].includes(v.impact)).map(v => v.id)), []);
  await page.evaluate(() => window.testOs.fail = true);
  await remove.click();
  await page.getByRole('alert').filter({ hasText: 'cancelled' }).waitFor();
  assert.equal(await remove.isVisible(), true, 'failed removal remains retryable');
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await page.evaluate(() => { window.testOs.fail = false; document.documentElement.dataset.motion = 'reduced'; });
  await remove.hover();
  assert.equal(await remove.evaluate(el => getComputedStyle(el).transform), 'none', 'the app motion setting also prevents hover movement');
  await remove.click();
  await enable.waitFor();
  await page.goto(`${origin}/os-unlock-test?locked`);
  await page.getByRole('button', { name: 'Unlock with this OS account', exact: true }).waitFor();
  assert.equal(await page.evaluate(() => window.testOs.calls.includes('hoi_os_unlock')), false);
  await page.getByRole('button', { name: 'Unlock with this OS account', exact: true }).click();
  await page.getByRole('alert').filter({ hasText: 'cancelled' }).waitFor();
  assert.equal(await page.getByRole('button', { name: 'Unlock', exact: true }).isEnabled(), true, 'password fallback stays available');
  assert.equal(await page.locator('#vault-passphrase').getAttribute('autocomplete'), 'current-password');
  await page.goto(`${origin}/os-unlock-test?updates`);
  await page.getByRole('button', { name: 'Check for updates', exact: true }).waitFor();
  assert.deepEqual(await page.evaluate(() => window.testOs.calls), [], 'updates require an explicit check');
  await page.getByRole('button', { name: 'Check for updates', exact: true }).click();
  await page.getByRole('button', { name: 'Download and verify 2.0.0', exact: true }).click();
  await page.getByRole('alert').filter({ hasText: 'signature did not verify' }).waitFor();
  assert.equal(await page.getByRole('button', { name: 'Install and restart' }).count(), 0);
  await page.evaluate(() => window.testOs.fail = false);
  await page.getByRole('button', { name: 'Download and verify 2.0.0', exact: true }).click();
  await page.getByRole('button', { name: 'Install and restart' }).waitFor();
  assert.equal(await page.evaluate(() => window.testOs.calls.includes('hoi_update_install')), false, 'a verified download does not install itself');
  await page.addScriptTag({ path: axe });
  assert.deepEqual(await page.evaluate(async () => (await axe.run(document)).violations.filter(v => ['serious', 'critical'].includes(v.impact)).map(v => v.id)), []);
  await page.getByRole('button', { name: 'Install and restart' }).click();
  await page.getByRole('alert').filter({ hasText: 'stopped before replacing' }).waitFor();
  assert.deepEqual(errors, []);
  await context.close();
  console.log('PASS: desktop OS unlock and verified update controls, explicit install, failure/retry and accessibility (stubbed native boundary).');
} finally {
  await browser.close();
  await server.close();
}
