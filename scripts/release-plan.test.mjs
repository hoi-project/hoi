import assert from 'node:assert/strict';
import { cpSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { planRelease } from './release-plan.mjs';
import { applyVersion, LOCKFILES, planVersion, versionFiles, writeVersion } from './release-version.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const sha = 'a'.repeat(40);
const before = 'b'.repeat(40);
const automatic = { automatic: true, ref: 'refs/heads/main', before };

function fixture(t) {
  const base = mkdtempSync(join(tmpdir(), 'hoi-release-plan-'));
  t.after(() => rmSync(base, { recursive: true, force: true }));
  for (const path of new Set([...versionFiles().map(file => file.path), ...LOCKFILES, 'release.json', 'package.json'])) {
    cpSync(join(root, path), join(base, path), { recursive: true });
  }
  applyVersion(planVersion('1.0.0-preview.1', {}, base), base);
  return base;
}

function history({ previous = '0.1.0', tagSha = null } = {}) {
  return args => {
    switch (args.join(' ')) {
      case `show ${before}:core/Cargo.toml`: return `[workspace.package]\nversion = "${previous}"\n[dependencies]\nother = "99.0.0"\n`;
      case 'rev-parse HEAD^{commit}': return sha;
      case 'show-ref --verify --quiet refs/tags/v1.0.0-preview.1': return tagSha === null ? null : '';
      case 'rev-parse refs/tags/v1.0.0-preview.1^{commit}': return tagSha;
      default: throw new Error(`Unexpected Git operation: ${args.join(' ')}`);
    }
  };
}

test('a pushed version change plans a tag on the exact commit and full preview checks', t => {
  const plan = planRelease(automatic, fixture(t), history());
  assert.deepEqual(plan, { release: true, version: '1.0.0-preview.1', tag: 'v1.0.0-preview.1',
    tier: 'major', channel: 'preview', prerelease: true, sha, create_tag: true });
});

test('ordinary pushes and initial or non-main branch pushes do not release', t => {
  const base = fixture(t);
  assert.equal(planRelease(automatic, base, history({ previous: '1.0.0-preview.1' })).release, false);
  const noGit = () => { throw new Error('This event should not inspect Git history.'); };
  for (const patch of [{ ref: 'refs/heads/feature' }, { before: '0'.repeat(40) }, { before: undefined }]) {
    assert.equal(planRelease({ ...automatic, ...patch }, base, noGit).release, false);
  }
});

test('manual runs release the selected commit without a previous commit or tag', t => {
  const plan = planRelease({}, fixture(t), history());
  assert.equal(plan.release, true);
  assert.equal(plan.create_tag, true);
  assert.equal(plan.sha, sha);
});

test('retrying the same commit reuses its tag; reusing a version on another commit fails', t => {
  const base = fixture(t);
  const retry = planRelease({}, base, history({ tagSha: sha }));
  assert.equal(retry.create_tag, false);
  assert.equal(retry.release, true);
  assert.throws(() => planRelease({}, base, history({ tagSha: before })), /already points to another commit/);
});

test('inconsistent versions fail before a tag can be planned', t => {
  const base = fixture(t);
  writeVersion({ path: 'apps/desktop/src-tauri/tauri.conf.json', kind: 'json' }, '9.0.0', base);
  assert.throws(() => planRelease(automatic, base, history()), /tauri.conf.json/);
});

test('malformed or unavailable previous history cannot silently trigger a release', t => {
  const base = fixture(t);
  assert.throws(() => planRelease({ ...automatic, before: '--help' }, base, history()), /Invalid previous commit/);
  assert.throws(() => planRelease(automatic, base, () => ''), /Previous commit has no workspace version/);
  assert.throws(() => planRelease(automatic, base, () => { throw new Error('Commit is unavailable'); }), /Commit is unavailable/);
});
