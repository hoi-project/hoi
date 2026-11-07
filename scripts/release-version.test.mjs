import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { applyVersion, checkVersion, LOCKFILES, planVersion, versionFiles } from './release-version.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
function fixture(t) {
  const base = mkdtempSync(join(tmpdir(), 'hoi-release-version-'));
  t.after(() => rmSync(base, { recursive: true, force: true }));
  const paths = [...new Set([...versionFiles().map(file => file.path), ...LOCKFILES,
    'package.json', 'release.json', 'scripts/release-version.mjs', 'scripts/release-tier.mjs',
    'scripts/release-config.mjs', 'scripts/release-schema.mjs'])];
  for (const path of paths) cpSync(join(root, path), join(base, path), { recursive: true });
  // The suite must keep passing after the real repository is promoted to stable.
  applyVersion(planVersion('0.1.0', { channel: 'preview' }, base), base);
  const snapshot = () => paths.map(path => readFileSync(join(base, path), 'utf8'));
  const cli = (...args) => spawnSync(process.execPath, [join(base, 'scripts/release-version.mjs'), ...args],
    { cwd: base, encoding: 'utf8', env: { ...process.env, GITHUB_OUTPUT: join(base, 'outputs') } });
  return { base, snapshot, cli };
}

test('preview preparation synchronizes manifests and lockfiles without changing dependencies or compatibility', t => {
  const { base, cli } = fixture(t);
  const metadata = readFileSync(join(base, 'release.json'), 'utf8');
  const before = LOCKFILES.map(path => readFileSync(join(base, path), 'utf8'));
  const result = cli('v1.0.0-preview.1');
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(checkVersion('1.0.0-preview.1', base), {
    version: '1.0.0-preview.1', tag: 'v1.0.0-preview.1', tier: 'major', channel: 'preview', prerelease: true,
  });
  assert.equal(readFileSync(join(base, 'release.json'), 'utf8'), metadata);
  const withoutOwnVersions = text => text.replace(/(name = "hoi-[a-z-]+"\r?\n)version = "[^"]*"/g, '$1');
  for (const [i, path] of LOCKFILES.entries()) {
    assert.equal(withoutOwnVersions(readFileSync(join(base, path), 'utf8')), withoutOwnVersions(before[i]));
  }
  assert.equal(planVersion('1.0.0-preview.1', {}, base).changes.length, 0);
});

test('stable promotion and returning to preview update the channel and notice once', t => {
  const { base } = fixture(t);
  const original = JSON.parse(readFileSync(join(base, 'release.json'), 'utf8'));
  const stable = planVersion('1.0.0', {}, base);
  assert.equal(stable.release.revision, original.revision + 1);
  assert.equal(stable.release.notice.title, 'Stable release');
  assert.equal(stable.prerelease, false);
  applyVersion(stable, base);
  assert.equal(planVersion('1.0.0', {}, base).changes.length, 0);
  assert.equal(checkVersion('1.0.0', base).prerelease, false);
  const preview = planVersion('1.1.0-preview.1', {}, base);
  assert.equal(preview.release.channel, 'preview');
  assert.equal(preview.release.revision, original.revision + 2);
  assert.match(preview.release.notice.title, /Preview/);
  assert.throws(() => planVersion('1.1.0-preview.1', { channel: 'stable' }, base), /requires channel preview/);
});

test('explicit compatibility and notice changes increment metadata revision and retain only declared protocols', t => {
  const { base, cli } = fixture(t);
  const original = JSON.parse(readFileSync(join(base, 'release.json'), 'utf8'));
  const args = ['1.0.0-preview.2', '--protocol', '0.4', '--index', '2', '--notice-title', 'Preview 2', '--notice-message', 'Back up before upgrading.'];
  const result = cli(...args);
  assert.equal(result.status, 0, result.stderr);
  const updated = JSON.parse(readFileSync(join(base, 'release.json'), 'utf8'));
  assert.equal(updated.revision, original.revision + 1);
  assert.equal(updated.protocolVersion, '0.4');
  assert.deepEqual(updated.supportedProtocols, ['0.4']);
  assert.equal(updated.indexVersion, 2);
  assert.equal(updated.notice.message, 'Back up before upgrading.');
  assert.equal(cli(...args).status, 0);
  assert.deepEqual(JSON.parse(readFileSync(join(base, 'release.json'), 'utf8')), updated);
  assert.equal(cli('1.0.0-preview.2', '--supported-protocols', '0.3,0.4').status, 0);
  assert.deepEqual(JSON.parse(readFileSync(join(base, 'release.json'), 'utf8')).supportedProtocols, ['0.3', '0.4']);
});

test('dry-run, status and check are read-only; GitHub outputs reflect numeric previews too', t => {
  const { base, snapshot, cli } = fixture(t);
  const before = snapshot();
  assert.equal(cli('1.0.0-preview.1', '--dry-run').status, 0);
  assert.equal(cli().status, 0);
  assert.equal(cli('--check').status, 0);
  assert.deepEqual(snapshot(), before);
  applyVersion(planVersion('0.2.0', {}, base), base);
  const result = cli('--check', 'v0.2.0', '--github-output');
  assert.equal(result.status, 0, result.stderr);
  const outputs = readFileSync(join(base, 'outputs'), 'utf8');
  assert.match(outputs, /^tag=v0.2.0$/m);
  assert.match(outputs, /^tier=minor$/m);
  assert.match(outputs, /^prerelease=true$/m);
});

test('invalid arguments fail before editing any file', t => {
  const { snapshot, cli } = fixture(t);
  const before = snapshot();
  for (const args of [
    ['1.0.0-preview.01'], ['1.0.0\n'], ['1.0.0', '--unknown'],
    ['1.0.0', '--channel', 'bogus'], ['1.0.0-rc.1', '--channel', 'stable'],
    ['1.0.0', '--index', '0'], ['1.0.0', '--index', '2147483648'],
    ['1.0.0', '--protocol', '0.4', '--supported-protocols', '0.3'],
    ['1.0.0', '--notice-title'], ['1.0.0', '--check', '--dry-run'],
    ['1.0.0', '--check', '--channel', 'preview'], ['1.0.0', '--github-output'],
  ]) {
    assert.notEqual(cli(...args).status, 0, args.join(' '));
    assert.deepEqual(snapshot(), before, args.join(' '));
  }
});

test('a malformed late manifest prevents any earlier version writes', t => {
  const { base, snapshot, cli } = fixture(t);
  writeFileSync(join(base, 'apps/desktop/src-tauri/tauri.conf.json'), '{broken');
  const before = snapshot();
  assert.notEqual(cli('1.0.0-preview.1').status, 0);
  assert.deepEqual(snapshot(), before);
});

test('checks catch lockfile drift and a stable channel on a prerelease', t => {
  const { base, cli } = fixture(t);
  applyVersion(planVersion('1.0.0-preview.1', {}, base), base);
  const lock = join(base, LOCKFILES[0]);
  const original = readFileSync(lock, 'utf8');
  writeFileSync(lock, original.replace(/(name = "hoi-[a-z-]+"\r?\n)version = "[^"]*"/, '$1version = "9.0.0"'));
  assert.match(cli('--check').stderr, /Cargo.lock/);
  writeFileSync(lock, original);
  const path = join(base, 'release.json');
  const release = JSON.parse(readFileSync(path, 'utf8'));
  writeFileSync(path, JSON.stringify({ ...release, channel: 'stable' }));
  assert.match(cli('--check').stderr, /requires channel preview/);
});
