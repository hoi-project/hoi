import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';
import { build } from '../apps/web/node_modules/vite/dist/node/index.js';

test('the desktop frontend builds without loading browser workers or generated WASM', async () => {
  const result = await build({
    root: fileURLToPath(new URL('../apps/web/', import.meta.url)),
    mode: 'desktop',
    logLevel: 'silent',
    build: { write: false },
    plugins: [{
      name: 'assert-native-desktop-backend',
      enforce: 'pre',
      load(id) {
        assert.doesNotMatch(id.replaceAll('\\', '/'), /\/core\/(?:worker-core\.ts|core\.worker\.ts|wasm\/)/,
          'a clean desktop checkout must not need browser WASM bindings');
      },
    }],
  });
  const output = (Array.isArray(result) ? result : [result]).flatMap(bundle => bundle.output);
  const modules = output.filter(item => item.type === 'chunk').flatMap(chunk => Object.keys(chunk.modules));
  assert.ok(modules.some(id => id.replaceAll('\\', '/').endsWith('/core/tauri-core.ts')));
  assert.ok(!output.some(item => item.fileName.endsWith('.wasm')));
});
