import test from 'node:test';
import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import { syncBuiltinESMExports } from 'node:module';
import { nameLog, treeRoot, leafHash } from './log.mjs';

const realCreateHash = crypto.createHash;

/** A store holding `entries` (strings), the shape `nameLog` reads. */
const memoryStore = (entries) => ({
  logSecret: () => '11'.repeat(32),
  logSize: () => entries.length,
  logEntries: (start, end) => entries.slice(start, end).map((entry) => ({ entry, withheld: false })),
});

/** Runs `fn` and returns how many SHA-256 hashes `log.mjs` created meanwhile. */
function hashesDuring(fn) {
  let count = 0;
  crypto.createHash = (...args) => { count += 1; return realCreateHash(...args); };
  syncBuiltinESMExports();
  try { fn(); } finally {
    crypto.createHash = realCreateHash;
    syncBuiltinESMExports();
  }
  return count;
}

test('a head of an unchanged log hashes no tree nodes, and a grown log stays correct', () => {
  const entries = Array.from({ length: 64 }, (_, i) => `entry ${i}`);
  const log = nameLog(memoryStore(entries));
  const first = log.head(1);
  assert.equal(first.size, 64);
  assert.equal(first.root, treeRoot(entries.map(leafHash)).toString('hex'));

  assert.equal(hashesDuring(() => log.head(2)), 0);
  assert.equal(log.head(3).root, first.root);

  entries.push('entry 64');
  const grown = log.head(4);
  assert.equal(grown.size, 65);
  assert.equal(grown.root, treeRoot(entries.map(leafHash)).toString('hex'));
  assert.notEqual(grown.root, first.root);
});
