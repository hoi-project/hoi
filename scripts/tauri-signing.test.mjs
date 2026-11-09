import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { clearEmptyAppleSigningEnv } from './tauri-signing.mjs';

const appleNames = ['APPLE_CERTIFICATE', 'APPLE_CERTIFICATE_PASSWORD', 'APPLE_SIGNING_IDENTITY',
  'APPLE_ID', 'APPLE_PASSWORD', 'APPLE_TEAM_ID'];

test('missing Apple secrets do not request certificate import or notarization', () => {
  const env = Object.fromEntries(appleNames.map(name => [name, '']));
  env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD = '';
  clearEmptyAppleSigningEnv(env);
  assert.deepEqual(env, { TAURI_SIGNING_PRIVATE_KEY_PASSWORD: '' });
});

test('configured Apple credentials, including an empty certificate password, are preserved', () => {
  const env = Object.fromEntries(appleNames.map(name => [name, `configured-${name}`]));
  env.APPLE_CERTIFICATE_PASSWORD = '';
  const before = { ...env };
  clearEmptyAppleSigningEnv(env);
  assert.deepEqual(env, before);
  // Nonempty malformed inputs must still reach Tauri's validator instead of disabling signing.
  for (const certificate of ['not-base64', ' ']) {
    const invalid = { APPLE_CERTIFICATE: certificate, APPLE_CERTIFICATE_PASSWORD: '' };
    clearEmptyAppleSigningEnv(invalid);
    assert.deepEqual(invalid, { APPLE_CERTIFICATE: certificate, APPLE_CERTIFICATE_PASSWORD: '' });
  }
});

test('both application wrappers clear absent credentials before invoking Tauri', t => {
  const base = mkdtempSync(join(tmpdir(), 'hoi-tauri-signing-'));
  t.after(() => rmSync(base, { recursive: true, force: true }));
  for (const path of ['scripts/tauri-signing.mjs', 'scripts/tauri-bundles.mjs', 'scripts/relay-config.mjs',
    'scripts/publication.mjs', 'scripts/release-tier.mjs',
    ...['desktop', 'setup'].flatMap(app => [`apps/${app}/scripts/tauri.mjs`, `apps/${app}/src-tauri/tauri.conf.json`])]) {
    cpSync(new URL(`../${path}`, import.meta.url), join(base, path));
  }
  writeFileSync(join(base, 'publication.json'), JSON.stringify({ schema: 1,
    repository: '', site: '', downloads: '', contact: '', manifestKey: '', updaterKey: '' }));
  const cli = join(base, 'node_modules/@tauri-apps/cli');
  mkdirSync(cli, { recursive: true });
  writeFileSync(join(cli, 'package.json'), JSON.stringify({ type: 'module', exports: './index.js' }));
  writeFileSync(join(cli, 'index.js'), `import { writeFileSync } from 'node:fs';
  export function run() {
    // Tauri uses var_os: present-but-empty credentials take the signing/notarization branches.
    if (process.env.APPLE_CERTIFICATE === '' || process.env.APPLE_SIGNING_IDENTITY === ''
      || process.env.APPLE_ID === '') throw new Error('Empty Apple signing credentials reached Tauri.');
    writeFileSync('tauri-env.json', JSON.stringify(Object.fromEntries(${JSON.stringify(appleNames)}
      .filter(name => process.env[name] !== undefined).map(name => [name, process.env[name]]))));
  }`);
  for (const app of ['desktop', 'setup']) {
    for (const credentials of [Object.fromEntries(appleNames.map(name => [name, ''])),
      { APPLE_CERTIFICATE: 'configured-certificate', APPLE_CERTIFICATE_PASSWORD: '', APPLE_SIGNING_IDENTITY: '-' }]) {
      const result = spawnSync(process.execPath, [join(base, `apps/${app}/scripts/tauri.mjs`), 'build'], {
        cwd: join(base, `apps/${app}`), encoding: 'utf8', env: { PATH: process.env.PATH, ...credentials },
      });
      assert.equal(result.status, 0, `${app}: ${result.stderr}`);
      assert.deepEqual(JSON.parse(readFileSync(join(base, `apps/${app}/tauri-env.json`), 'utf8')),
        credentials.APPLE_CERTIFICATE ? credentials : {});
    }
  }
});
