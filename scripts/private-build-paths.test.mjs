import assert from 'node:assert/strict';
import { test } from 'node:test';
import { privateBuildPaths } from './private-build-paths.mjs';

test('public WASM remaps developer paths without losing existing compiler flags or mutating the environment', () => {
  const env = { CARGO_ENCODED_RUSTFLAGS: '-C\x1ftarget-feature=+bulk-memory', CARGO_HOME: 'C:\\private work\\cargo', USERPROFILE: 'C:\\Users\\developer' };
  const result = privateBuildPaths('G:\\project with spaces\\hoi', env, 'C:\\Users\\developer');
  const flags = result.CARGO_ENCODED_RUSTFLAGS.split('\x1f');
  assert.deepEqual(flags.slice(0, 2), ['-C', 'target-feature=+bulk-memory']);
  assert.ok(flags.includes('--remap-path-prefix=C:\\Users\\developer=/build-user'));
  assert.ok(flags.includes('--remap-path-prefix=C:/Users/developer=/build-user'));
  assert.ok(flags.includes('--remap-path-prefix=G:\\project with spaces\\hoi=/hoi'));
  assert.ok(flags.includes('--remap-path-prefix=C:\\private work\\cargo=/cargo'));
  assert.equal(env.CARGO_ENCODED_RUSTFLAGS, '-C\x1ftarget-feature=+bulk-memory');
});
