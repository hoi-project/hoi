// Fast regression guard. cargo-deny remains responsible for current advisory data.
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export function dependencySecurityErrors(lock, label = 'Cargo.lock') {
  const errors = [];
  const packages = lock.split(/^\[\[package\]\]\s*$/m).slice(1).map((entry) => ({
    name: /^name = "([^"]+)"$/m.exec(entry)?.[1],
    version: /^version = "([^"]+)"$/m.exec(entry)?.[1],
    source: /^source = "([^"]+)"$/m.exec(entry)?.[1],
  }));
  const integers = packages.filter((entry) => entry.name === 'crypto-bigint');
  if (!integers.length) errors.push(`${label}: expected crypto-bigint is missing; review this gate with the dependency change.`);
  for (const entry of integers) {
    const version = /^(\d+)\.(\d+)\.(\d+)$/.exec(entry.version ?? '');
    if (!version || (Number(version[1]) === 0 && (Number(version[2]) < 7 || (Number(version[2]) === 7 && Number(version[3]) < 6)))) {
      errors.push(`${label}: crypto-bigint ${entry.version} fails RUSTSEC-2026-0387; require >=0.7.6 without a prerelease.`);
    }
  }
  // rusqlite's bundled feature ships this SQLite source in the node and desktop app.
  for (const entry of packages.filter((entry) => entry.name === 'libsqlite3-sys')) {
    const version = /^(\d+)\.(\d+)\.(\d+)$/.exec(entry.version ?? '');
    if (!version || (Number(version[1]) === 0 && Number(version[2]) < 39)) {
      errors.push(`${label}: libsqlite3-sys ${entry.version} fails RUSTSEC-2026-0459; require >=0.39.0 without a prerelease.`);
    }
  }
  if (packages.some((entry) => entry.name === 'proc-macro-error2')) errors.push(`${label}: proc-macro-error2 reintroduces RUSTSEC-2026-0173.`);
  if (packages.some((entry) => entry.name === 'async-executor' && entry.version === '1.14.0')) {
    errors.push(`${label}: async-executor 1.14.0 was yanked for a use-after-free; require the fixed 1.14.1 release or newer.`);
  }
  for (const entry of packages.filter((entry) => entry.name === 'netdev' && entry.version === '0.46.3')) {
    if (entry.source) errors.push(`${label}: netdev 0.46.3 must use the local iOS compatibility patch.`);
  }
  // The reviewed patches belong to the core's MLS and media graph. A graph without openmls (the
  // node setup app's) must not pull any of them from a registry either.
  const mls = packages.some((entry) => entry.name === 'openmls');
  for (const name of ['openmls_rust_crypto', 'openmls_basic_credential', 'dimpl', 'core-models']) {
    const entries = packages.filter((entry) => entry.name === name);
    const patched = entries.length === 1 && !entries[0].source;
    if (mls ? !patched : entries.some((entry) => entry.source)) errors.push(`${label}: ${name} must use the reviewed local patch; review upstream before removing this requirement.`);
  }
  return errors;
}

export function checkDependencySecurity(root = resolve(dirname(fileURLToPath(import.meta.url)), '..')) {
  const errors = ['core/Cargo.lock', 'apps/desktop/src-tauri/Cargo.lock', 'apps/setup/src-tauri/Cargo.lock'].flatMap((file) => dependencySecurityErrors(readFileSync(resolve(root, file), 'utf8'), file));
  if (errors.length) throw new Error(errors.join('\n'));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  checkDependencySecurity();
  console.log('Dependency regression gate passed for core, desktop and the node setup app. Run cargo deny check for current advisories.');
}
