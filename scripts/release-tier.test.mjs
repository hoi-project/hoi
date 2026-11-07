import assert from 'node:assert/strict';
import { test } from 'node:test';
import { releaseTier } from './release-tier.mjs';

test('only numeric major releases select the expensive suites', () => {
  for (const [version, tier] of [
    ['v1.0.1', 'patch'], ['0.1.2', 'patch'], ['v1.2.0', 'minor'],
    ['v0.1.0', 'minor'], ['v2.0.0', 'major'], ['v2.0.0-rc.1', 'major'],
    ['v2.1.0-beta.2', 'minor'], ['v2.1.1-rc.1', 'patch'], ['v0.0.0', 'patch'],
  ]) assert.equal(releaseTier(version), tier, version);
});

test('malformed tags cannot select a release tier', () => {
  for (const version of ['', 'main', 'v1', 'v01.0.0', 'v1.0.0-01', 'v1.0.0-',
    'v1.0.0-rc..1', 'v1.0.0;echo hi', 'v1.0.0\n', 'v1.0.0+build']) {
    assert.throws(() => releaseTier(version), /Invalid release version/, version);
  }
});
