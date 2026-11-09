import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { linuxBundles, msiTakesVersion, windowsBundles } from './tauri-bundles.mjs';

test('WiX takes plain and numeric pre-release versions only, as Tauri checks them', () => {
  for (const v of ['1.0.0', '0.3.12', '1.0.0-7', '1.0.0+42', '1.0.0-preview.1+3', '255.255.65535', '1.0.0-65535']) assert.equal(msiTakesVersion(v), true, v);
  for (const v of ['1.0.0-preview.1', '1.0.0-rc.1', '1.0.0-65536', '1.0.0+build.5', '256.0.0', '1.256.0', '1.0.65536', '1.0', 'next']) assert.equal(msiTakesVersion(v), false, v);
});

test('a preview built on Windows makes the NSIS setup alone; everything else is left as it is', () => {
  const narrowed = windowsBundles(['build'], '1.0.0-preview.1', 'win32');
  assert.deepEqual(narrowed.args, ['build', '--bundles', 'nsis']);
  assert.match(narrowed.note, /1\.0\.0-preview\.1/);
  // The release workflow's arguments (signing config) are kept.
  assert.deepEqual(windowsBundles(['build', '--config', 'x.json'], '1.0.0-preview.1', 'win32').args, ['build', '--config', 'x.json', '--bundles', 'nsis']);
  // Cross-building for Windows from elsewhere.
  assert.deepEqual(windowsBundles(['build', '--target', 'x86_64-pc-windows-msvc'], '1.0.0-preview.1', 'linux').args.slice(-2), ['--bundles', 'nsis']);
  assert.deepEqual(windowsBundles(['build', '--target=aarch64-pc-windows-msvc'], '1.0.0-preview.1', 'linux').args.slice(-2), ['--bundles', 'nsis']);

  for (const [args, version, platform] of [
    [['build'], '1.0.0', 'win32'], // WiX takes it: .msi and -setup.exe
    [['build'], '1.0.0-preview.1', 'linux'],
    [['build'], '1.0.0-preview.1', 'darwin'],
    [['build', '--target', 'universal-apple-darwin'], '1.0.0-preview.1', 'win32'],
    [['dev'], '1.0.0-preview.1', 'win32'],
    [['build', '--bundles', 'msi'], '1.0.0-preview.1', 'win32'], // asked for: Tauri says why it can't
    [['build', '--bundles=nsis,msi'], '1.0.0-preview.1', 'win32'],
    [['build', '-b', 'nsis'], '1.0.0-preview.1', 'win32'],
    [['build', '--no-bundle'], '1.0.0-preview.1', 'win32'],
  ]) {
    const r = windowsBundles(args, version, platform);
    assert.deepEqual(r.args, args, `${args.join(' ')} ${version} ${platform}`);
    assert.equal(r.note, null);
  }
});

test('both Tauri apps narrow their Windows bundles', () => {
  for (const app of ['desktop', 'setup']) {
    assert.match(readFileSync(new URL(`../apps/${app}/scripts/tauri.mjs`, import.meta.url), 'utf8'), /windowsBundles\(/, app);
  }
});

test('Linux prereleases keep their full version in Debian and AppImage bundles', () => {
  for (const version of ['1.0.0-preview.2', '1.0.0-rc.1', '1.0.0-7', '1.0.0-alpha-beta']) {
    const result = linuxBundles(['build', '--config', 'signing.json'], version, 'linux');
    assert.deepEqual(result.args, ['build', '--config', 'signing.json', '--bundles', 'deb,appimage']);
    assert.ok(result.note.includes(version));
  }
  // A target triple, rather than the build host, determines the package format.
  for (const target of ['x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu']) {
    assert.deepEqual(linuxBundles(['build', '--target', target], '1.0.0-preview.2', 'win32').args,
      ['build', '--target', target, '--bundles', 'deb,appimage']);
    assert.deepEqual(linuxBundles(['bundle', `--target=${target}`], '1.0.0-preview.2', 'darwin').args,
      ['bundle', `--target=${target}`, '--bundles', 'deb,appimage']);
  }
  assert.deepEqual(linuxBundles(['build', '--', '--locked'], '1.0.0-preview.2', 'linux').args,
    ['build', '--bundles', 'deb,appimage', '--', '--locked']);
});

test('an explicit unsupported RPM request fails before compiling', () => {
  for (const selection of [['--bundles', 'rpm'], ['--bundles', 'deb,rpm'], ['--bundles', 'deb', 'rpm'], ['--bundles=rpm,appimage'], ['--bundles=deb', 'rpm'], ['-b', 'rpm'], ['-b=rpm'], ['-brpm']]) {
    for (const command of ['build', 'bundle']) {
      assert.throws(() => linuxBundles([command, ...selection], '1.0.0-preview.2', 'linux'),
        /RPM cannot package version 1\.0\.0-preview\.2.*Use --bundles deb,appimage/);
    }
  }
});

test('stable RPMs, supported bundle selections and other platforms are preserved', () => {
  for (const [args, version, platform] of [
    [['build'], '1.0.0', 'linux'],
    [['build', '--bundles', 'rpm'], '1.0.0', 'linux'],
    [['build'], '1.0.0-preview.2', 'darwin'],
    [['build'], '1.0.0-preview.2', 'win32'],
    [['build', '--target', 'universal-apple-darwin'], '1.0.0-preview.2', 'linux'],
    [['build', '-t', 'x86_64-pc-windows-msvc'], '1.0.0-preview.2', 'linux'],
    [['dev'], '1.0.0-preview.2', 'linux'],
    [['build', '--bundles', 'deb'], '1.0.0-preview.2', 'linux'],
    [['build', '--bundles=deb,appimage'], '1.0.0-preview.2', 'linux'],
    [['build', '--bundles', 'deb', 'appimage', '--config', 'signing.json'], '1.0.0-preview.2', 'linux'],
    [['build', '--bundles'], '1.0.0-preview.2', 'linux'],
    [['build', '-b', 'appimage'], '1.0.0-preview.2', 'linux'],
    [['build', '--no-bundle'], '1.0.0-preview.2', 'linux'],
  ]) {
    assert.deepEqual(linuxBundles(args, version, platform), { args, note: null });
  }
});

test('both Tauri apps select Linux bundles before invoking the CLI', () => {
  for (const app of ['desktop', 'setup']) {
    const script = readFileSync(new URL(`../apps/${app}/scripts/tauri.mjs`, import.meta.url), 'utf8');
    assert.match(script, /linuxBundles\(windows\.args, (?:base\.)?version\)/, app);
    assert.match(script, /await run\(linux\.args,/, app);
  }
});
