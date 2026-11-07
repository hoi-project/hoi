import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { checkFiles, checkLinks, isTrackable } from './check-tracked.mjs';

test('sources, examples and licences may be tracked; credentials, databases, logs and build output may not', () => {
  for (const path of ['README.md', 'LICENSE', 'core/Cargo.lock', 'design/tokens/tokens.json',
    'deploy/home/.env.example', 'vendor/dimpl/LICENSE-MIT.txt', '.github/workflows/release.yml', 'docs/spec/PROTOCOL.md']) {
    assert.ok(isTrackable(path), path);
  }
  for (const path of ['.git/config', '.codex/config.toml', 'apps/web/.env.local', '.env', 'signing.key',
    'cert.pfx', 'node_modules/a.js', 'dist/app.js', 'core/target/a', '../outside', 'a/../b',
    'test-results/log.txt', 'C:outside', 'work/audit.txt', 'apps/web/dist-setup/index.html',
    'apps/web/.shots/you.png', 'coverage/report.json', '.ssh/id_ed25519', '.npmrc', '.netrc', 'id_rsa',
    'hoi.db-wal', 'hoi.db-shm', 'registry.sqlite', 'registry.sqlite3-wal', 'debug.log']) assert.ok(!isTrackable(path), path);
});

test('documentation links resolve to tracked files; link syntax inside code is not a link', () => {
  const temp = mkdtempSync(join(tmpdir(), 'hoi-check-tracked-'));
  try {
    writeFileSync(join(temp, 'README.md'), '[missing](missing.md)');
    assert.throws(() => checkLinks(temp, ['README.md']), /missing/);
    writeFileSync(join(temp, 'local.md'), 'exists locally but is not tracked');
    writeFileSync(join(temp, 'README.md'), '[local](local.md)');
    assert.throws(() => checkLinks(temp, ['README.md']), /untracked/);
    checkLinks(temp, ['README.md', 'local.md']);
    writeFileSync(join(temp, 'README.md'), 'Write `[text](target)`.\n\n```md\n[x](nowhere.md)\n```\n[site](https://hoi.sh) [top](#start)\n');
    checkLinks(temp, ['README.md']);
    writeFileSync(join(temp, 'README.md'), 'Write `[text](target)` or [this](missing.md).');
    assert.throws(() => checkLinks(temp, ['README.md']), /missing/);
  } finally {
    rmSync(temp, { recursive: true, force: true });
  }
});

test('tracked paths are regular files reached without symlinks', { skip: process.platform === 'win32' }, () => {
  const temp = mkdtempSync(join(tmpdir(), 'hoi-check-tracked-symlink-'));
  try {
    mkdirSync(join(temp, 'elsewhere'));
    writeFileSync(join(temp, 'elsewhere', 'note.md'), 'outside');
    symlinkSync(join(temp, 'elsewhere', 'note.md'), join(temp, 'note.md'));
    symlinkSync(join(temp, 'elsewhere'), join(temp, 'docs'));
    for (const path of ['note.md', 'docs/note.md']) assert.throws(() => checkFiles(temp, [path]), /Refusing symlink/);
    assert.throws(() => checkFiles(temp, ['.env']), /Refusing a credential/);
    checkFiles(temp, ['elsewhere/note.md']);
  } finally {
    rmSync(temp, { recursive: true, force: true });
  }
});
