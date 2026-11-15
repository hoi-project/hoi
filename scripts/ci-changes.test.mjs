import assert from 'node:assert/strict';
import { test } from 'node:test';
import { classify } from './ci-changes.mjs';

const codeql = (paths) => JSON.parse(classify(paths).codeql);

test('without a base, everything runs', () => {
  assert.deepEqual(classify(null), {
    rust: true,
    web: true,
    deps: true,
    workflows: true,
    sonar: true,
    codeql: '["actions","javascript-typescript","rust"]',
  });
});

test('documentation alone selects no conditional build or security jobs', () => {
  assert.deepEqual(classify(['README.md', 'docs/CI.md', 'docs/spec/PROTOCOL.md', 'rfcs/README.md']),
    { rust: false, web: false, deps: false, workflows: false, sonar: false, codeql: '[]' });
});

test('each area selects its jobs', () => {
  assert.equal(classify(['core/crates/hoi-types/src/record.rs']).rust, true);
  assert.equal(classify(['core/crates/hoi-types/src/record.rs']).web, false);
  assert.equal(classify(['apps/web/src/lib/util.ts']).web, true);
  assert.equal(classify(['apps/web/src/lib/util.ts']).rust, false);
  assert.equal(classify(['scripts/build-all.mjs']).web, false);
  assert.equal(classify(['scripts/build-all.mjs']).rust, false);
  assert.equal(classify(['core/Cargo.lock']).deps, true);
  assert.equal(classify(['apps/desktop/src-tauri/Cargo.toml']).deps, true);
  assert.equal(classify(['pnpm-lock.yaml']).deps, true);
  assert.equal(classify(['pnpm-lock.yaml']).web, true);
  assert.equal(classify(['core/supply-chain/audits.toml']).deps, true);
  assert.equal(classify(['.github/workflows/release.yml']).workflows, true);
  assert.equal(classify(['.github/workflows/release.yml']).rust, false);
});

test('release.json is compiled into hoi-core, so it selects the Rust job', () => {
  assert.equal(classify(['release.json']).rust, true);
  assert.equal(classify(['release.json']).web, true);
});

test('changing CI itself runs the jobs it defines', () => {
  const ci = classify(['.github/workflows/ci.yml']);
  assert.equal(ci.rust, true);
  assert.equal(ci.web, true);
  assert.equal(ci.workflows, true);
  assert.equal(classify(['.github/workflows/security.yml']).deps, true);
  assert.equal(classify(['vendor/dimpl/src/lib.rs']).rust, true);
  assert.equal(classify(['vendor/dimpl/src/lib.rs']).deps, true);
});

test('CodeQL runs only the languages whose files changed', () => {
  assert.deepEqual(codeql(['README.md', 'docs/CI.md']), []);
  assert.deepEqual(codeql(['core/crates/hoi-types/src/record.rs']), ['rust']);
  assert.deepEqual(codeql(['core/Cargo.lock']), ['rust']);
  assert.deepEqual(codeql(['apps/web/src/lib/util.ts', 'apps/web/src/App.svelte']), ['javascript-typescript']);
  assert.deepEqual(codeql(['scripts/build-all.mjs']), ['javascript-typescript']);
  assert.deepEqual(codeql(['.github/workflows/release.yml']), ['actions']);
  assert.deepEqual(codeql(['core/a.rs', 'apps/web/a.ts', '.github/workflows/ci.yml']),
    ['actions', 'javascript-typescript', 'rust']);
});

test('CodeQL ignores the paths its configuration ignores', () => {
  assert.deepEqual(codeql(['vendor/dimpl/src/lib.rs', 'apps/web/src/lib/core/wasm/pkg/hoi.js']), []);
});

test('changing the security workflow runs every CodeQL language', () => {
  assert.deepEqual(codeql(['.github/workflows/security.yml']), ['actions', 'javascript-typescript', 'rust']);
});

test('SonarCloud runs for Rust, web and its own settings, not for documentation', () => {
  assert.equal(classify(['README.md']).sonar, false);
  assert.equal(classify(['.github/workflows/release.yml']).sonar, false);
  assert.equal(classify(['core/a.rs']).sonar, true);
  assert.equal(classify(['apps/web/a.ts']).sonar, true);
  assert.equal(classify(['sonar-project.properties']).sonar, true);
});
