import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';
import { build } from '../apps/web/node_modules/vite/dist/node/index.js';

test('the node setup app window builds without the core, its worker, WASM or a service worker', async () => {
  const result = await build({
    root: fileURLToPath(new URL('../apps/web/setup/', import.meta.url)),
    configFile: fileURLToPath(new URL('../apps/web/vite.config.ts', import.meta.url)),
    mode: 'setup',
    logLevel: 'silent',
    build: { write: false },
    plugins: [{
      name: 'assert-no-core-in-setup',
      enforce: 'pre',
      load(id) {
        assert.doesNotMatch(id.replaceAll('\\', '/'), /\/lib\/core\/(?:worker-core\.ts|core\.worker\.ts|tauri-core\.ts|wasm\/)|\/lib\/app\.svelte\.ts$/,
          'the setup window must not load the client core');
      },
    }],
  });
  const output = (Array.isArray(result) ? result : [result]).flatMap(bundle => bundle.output);
  const modules = output.filter(item => item.type === 'chunk').flatMap(chunk => Object.keys(chunk.modules));
  const normalized = modules.map(id => id.replaceAll('\\', '/'));
  assert.ok(normalized.some(id => id.endsWith('/src/setup/Setup.svelte')), 'the wizard is the page');
  assert.ok(normalized.some(id => id.endsWith('/deploy/install/install.sh?raw')), 'the installer ships inside the window');
  assert.ok(!output.some(item => item.fileName.endsWith('.wasm') || item.fileName === 'sw.js' || item.fileName === '_headers'));
  const html = output.find(item => item.fileName === 'index.html');
  assert.ok(html, 'index.html');
  assert.doesNotMatch(html.source, /Content-Security-Policy/, 'Tauri sets the CSP (tauri.conf.json), not a meta tag');
});
