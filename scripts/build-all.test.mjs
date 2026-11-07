import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { listFiles, parseArgs, readme, sha256sums, stage } from './build-all.mjs';
import { PARTS, archIn, bundleName, describe, hostArch, hostOs, nodeName, portableName, serverName, size } from './build-names.mjs';
import { readVersion } from './release-version.mjs';

const script = fileURLToPath(new URL('./build-all.mjs', import.meta.url));

function temporary(t) {
  const dir = mkdtempSync(join(tmpdir(), 'hoi-build-all-'));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  return dir;
}

test('Tauri bundle names become <tool>_<version>_<os>-<arch>[-setup]', () => {
  const v = '1.0.0-preview.1';
  for (const [file, slug, want] of [
    ['hoi Node Setup_1.0.0-preview.1_x64-setup.exe', 'hoi-node-setup', 'hoi-node-setup_1.0.0-preview.1_windows-x64-setup.exe'],
    ['hoi_1.0.0-preview.1_arm64-setup.exe', 'hoi-desktop', 'hoi-desktop_1.0.0-preview.1_windows-arm64-setup.exe'],
    ['hoi_1.0.0-preview.1_x64_en-US.msi', 'hoi-desktop', 'hoi-desktop_1.0.0-preview.1_windows-x64.msi'],
    ['hoi Node Setup_1.0.0-preview.1_aarch64.dmg', 'hoi-node-setup', 'hoi-node-setup_1.0.0-preview.1_macos-arm64.dmg'],
    ['hoi_1.0.0-preview.1_universal.dmg', 'hoi-desktop', 'hoi-desktop_1.0.0-preview.1_macos-universal.dmg'],
    ['hoi_1.0.0-preview.1_amd64.AppImage', 'hoi-desktop', 'hoi-desktop_1.0.0-preview.1_linux-x64.AppImage'],
    ['hoi-node-setup_1.0.0-preview.1_arm64.deb', 'hoi-node-setup', 'hoi-node-setup_1.0.0-preview.1_linux-arm64.deb'],
    ['hoi-1.0.0-preview.1-1.x86_64.rpm', 'hoi-desktop', 'hoi-desktop_1.0.0-preview.1_linux-x64.rpm'],
    ['hoi-1.0.0-preview.1-1.aarch64.rpm', 'hoi-desktop', 'hoi-desktop_1.0.0-preview.1_linux-arm64.rpm'],
  ]) assert.equal(bundleName(slug, v, file), want, file);
  assert.equal(bundleName('hoi-desktop', v, 'hoi.app.tar.gz'), null);
  assert.equal(archIn('hoi_1.0.0_x86-setup.exe'), 'x86');
  assert.equal(portableName('hoi-node-setup', v, 'windows', 'x64'), 'hoi-node-setup_1.0.0-preview.1_windows-x64-portable.exe');
  assert.equal(portableName('hoi-node-setup', v, 'macos', 'arm64'), 'hoi-node-setup_1.0.0-preview.1_macos-arm64-portable.zip');
  assert.equal(serverName(v, 'arm64'), 'hoi-server_1.0.0-preview.1_linux-arm64.tar.gz');
  assert.equal(nodeName(v, 'x86_64-unknown-linux-gnu'), 'hoi-node_1.0.0-preview.1_x86_64-unknown-linux-gnu.tar.gz');
  assert.deepEqual([hostOs('win32'), hostOs('darwin'), hostOs('linux'), hostArch('x64'), hostArch('arm64')], ['windows', 'macos', 'linux', 'x64', 'arm64']);
  assert.equal(size(500), '500 B');
  assert.equal(size(12.34 * 1024 * 1024), '12 MB');
  assert.equal(size(1.5 * 1024 * 1024), '1.5 MB');
});

test('every file name of a build gets a description', () => {
  for (const name of [
    'hoi-node-setup_1.0.0_windows-x64-portable.exe', 'hoi-node-setup_1.0.0_macos-arm64-portable.zip',
    'hoi-desktop_1.0.0_windows-x64-setup.exe', 'hoi-desktop_1.0.0_windows-x64.msi', 'hoi-desktop_1.0.0_macos-arm64.dmg',
    'hoi-desktop_1.0.0_linux-x64.AppImage', 'hoi-desktop_1.0.0_linux-x64.deb', 'hoi-desktop_1.0.0_linux-x64.rpm',
    'hoi-server_1.0.0_linux-x64.tar.gz', 'hoi-server_1.0.0_linux-arm64.tar.gz',
    'hoi-node_1.0.0_x86_64-unknown-linux-gnu.tar.gz', 'hoi-node_1.0.0_aarch64-unknown-linux-gnu.tar.gz',
    'hoi-website_1.0.0.tar.gz',
  ]) assert.ok(describe(name).length > 20, name);
  assert.match(describe('hoi-node-setup_1.0.0_windows-x64-portable.exe'), /nothing is installed/);
  assert.match(describe('hoi-server_1.0.0_linux-arm64.tar.gz'), /hoi-server-linux-arm64\/install\.sh/);
  assert.match(describe('hoi-node_1.0.0_x86_64-unknown-linux-gnu.tar.gz'), /hoi Node Setup uploads/);
});

test('arguments choose parts in build order and refuse what makes no sense', () => {
  assert.deepEqual(parseArgs([]).parts, PARTS);
  assert.deepEqual(parseArgs(['--only', 'app,server']).parts, ['server', 'app']);
  assert.deepEqual(parseArgs(['--skip', 'website']).parts, ['server', 'node-setup', 'app']);
  assert.deepEqual(parseArgs(['--arch', 'all']).arches, ['x64', 'arm64']);
  assert.deepEqual(parseArgs(['--arch', 'arm64']).arches, ['arm64']);
  assert.equal(parseArgs(['--help']).help, true);
  for (const args of [['--only', 'desktop'], ['--skip', 'nope'], ['--arch', 'x86'], ['--only'], ['--out', '--only'],
    ['--only', 'server', '--skip', 'server'], ['--node-binary', 'x', '--arch', 'all'], ['--bogus', '1']]) {
    assert.throws(() => parseArgs(args), undefined, args.join(' '));
  }
});

test('a part is swapped in only when it is complete', (t) => {
  const dir = temporary(t);
  mkdirSync(join(dir, 'server'));
  writeFileSync(join(dir, 'server', 'old.tar.gz'), 'old');
  const failed = stage(dir, 'server');
  writeFileSync(join(failed.work, 'half.tar.gz'), 'half');
  failed.abandon();
  assert.deepEqual(readdirSync(join(dir, 'server')), ['old.tar.gz']);
  assert.equal(existsSync(join(dir, '.server.new')), false);
  const done = stage(dir, 'server');
  writeFileSync(join(done.work, 'new.tar.gz'), 'new');
  done.commit();
  assert.deepEqual(readdirSync(join(dir, 'server')), ['new.tar.gz']);
  assert.throws(() => stage(dir, '../elsewhere'), /not a build folder/);
});

test('the README and SHA256SUMS list every file by part', (t) => {
  const dir = temporary(t);
  const put = (path, text) => {
    mkdirSync(join(dir, path, '..'), { recursive: true });
    writeFileSync(join(dir, path), text);
  };
  put('app/hoi-desktop_1.0.0_windows-x64-setup.exe', 'app');
  put('server/hoi-server_1.0.0_linux-x64.tar.gz', 'server');
  put('server/hoi-node_1.0.0_x86_64-unknown-linux-gnu.tar.gz', 'node');
  put('node-setup/hoi-node-setup_1.0.0_windows-x64-portable.exe', 'portable');
  put('node-setup/.leftover', 'hidden');
  put('stray.txt', 'not a part');
  const files = listFiles(dir);
  assert.deepEqual(files.map((f) => f.path), [
    'server/hoi-node_1.0.0_x86_64-unknown-linux-gnu.tar.gz', 'server/hoi-server_1.0.0_linux-x64.tar.gz',
    'node-setup/hoi-node-setup_1.0.0_windows-x64-portable.exe', 'app/hoi-desktop_1.0.0_windows-x64-setup.exe',
  ]);
  const sums = sha256sums(dir, files);
  assert.match(sums, new RegExp(`^${createHash('sha256').update('node').digest('hex')}  server/hoi-node_`, 'm'));
  assert.equal(sums.trim().split('\n').length, 4);
  const text = readme({ version: '1.0.0', commit: 'abc1234', dirty: true, date: '2026-11-05 12:00 UTC', host: 'windows x64', files });
  assert.match(text, /^# hoi 1\.0\.0$/m);
  assert.match(text, /commit `abc1234` with uncommitted changes on windows x64/);
  assert.match(text, /## Set up a node on your server/);
  for (const heading of ['## server/: hoi Server', '## node-setup/: hoi Node Setup', '## app/: hoi']) assert.ok(text.includes(heading), heading);
  assert.doesNotMatch(text, /website\//);
  assert.ok(text.indexOf('## server/') < text.indexOf('## node-setup/') && text.indexOf('## node-setup/') < text.indexOf('## app/'));
});

test('pnpm build:all --only server packages a Linux hoi-node into builds/hoi-<version>/server', { skip: process.platform === 'win32' && 'needs tar with Unix paths' }, (t) => {
  const dir = temporary(t);
  const elf = Buffer.alloc(64);
  Buffer.from([127, 69, 76, 70, 2, 1]).copy(elf);
  elf.writeUInt16LE(62, 18);
  writeFileSync(join(dir, 'hoi-node'), elf);
  const out = join(dir, 'builds');
  const r = spawnSync(process.execPath, [script, '--only', 'server', '--node-binary', join(dir, 'hoi-node'), '--out', out], { encoding: 'utf8' });
  assert.equal(r.status, 0, r.stdout + r.stderr);
  const version = readVersion({ path: 'core/Cargo.toml', kind: 'cargo', section: 'workspace.package' });
  const folder = join(out, `hoi-${version}`);
  assert.deepEqual(readdirSync(folder).sort(), ['README.md', 'SHA256SUMS', 'server']);
  assert.deepEqual(readdirSync(join(folder, 'server')).sort(), [nodeName(version, 'x86_64-unknown-linux-gnu'), serverName(version, 'x64')]);
  // The hoi-node archive holds hoi-node at its root, as install.sh --binary takes it.
  const listing = spawnSync('tar', ['-tzf', join(folder, 'server', nodeName(version, 'x86_64-unknown-linux-gnu'))], { encoding: 'utf8' });
  assert.equal(listing.stdout.trim(), 'hoi-node');
  const server = spawnSync('tar', ['-tzf', join(folder, 'server', serverName(version, 'x64'))], { encoding: 'utf8' });
  assert.match(server.stdout, /^hoi-server-linux-x64\/install\.sh$/m);
  assert.equal(readFileSync(join(folder, 'SHA256SUMS'), 'utf8').trim().split('\n').length, 2);
  assert.match(readFileSync(join(folder, 'README.md'), 'utf8'), /## server\/: hoi Server/);
});
