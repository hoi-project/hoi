import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { reproducibleEnv, treeDigest } from './reproduce.mjs';

test('a tree digest depends on paths and bytes, never on order or times', () => {
  const a = mkdtempSync(join(tmpdir(), 'hoi-tree-'));
  const b = mkdtempSync(join(tmpdir(), 'hoi-tree-'));
  try {
    mkdirSync(join(a, 'x'));
    writeFileSync(join(a, 'x', 'one.js'), 'one');
    writeFileSync(join(a, 'two.css'), 'two');
    writeFileSync(join(b, 'two.css'), 'two');
    mkdirSync(join(b, 'x'));
    writeFileSync(join(b, 'x', 'one.js'), 'one');
    assert.deepEqual(treeDigest(a), treeDigest(b));
    assert.equal(treeDigest(a).files, 2);
    writeFileSync(join(b, 'x', 'one.js'), 'One');
    assert.notEqual(treeDigest(a).digest, treeDigest(b).digest);
  } finally {
    rmSync(a, { recursive: true, force: true });
    rmSync(b, { recursive: true, force: true });
  }
});

test('builds run with the commit time, UTC and every path remapped', () => {
  const env = reproducibleEnv({ HOME: '/home/someone', PATH: '/bin' }, '1793800000', '/tmp/target');
  assert.equal(env.SOURCE_DATE_EPOCH, '1793800000');
  assert.equal(env.TZ, 'UTC');
  assert.equal(env.CARGO_INCREMENTAL, '0');
  assert.equal(env.CARGO_TARGET_DIR, '/tmp/target');
  assert.match(env.CARGO_ENCODED_RUSTFLAGS, /--remap-path-prefix=.*=\/hoi/);
});
