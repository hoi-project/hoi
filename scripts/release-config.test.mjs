import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, unlinkSync, rmdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { loadBuildRelease, loadRelease, validateRelease } from './release-config.mjs';
import { noticeForBuild } from './release-schema.mjs';

const preview = () => ({ ...loadBuildRelease(), revision: 1, channel: 'preview' });

test('the checked-in release has an explicit valid channel and compatibility declaration', () => {
  const release = loadBuildRelease();
  assert.deepEqual(validateRelease(release), release);
  assert.ok(['preview', 'stable'].includes(release.channel));
  assert.ok(release.supportedProtocols.includes(release.protocolVersion));
});

test('unsupported schemas and ambiguous compatibility fail closed', () => {
  const release = loadRelease();
  for (const patch of [{ schemaVersion: 2 }, { revision: -1 }, { indexVersion: 0 }, { indexVersion: 2_147_483_648 }, { channel: 'production' }, { supportedProtocols: [] }, { notice: { title: '', message: '' } }]) {
    assert.throws(() => validateRelease({ ...release, ...patch }), /Invalid release/);
  }
});

test('operator changes refresh notices without relabelling old builds or exposing control fields', () => {
  const build = preview();
  const stable = validateRelease({ ...build, revision: 2, channel: 'stable', protocolVersion: '1.0', supportedProtocols: ['1.0'], wipe: true });
  assert.equal('wipe' in stable, false);
  const notice = noticeForBuild(build, stable);
  assert.equal(notice.preview, true);
  assert.equal(notice.title, build.notice.title);
  assert.match(notice.update, /installed build remains preview/);
  const edited = { ...build, revision: 2, notice: { title: 'Preview update', message: 'A new preview is available.' } };
  assert.equal(noticeForBuild(build, edited).message, edited.notice.message);
  assert.equal(noticeForBuild(build, { ...edited, revision: 0 }).message, build.notice.message);
});

test('the loader sees a manifest replacement without process restart', (t) => {
  const dir = mkdtempSync(join(tmpdir(), 'hoi-release-'));
  const path = join(dir, 'release.json');
  t.after(() => { unlinkSync(path); rmdirSync(dir); });
  const release = loadRelease();
  writeFileSync(path, JSON.stringify(release));
  assert.equal(loadRelease(path).revision, release.revision);
  writeFileSync(path, JSON.stringify({ ...release, revision: release.revision + 1 }));
  assert.equal(loadRelease(path).revision, release.revision + 1);
});

test('runtime operator overrides cannot relabel the compiled Rust and UI protocol snapshot', (t) => {
  const dir = mkdtempSync(join(tmpdir(), 'hoi-release-build-'));
  const path = join(dir, 'operator-release.json');
  const original = process.env.HOI_RELEASE_CONFIG;
  t.after(() => {
    if (original === undefined) delete process.env.HOI_RELEASE_CONFIG;
    else process.env.HOI_RELEASE_CONFIG = original;
    unlinkSync(path); rmdirSync(dir);
  });
  const build = loadBuildRelease();
  writeFileSync(path, JSON.stringify({ ...build, revision: build.revision + 1, protocolVersion: '99.0', supportedProtocols: ['99.0'], indexVersion: 99 }));
  process.env.HOI_RELEASE_CONFIG = path;
  assert.equal(loadRelease().protocolVersion, '99.0');
  assert.deepEqual(loadBuildRelease(), build);
});
