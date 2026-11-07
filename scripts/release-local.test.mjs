import assert from 'node:assert/strict';
import { cpSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { LOCKFILES, lockWithVersion, mismatches, readVersion, repoSlug, versionFiles, writeVersion } from './release-local.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

test('local releases target the public repository rather than the private checkout origin', () => {
  assert.equal(repoSlug({}), 'hoi-project/hoi');
  assert.equal(repoSlug({ GITHUB_REPOSITORY: 'fork/hoi' }), 'fork/hoi');
});

test('every package of the tree carries the same version', () => {
  const versions = new Set(versionFiles().map((f) => readVersion(f)));
  assert.equal(versions.size, 1, `versions differ: ${[...versions].join(', ')}`);
  assert.match([...versions][0], /^\d+\.\d+\.\d+/);
});

test('a bump writes the version everywhere and nothing else', () => {
  const copy = mkdtempSync(join(tmpdir(), 'hoi-release-'));
  try {
    const files = versionFiles();
    for (const path of [...files.map(f => f.path), ...LOCKFILES, 'package.json']) {
      cpSync(join(root, path), join(copy, path), { recursive: true });
    }
    const before = readVersion(files[0], copy);
    assert.deepEqual(mismatches(before, copy), []);
    for (const f of files) writeVersion(f, '9.8.7-rc.1', copy);
    // Manifest-only updates must also report the stale lockfiles.
    assert.equal(mismatches('9.8.7-rc.1', copy).length, LOCKFILES.length);
    assert.equal(mismatches(before, copy).length, files.length);
    // Dependency versions in the same files stay as they were.
    const cargo = files.find((f) => f.path === 'apps/desktop/src-tauri/Cargo.toml');
    assert.equal(readVersion({ ...cargo, section: 'workspace.package' }, copy), null);
  } finally {
    rmSync(copy, { recursive: true, force: true });
  }
});

test("the lockfiles get the new version for hoi's own crates only", () => {
  for (const lock of LOCKFILES) {
    const text = readFileSync(join(root, lock), 'utf8').replaceAll('\r\n', '\n');
    const next = lockWithVersion(text, '9.8.7');
    const own = [...next.matchAll(/^name = "(hoi-[a-z-]+)"\nversion = "([^"]*)"/gm)];
    // The node setup app's graph has two hoi crates (the app and its engine); the others carry the core.
    assert.ok(own.length >= (lock.includes('setup') ? 2 : 5), lock);
    for (const [, name, version] of own) assert.equal(version, '9.8.7', `${lock}: ${name}`);
    // Only those lines change; every other package, vendored ones included, keeps its version.
    const before = text.split('\n');
    const changed = next.split('\n').filter((line, i) => line !== before[i]);
    assert.deepEqual(changed, own.map(() => 'version = "9.8.7"'));
  }
});

test('version updates preserve Windows lockfile line endings', () => {
  const text = 'name = "hoi-types"\r\nversion = "0.1.0"\r\n\r\nname = "hoi-external"\r\nversion = "0.1.0"\r\nsource = "registry+example"\r\n';
  assert.equal(lockWithVersion(text, '1.0.0'), text.replace('version = "0.1.0"', 'version = "1.0.0"'));
});
