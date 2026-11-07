import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { buildManifest, classify } from './release-manifest.mjs';
import { findBundles, releaseBundleName } from './tauri-bundles.mjs';

test('the node setup app is marked whichever way its product name was spelled', () => {
  for (const name of [
    'hoi-node-setup_1.0.0_x64-setup.exe', // release-local.mjs and build-all.mjs
    'hoi Node Setup_1.0.0_x64-setup.exe', // straight from tauri build
    'hoi.Node.Setup_1.0.0_x64-setup.exe', // a GitHub release asset uploaded by the workflow
    'hoi-node-setup_1.0.0_windows-x64-portable.exe',
    'hoi Node Setup_1.0.0_aarch64.dmg',
    'hoi-node-setup_1.0.0_amd64.AppImage',
  ]) assert.equal(classify(name)?.product, 'setup', name);
  assert.deepEqual(classify('hoi.Node.Setup_1.0.0_aarch64.dmg'), { platform: 'macos', kind: 'dmg', arch: 'aarch64', product: 'setup' });
});

test('the app and hoi-node are not taken for the setup app', () => {
  assert.deepEqual(classify('hoi_1.0.0_x64-setup.exe'), { platform: 'windows', kind: 'exe', arch: 'x86_64' });
  assert.deepEqual(classify('hoi_1.0.0_amd64.AppImage'), { platform: 'linux', kind: 'AppImage', arch: 'x86_64' });
  assert.deepEqual(classify('hoi-node_1.0.0_aarch64-unknown-linux-gnu.tar.gz'), { platform: 'node', kind: 'tar.gz', arch: 'aarch64' });
  // The server bundle (build-server.mjs) and the website archive are not release-list files.
  assert.equal(classify('hoi-server_1.0.0_linux-x64.tar.gz'), null);
  assert.equal(classify('hoi-website_1.0.0.tar.gz'), null);
});

test('setup bundles get the plain release name; other names are left as they are', () => {
  assert.equal(releaseBundleName('hoi Node Setup_1.0.0-preview.1_x64-setup.exe'), 'hoi-node-setup_1.0.0-preview.1_x64-setup.exe');
  assert.equal(releaseBundleName('hoi.Node.Setup_1.0.0_aarch64.dmg'), 'hoi-node-setup_1.0.0_aarch64.dmg');
  assert.equal(releaseBundleName('hoi_1.0.0_x64-setup.exe'), 'hoi_1.0.0_x64-setup.exe');
  assert.equal(releaseBundleName('hoi-node_1.0.0_x86_64-unknown-linux-gnu.tar.gz'), 'hoi-node_1.0.0_x86_64-unknown-linux-gnu.tar.gz');
});

test('the bundle finder walks a Tauri target folder and skips .app folders', () => {
  const dir = mkdtempSync(join(tmpdir(), 'hoi-bundles-'));
  try {
    for (const p of ['release/bundle/nsis', 'release/bundle/macos/hoi.app/Contents/MacOS', 'release/bundle/deb']) mkdirSync(join(dir, p), { recursive: true });
    for (const p of ['release/bundle/nsis/hoi_1.0.0_x64-setup.exe', 'release/bundle/deb/hoi_1.0.0_amd64.deb', 'release/bundle/macos/hoi.app/Contents/MacOS/hoi.exe', 'release/hoi-desktop.d']) writeFileSync(join(dir, p), '');
    assert.deepEqual(findBundles(dir).map((p) => p.slice(dir.length + 1).replaceAll('\\', '/')).sort(), ['release/bundle/deb/hoi_1.0.0_amd64.deb', 'release/bundle/nsis/hoi_1.0.0_x64-setup.exe']);
    assert.deepEqual(findBundles(join(dir, 'missing')), []);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('the files stay on the GitHub release when the list is served from the site', () => {
  const dir = mkdtempSync(join(tmpdir(), 'hoi-manifest-'));
  const saved = process.env.HOI_RELEASE_BASE_URL;
  delete process.env.HOI_RELEASE_BASE_URL;
  try {
    writeFileSync(join(dir, 'hoi_1.0.0_amd64.AppImage'), 'app');
    const publication = { repository: 'https://github.com/hoi-project/hoi', downloads: 'https://hoi.sh/releases' };
    const [file] = buildManifest(dir, 'v1.0.0', publication).releases[0].files;
    assert.equal(file.url, 'https://github.com/hoi-project/hoi/releases/download/v1.0.0/hoi_1.0.0_amd64.AppImage');
    process.env.HOI_RELEASE_BASE_URL = 'https://files.example/hoi/';
    assert.equal(buildManifest(dir, 'v1.0.0', publication).releases[0].files[0].url, 'https://files.example/hoi/hoi_1.0.0_amd64.AppImage');
  } finally {
    if (saved === undefined) delete process.env.HOI_RELEASE_BASE_URL; else process.env.HOI_RELEASE_BASE_URL = saved;
    rmSync(dir, { recursive: true, force: true });
  }
});
