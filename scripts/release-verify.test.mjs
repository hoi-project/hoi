import test from 'node:test';
import assert from 'node:assert/strict';
import { fetchVerifiedManifest, verifyMinisign, parseManifest, compareVersions } from './release-verify.mjs';
import { loadBuildRelease } from './release-config.mjs';
import { validatePublication, configureInstaller, publicationLevel, loadPublication } from './publication.mjs';
import { checkCandidate } from './release-check.mjs';
import { mkdtempSync, readFileSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { signer } from './fixtures/minisign-test-signer.mjs';
const manifest = () => ({ schema: 1, latest: '2.0.0', release: loadBuildRelease(), updater: { version: '2.0.0', platforms: {} }, releases: [{ version: '2.0.0', date: '2026-11-06', files: [{ platform: 'linux', kind: 'AppImage', arch: 'x86_64', name: 'hoi.AppImage', url: 'https://releases.invalid/hoi.AppImage', size: 3, sha256: 'a'.repeat(64) }] }] });
const encode = m => Buffer.from(JSON.stringify(m));

test('minisign verifies exact bytes, comment and key using an isolated signing key', () => {
  const s = signer(), bytes = encode(manifest()), sig = s.sign(bytes);
  verifyMinisign(bytes, sig, s.key);
  assert.throws(() => verifyMinisign(Buffer.concat([bytes, Buffer.from(' ')]), sig, s.key), /verify/);
  assert.throws(() => verifyMinisign(bytes, sig.replace('version:2.0.0', 'version:9.0.0'), s.key), /verify/);
  assert.throws(() => verifyMinisign(bytes, sig, signer().key), /verify/);
  assert.throws(() => verifyMinisign(bytes, '', s.key));
});

test('fetch authenticates the manifest before URLs/notices; rejects interruption and oversized responses', async () => {
  const s = signer(), bytes = encode(manifest()), config = { downloads: 'https://releases.invalid', manifestKey: s.key };
  const fetcher = async url => new Response(url.endsWith('.minisig') ? s.sign(bytes) : bytes);
  assert.equal((await fetchVerifiedManifest(config, '1.0.0', fetcher)).latest, '2.0.0');
  await assert.rejects(fetchVerifiedManifest(config, '3.0.0', fetcher), /older/);
  await assert.rejects(fetchVerifiedManifest(config, null, async () => new Response('bad')), /signature/);
  await assert.rejects(fetchVerifiedManifest(config, null, async () => new Response(new Uint8Array(1024 * 1024 + 1))), /large/);
  await assert.rejects(fetchVerifiedManifest(config, null, async () => { throw new Error('interrupted'); }), /interrupted/);
  await assert.rejects(fetchVerifiedManifest({ downloads: '', manifestKey: '' }), /development/);
});

test('manifest validation rejects unsafe artifact addresses, ambiguity and rollback', () => {
  for (const patch of [{ url: 'http://releases.invalid/file' }, { url: 'https://user:password@releases.invalid/file' }, { product: 'wallet' }, { size: -1 }, { name: '../file' }, { sha256: 'bad' }]) {
    const m = manifest(); Object.assign(m.releases[0].files[0], patch);
    assert.throws(() => parseManifest(encode(m)));
  }
  const m = manifest(); m.releases[0].files.push(m.releases[0].files[0]);
  assert.throws(() => parseManifest(encode(m)));
  assert.throws(() => parseManifest(encode(manifest()), '2.0.1'), /older/);
});

test('semver precedence includes prerelease numeric identifiers and ignores build metadata', () => {
  const versions = ['1.0.0-alpha', '1.0.0-alpha.1', '1.0.0-alpha.beta', '1.0.0-beta', '1.0.0-beta.2', '1.0.0-beta.11', '1.0.0-rc.1', '1.0.0', '1.0.1', '2.0.0'];
  for (let i = 1; i < versions.length; i++) assert.equal(compareVersions(versions[i - 1], versions[i]), -1);
  assert.equal(compareVersions('1.0.0+one', '1.0.0+two'), 0);
  for (const invalid of ['1.0', 'v1.0.0', '01.0.0', '1.0.0-01']) assert.throws(() => compareVersions(invalid, '1.0.0'));
});

test('publication rejects unconfigured builds and configures both installer trust roots', () => {
  const s = signer();
  const cfg = { schema: 1, repository: 'https://github.com/publisher/hoi', site: 'https://hoi.invalid', downloads: 'https://hoi.invalid/releases', contact: 'publisher contact', manifestKey: s.key, updaterKey: s.key };
  validatePublication(cfg, true);
  for (const field of ['repository', 'site', 'downloads', 'contact', 'manifestKey', 'updaterKey']) assert.throws(() => validatePublication({ ...cfg, [field]: '' }, true), /required/);
  assert.throws(() => validatePublication({ ...cfg, site: '', contact: '  ', manifestKey: '', updaterKey: '' }, true), /required before publication: site, contact, manifestKey, updaterKey/);
  assert.throws(() => validatePublication({ ...cfg, contact: '' }, 'release'), /required before publication: contact/);
  // A website deployment (a preview included) needs only its repository and its address.
  const preview = { ...cfg, downloads: '', contact: '', manifestKey: '', updaterKey: '' };
  validatePublication(preview, 'site');
  assert.throws(() => validatePublication(preview, true), /required before publication: downloads, contact, manifestKey, updaterKey/);
  assert.throws(() => validatePublication({ ...preview, site: '' }, 'site'), /required before deploying the website: site/);
  assert.throws(() => validatePublication({ ...preview, repository: 'https://github.com/OWNER/hoi' }, 'site'), /deploying the website: repository/);
  // Downloads are offered only with the key that verifies them.
  assert.throws(() => validatePublication({ ...preview, downloads: cfg.downloads }, 'site'), /downloads and manifestKey are set together/);
  assert.throws(() => validatePublication({ ...preview, manifestKey: cfg.manifestKey }, 'site'), /downloads and manifestKey are set together/);
  validatePublication({ ...preview, downloads: cfg.downloads, manifestKey: cfg.manifestKey }, 'site');
  assert.throws(() => validatePublication(cfg, 'stable'), /Unknown publication level/);
  assert.equal(publicationLevel(undefined), undefined);
  assert.equal(publicationLevel(''), undefined);
  assert.equal(publicationLevel('1'), 'release');
  assert.equal(publicationLevel('release'), 'release');
  assert.equal(publicationLevel('site'), 'site');
  assert.throws(() => publicationLevel('true'), /HOI_PUBLICATION must be/);
  const sh = configureInstaller('DEFAULT_MANIFEST=old\nRELEASE_PUBKEY=""', cfg);
  assert.ok(sh.includes(s.key)); assert.ok(sh.includes('https://hoi.invalid/releases/manifest.json'));
  assert.ok(configureInstaller("[string]$PubKey = ''", cfg).includes(s.key));
});

test('the cross-language candidate fixture verifies offline, then rejects a changed artifact', () => {
  const fixture = JSON.parse(readFileSync(new URL('./fixtures/release-signed.json', import.meta.url), 'utf8'));
  const dir = mkdtempSync(join(tmpdir(), 'hoi-candidate-'));
  try {
    writeFileSync(join(dir, 'manifest.json'), fixture.manifest);
    writeFileSync(join(dir, 'manifest.json.minisig'), fixture.signature);
    const file = JSON.parse(fixture.manifest).releases[0].files[0].name;
    writeFileSync(join(dir, file), fixture.artifact);
    const config = { manifestKey: fixture.key, updaterKey: fixture.key };
    assert.equal(checkCandidate(dir, config).latest, '2.0.0');
    const signerKey = signer();
    for (const target of [null, 'linux-x86_64-nsis']) {
      const m = JSON.parse(fixture.manifest);
      const entry = m.updater.platforms['linux-x86_64'];
      m.updater.platforms = target ? { [target]: entry } : {};
      const bytes = encode(m);
      writeFileSync(join(dir, 'manifest.json'), bytes);
      writeFileSync(join(dir, 'manifest.json.minisig'), signerKey.sign(bytes));
      assert.throws(() => checkCandidate(dir, { ...config, manifestKey: signerKey.key }), target ? /Wrong updater/ : /missing its signed updater/);
    }
    writeFileSync(join(dir, 'manifest.json'), fixture.manifest);
    writeFileSync(join(dir, 'manifest.json.minisig'), fixture.signature);
    writeFileSync(join(dir, file), 'changed');
    assert.throws(() => checkCandidate(dir, config), /checksum/);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test('a test publication file is read only for test builds, never for a website or release build', () => {
  const dir = mkdtempSync(join(tmpdir(), 'hoi-publication-'));
  const file = join(dir, 'publication.json');
  const s = signer();
  writeFileSync(file, JSON.stringify({ schema: 1, repository: 'https://github.com/hoi-project/hoi', site: 'https://hoi.sh', downloads: 'https://downloads.invalid/releases', contact: '', manifestKey: s.key, updaterKey: '' }));
  const previous = process.env.HOI_PUBLICATION_FILE;
  process.env.HOI_PUBLICATION_FILE = file;
  try {
    assert.equal(loadPublication(undefined).manifestKey, s.key);
    for (const level of ['site', 'release', true]) assert.throws(() => loadPublication(level), /test builds only/, String(level));
  } finally {
    if (previous === undefined) delete process.env.HOI_PUBLICATION_FILE; else process.env.HOI_PUBLICATION_FILE = previous;
    rmSync(dir, { recursive: true, force: true });
  }
  assert.notEqual(loadPublication(undefined).manifestKey, s.key, 'without the variable, publication.json');
});
