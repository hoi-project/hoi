import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { checkSiteList } from './release-site-list.mjs';
import { loadBuildRelease } from './release-config.mjs';
import { signer } from './fixtures/minisign-test-signer.mjs';

const list = Buffer.from(JSON.stringify({ schema: 1, latest: '2.0.0', release: loadBuildRelease(), releases: [{ version: '2.0.0', date: '2026-11-06', files: [] }] }));

test('the site takes a release list only when it verifies against the trusted key', () => {
  const dir = mkdtempSync(join(tmpdir(), 'hoi-site-list-'));
  try {
    const s = signer();
    writeFileSync(join(dir, 'manifest.json'), list);
    writeFileSync(join(dir, 'manifest.json.minisig'), s.sign(list));
    assert.equal(checkSiteList(dir, s.key).latest, '2.0.0');
    assert.throws(() => checkSiteList(dir, signer().key), /wrong key|verify/);
    writeFileSync(join(dir, 'manifest.json'), Buffer.concat([list, Buffer.from(' ')]));
    assert.throws(() => checkSiteList(dir, s.key), /verify/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
