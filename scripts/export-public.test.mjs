import assert from 'node:assert/strict';
import { existsSync, mkdtempSync, mkdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { checkPublicLinks, copySnapshot, isPublicPath, validatePublicFiles } from './export-public.mjs';

test('public export keeps build inputs and excludes private/local material', () => {
  for (const path of ['README.md', 'LICENSE', 'core/Cargo.lock', 'design/tokens/tokens.json',
    'deploy/home/.env.example', 'vendor/dimpl/LICENSE-MIT.txt', '.github/workflows/release.yml']) {
    assert.ok(isPublicPath(path), path);
  }
  for (const path of ['.private/docs/ROADMAP.md', '.private\\docs\\DECISIONS.md',
    '.git/config', '.codex/config.toml', 'apps/web/.env.local', '.env', 'signing.key',
    'cert.pfx', 'node_modules/a.js', 'dist/app.js', 'core/target/a', '../outside', 'a/../b',
    'public-export/README.md', 'test-results/log.txt', 'docs/ROADMAP.md',
    'docs/PROTOCOL.md', 'docs/spec/DECISIONS.md', 'design/canvas/Main.dc.html', 'C:outside', 'work/audit.txt',
    'apps/web/dist-setup/index.html', 'apps/web/.shots/you.png', 'coverage/report.json',
    '.ssh/id_ed25519', '.npmrc', '.netrc', 'id_rsa', 'hoi.db-wal', 'hoi.db-shm',
    'registry.sqlite', 'registry.sqlite3-wal']) assert.ok(!isPublicPath(path), path);
});

test('snapshot copies only selected files and refuses to overwrite or copy unsafe paths', () => {
  const temp = mkdtempSync(join(tmpdir(), 'hoi-public-export-'));
  try {
    const source = join(temp, 'source');
    const output = join(temp, 'output');
    mkdirSync(join(source, '.private'), { recursive: true });
    writeFileSync(join(source, 'README.md'), 'public');
    writeFileSync(join(source, '.private', 'plan.md'), 'private');
    writeFileSync(join(source, 'README.md'), '[plan](.private/plan.md)');
    assert.throws(() => checkPublicLinks(source, ['README.md']), /private source/);
    writeFileSync(join(source, 'README.md'), '[missing](missing.md)');
    assert.throws(() => checkPublicLinks(source, ['README.md']), /missing/);
    writeFileSync(join(source, 'local.md'), 'exists locally but is not part of the snapshot');
    writeFileSync(join(source, 'README.md'), '[local](local.md)');
    assert.throws(() => checkPublicLinks(source, ['README.md']), /missing/);
    checkPublicLinks(source, ['README.md', 'local.md']);
    // Link syntax shown in code is not a link; a real link next to it still is.
    writeFileSync(join(source, 'README.md'), 'Write `[text](target)`.\n\n```md\n[x](nowhere.md)\n```\n');
    checkPublicLinks(source, ['README.md']);
    writeFileSync(join(source, 'README.md'), 'Write `[text](target)` or [this](missing.md).');
    assert.throws(() => checkPublicLinks(source, ['README.md']), /missing/);
    writeFileSync(join(source, 'README.md'), 'public');
    assert.throws(() => copySnapshot(source, output, ['.private/plan.md']), /Refusing/);
    assert.ok(!existsSync(output));
    copySnapshot(source, output, ['README.md']);
    assert.equal(readFileSync(join(output, 'README.md'), 'utf8'), 'public');
    assert.ok(!existsSync(join(output, '.private')));
    assert.ok(!existsSync(join(output, '.git')));
    assert.throws(() => copySnapshot(source, output, ['README.md']), /already exists/);
  } finally {
    rmSync(temp, { recursive: true, force: true });
  }
});

test('checks reject symlink files and directories before an export is written', { skip: process.platform === 'win32' }, () => {
  const temp = mkdtempSync(join(tmpdir(), 'hoi-public-symlink-'));
  try {
    mkdirSync(join(temp, '.private'));
    writeFileSync(join(temp, '.private', 'note.md'), 'private');
    symlinkSync(join(temp, '.private', 'note.md'), join(temp, 'note.md'));
    symlinkSync(join(temp, '.private'), join(temp, 'docs'));
    for (const path of ['note.md', 'docs/note.md']) {
      assert.throws(() => validatePublicFiles(temp, [path]), /Refusing symlink/);
      assert.throws(() => copySnapshot(temp, join(temp, 'output'), [path]), /Refusing symlink/);
      assert.ok(!existsSync(join(temp, 'output')));
    }
  } finally {
    rmSync(temp, { recursive: true, force: true });
  }
});
