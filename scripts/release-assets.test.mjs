import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { transferAssets } from './release-assets.mjs';
import { buildManifest } from './release-manifest.mjs';
import { checkCandidate } from './release-check.mjs';
import { signer } from './fixtures/minisign-test-signer.mjs';

const env = { GH_REPO: 'example/hoi', RELEASE_ID: '42', TAG: 'v1.0.0-preview.2', SOURCE_SHA: 'a'.repeat(40), GH_TOKEN: 'test-token' };
const draft = () => ({ id: 42, draft: true, tag_name: 'untagged-abcdef', name: `hoi ${env.TAG}`,
  body: `Release notes\n\n<!-- hoi-release-source: ${env.SOURCE_SHA} -->` });
const json = value => new Response(JSON.stringify(value), { headers: { 'Content-Type': 'application/json' } });

function fixture(t, release = draft()) {
  const dir = mkdtempSync(join(tmpdir(), 'hoi-release-assets-'));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const state = { release, sha: env.SOURCE_SHA, assets: [], calls: [], uploaded: [], downloaded: [], deleted: [] };
  const fetcher = async (url, options) => {
    const path = new URL(url).pathname;
    state.calls.push({ url, method: options.method ?? 'GET' });
    assert.equal(options.headers.Authorization, 'Bearer test-token');
    if (path === '/repos/example/hoi/releases/42' && !options.method) return json(state.release);
    if (path === `/repos/example/hoi/commits/${env.TAG}`) return json({ sha: state.sha });
    if (path === '/repos/example/hoi/releases/42/assets' && !options.method) {
      const page = Number(new URL(url).searchParams.get('page'));
      return json(state.assets.slice((page - 1) * 100, page * 100));
    }
    const asset = state.assets.find(asset => path === `/repos/example/hoi/releases/assets/${asset.id}`);
    if (asset && options.method === 'DELETE') { state.deleted.push(asset.id); return new Response(null, { status: 204 }); }
    if (asset && options.headers.Accept === 'application/octet-stream') {
      state.downloaded.push(asset.name);
      return new Response(asset.bytes ?? new Uint8Array([0, 255, 1]));
    }
    if (url.startsWith('https://uploads.github.com/repos/example/hoi/releases/42/assets?') && options.method === 'POST') {
      const bytes = [];
      for await (const chunk of options.body) bytes.push(chunk);
      state.uploaded.push({ name: new URL(url).searchParams.get('name'), bytes: Buffer.concat(bytes) });
      assert.equal(String(Buffer.concat(bytes).length), options.headers['Content-Length']);
      return json({ id: 999 });
    }
    // Tag-addressed draft APIs cannot resolve the pending tag.
    return new Response('release not found', { status: 404 });
  };
  return { dir, state, fetcher };
}

test('node archives and manifest files upload into the verified draft ID despite its pending tag', async t => {
  const { dir, state, fetcher } = fixture(t);
  const names = ['hoi-node_1.0.0-preview.2_x86_64-unknown-linux-gnu.tar.gz', 'manifest.json', 'manifest.json.minisig', 'install.sh', 'install.ps1'];
  const bytes = Buffer.from([0, 127, 255]);
  const paths = names.map(name => { const path = join(dir, name); writeFileSync(path, bytes); return path; });
  state.assets = [{ id: 4, name: 'manifest.json' }, { id: 5, name: 'keep.exe' }];
  await transferAssets('upload', paths, env, fetcher);
  assert.deepEqual(state.uploaded, names.map(name => ({ name, bytes })));
  assert.deepEqual(state.deleted, [4]);
  assert.ok(state.calls.every(call => !call.url.includes('/tags/')));
});

test('manifest download enumerates every draft asset page and preserves binary bytes', async t => {
  const { dir, state, fetcher } = fixture(t);
  state.assets = Array.from({ length: 101 }, (_, i) => ({ id: i + 1, name: `artifact-${i}.bin` }));
  writeFileSync(join(dir, 'artifact-0.bin'), 'existing');
  await transferAssets('download', [dir], env, fetcher);
  assert.equal(state.downloaded.length, 100);
  assert.equal(readFileSync(join(dir, 'artifact-0.bin'), 'utf8'), 'existing');
  assert.deepEqual(readFileSync(join(dir, 'artifact-100.bin')), Buffer.from([0, 255, 1]));
  assert.equal(readdirSync(dir).length, 101);
  assert.ok(state.calls.some(call => call.url.endsWith('page=2')));
});

test('draft transfers reject wrong IDs, published releases, tags and sources before changing assets', async t => {
  const invalid = [
    { id: 43 }, { draft: false }, { tag_name: 'v2.0.0' }, { name: 'hoi v2.0.0' },
    { body: '' }, { body: `<!-- hoi-release-source: ${'b'.repeat(40)} -->` },
    { body: `${draft().body}\n<!-- hoi-release-source: ${env.SOURCE_SHA} -->` },
  ];
  for (const changes of invalid) {
    const { dir, state, fetcher } = fixture(t, { ...draft(), ...changes });
    await assert.rejects(transferAssets('download', [dir], env, fetcher), /draft ID, tag and source/);
    assert.equal(state.calls.length, 1);
    assert.deepEqual(readdirSync(dir), []);
  }
  const { dir, state, fetcher } = fixture(t, { ...draft(), tag_name: env.TAG });
  state.sha = 'b'.repeat(40);
  await assert.rejects(transferAssets('download', [dir], env, fetcher), /tag no longer points/);
  assert.equal(state.calls.length, 2);
});

test('tagged drafts work and malformed asset names cannot escape the destination', async t => {
  const { dir, state, fetcher } = fixture(t, { ...draft(), tag_name: env.TAG });
  await transferAssets('download', [dir], env, fetcher);
  for (const name of ['../escape', '..\\escape', '.', '..', 'bad\nname']) {
    state.assets = [{ id: 1, name }];
    await assert.rejects(transferAssets('download', [dir], env, fetcher), /Invalid release asset listing/);
    assert.deepEqual(readdirSync(dir), []);
  }
});

test('HTTP errors fail the transfer rather than creating a successful empty download', async t => {
  const { dir, fetcher } = fixture(t);
  await assert.rejects(transferAssets('download', [dir], env, async (url, options) =>
    url.includes('/assets?') ? new Response(null, { status: 403 }) : fetcher(url, options)), /HTTP 403/);
  assert.deepEqual(readdirSync(dir), []);
});

test('preview assets complete download, manifest generation, signing, verification and upload', async t => {
  const { dir, state, fetcher } = fixture(t);
  const version = env.TAG.slice(1);
  const key = signer(version);
  const names = [
    `hoi_${version}_amd64.AppImage`, `hoi_${version}_amd64.deb`, `hoi-${version}-1.x86_64.rpm`,
    `hoi_${version}_universal.app.tar.gz`, `hoi_${version}_universal.dmg`,
    `hoi_${version}_x64-setup.exe`, `hoi.Node.Setup_${version}_x64-setup.exe`,
    `hoi-node_${version}_aarch64-unknown-linux-gnu.tar.gz`,
  ];
  state.assets = names.map((name, i) => ({ id: i + 1, name, bytes: Buffer.from(`fixture: ${name}`) }));
  for (const asset of [...state.assets].filter(asset => /AppImage$|app\.tar\.gz$|^hoi_.*-setup\.exe$/.test(asset.name))) {
    state.assets.push({ id: state.assets.length + 1, name: `${asset.name}.sig`, bytes: Buffer.from(key.sign(asset.bytes)).toString('base64') });
  }
  await transferAssets('download', [dir], env, fetcher);
  const publication = { repository: 'https://github.com/example/hoi', manifestKey: key.key, updaterKey: key.key };
  const release = { schemaVersion: 1, revision: 1, channel: 'preview', protocolVersion: '0.3', indexVersion: 1,
    supportedProtocols: ['0.3'], notice: { title: 'Preview', message: 'Preview software.' } };
  const manifest = buildManifest(dir, env.TAG, publication, release);
  const bytes = Buffer.from(JSON.stringify(manifest));
  writeFileSync(join(dir, 'manifest.json'), bytes);
  writeFileSync(join(dir, 'manifest.json.minisig'), key.sign(bytes));
  const verified = checkCandidate(dir, publication);
  assert.equal(verified.latest, version);
  assert.equal(verified.releases[0].files.length, names.length);
  assert.equal(verified.updater.version, version);
  assert.deepEqual(Object.keys(verified.updater.platforms).sort(), ['darwin-aarch64', 'darwin-x86_64', 'linux-x86_64', 'windows-x86_64-nsis']);
  await transferAssets('upload', [join(dir, 'manifest.json'), join(dir, 'manifest.json.minisig')], env, fetcher);
  assert.deepEqual(state.uploaded.map(asset => asset.name), ['manifest.json', 'manifest.json.minisig']);
  assert.equal(JSON.parse(state.uploaded[0].bytes).latest, version);
});

test('node and manifest jobs pass the prepared draft ID to tooling from the workflow revision', () => {
  const workflow = readFileSync(new URL('../.github/workflows/release.yml', import.meta.url), 'utf8');
  for (const name of ['node', 'manifest']) {
    const job = workflow.split(`\n  ${name}:\n`)[1].split(/\n  [\w-]+:\n/)[0];
    assert.match(job, /needs: \[[^\n]*draft/);
    assert.match(job, /RELEASE_ID: \$\{\{ needs\.draft\.outputs\.id \}\}/);
    assert.match(job, /SOURCE_SHA: \$\{\{ needs\.plan\.outputs\.sha \}\}/);
    assert.match(job, /ref: \$\{\{ github\.sha \}\}\n          path: release-tools\n          sparse-checkout: scripts\/release-assets\.mjs/);
    assert.match(job, /node release-tools\/scripts\/release-assets\.mjs upload/);
    assert.doesNotMatch(job, /gh release (upload|download)/);
    if (name === 'manifest') assert.match(job, /node release-tools\/scripts\/release-assets\.mjs download out/);
  }
});
