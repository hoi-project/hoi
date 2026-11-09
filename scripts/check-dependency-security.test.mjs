import assert from 'node:assert/strict';
import test from 'node:test';
import { checkDependencySecurity, dependencySecurityErrors } from './check-dependency-security.mjs';

const dependency = (name, version, source = '') => `[[package]]\nname = "${name}"\nversion = "${version}"\n${source ? `source = "${source}"\n` : ''}`;
const patches = ['openmls_rust_crypto', 'openmls_basic_credential', 'dimpl', 'core-models'].map((name) => dependency(name, '0.6.0')).join('\n');
const safe = dependency('openmls', '0.6.0', 'registry+https://github.com/rust-lang/crates.io-index') + '\n' + patches + '\n' + dependency('crypto-bigint', '0.7.8', 'registry+https://github.com/rust-lang/crates.io-index');

test('every checked-in lockfile passes the security regression gate', () => assert.doesNotThrow(() => checkDependencySecurity()));
test('a second vulnerable version cannot hide behind a patched version', () => {
  assert.deepEqual(dependencySecurityErrors(safe), []);
  for (const version of ['0.5.5', '0.7.5', '0.7.6-rc.1']) assert.match(dependencySecurityErrors(safe + '\n' + dependency('crypto-bigint', version)).join('\n'), /RUSTSEC-2026-0387/);
});
test('bundled SQLite cannot regress to the page-cache memory exposure', () => {
  for (const version of ['0.39.0', '0.39.1', '0.40.0', '1.0.0']) {
    assert.deepEqual(dependencySecurityErrors(safe + '\n' + dependency('libsqlite3-sys', version)), []);
  }
  const patched = safe + '\n' + dependency('libsqlite3-sys', '0.39.1');
  for (const version of ['0.38.2', '0.39.0-rc.1', 'invalid']) {
    assert.match(dependencySecurityErrors(patched + '\n' + dependency('libsqlite3-sys', version)).join('\n'), /RUSTSEC-2026-0459/);
  }
});
test('an unmaintained macro or unpatched registry provider fails closed', () => {
  assert.match(dependencySecurityErrors(safe + '\n' + dependency('proc-macro-error2', '2.0.1')).join('\n'), /RUSTSEC-2026-0173/);
  assert.match(dependencySecurityErrors(safe.replace('name = "dimpl"', 'source = "registry+https://github.com/rust-lang/crates.io-index"\nname = "dimpl"')).join('\n'), /dimpl must use/);
  assert.match(dependencySecurityErrors('').join('\n'), /expected crypto-bigint is missing/);
  // Without openmls (the node setup app's graph) the patched crates are not required, but a registry copy still fails.
  const withoutMls = safe.replace(/\[\[package\]\]\nname = "openmls"[^[]*/, '');
  assert.match(dependencySecurityErrors(safe.replace(patches + '\n', '')).join('\n'), /openmls_rust_crypto must use/);
  assert.deepEqual(dependencySecurityErrors(withoutMls.replace(patches + '\n', '')), []);
  assert.match(dependencySecurityErrors(withoutMls.replace('name = "dimpl"', 'source = "registry+https://github.com/rust-lang/crates.io-index"\nname = "dimpl"')).join('\n'), /dimpl must use/);
});

test('release dependencies cannot regress to the yanked executor or broken iOS request type', () => {
  assert.match(dependencySecurityErrors(safe + '\n' + dependency('async-executor', '1.14.0')).join('\n'), /use-after-free/);
  assert.deepEqual(dependencySecurityErrors(safe + '\n' + dependency('async-executor', '1.14.1')), []);
  assert.match(dependencySecurityErrors(safe + '\n' + dependency('netdev', '0.46.3', 'registry+https://github.com/rust-lang/crates.io-index')).join('\n'), /iOS compatibility patch/);
  assert.deepEqual(dependencySecurityErrors(safe + '\n' + dependency('netdev', '0.46.3')), []);
});
