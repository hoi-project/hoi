import assert from 'node:assert/strict';
import { test } from 'node:test';
import { classify } from './ci-changes.mjs';

test('without a base, everything runs', () => {
  assert.deepEqual(classify(null), { rust: true, web: true, deps: true, workflows: true });
});

test('documentation alone selects no conditional build or security jobs', () => {
  assert.deepEqual(classify(['README.md', 'docs/CI.md', 'docs/spec/PROTOCOL.md', 'rfcs/README.md']),
    { rust: false, web: false, deps: false, workflows: false });
});

test('each area selects its jobs', () => {
  assert.deepEqual(classify(['core/crates/hoi-types/src/record.rs']), { rust: true, web: false, deps: false, workflows: false });
  assert.deepEqual(classify(['apps/web/src/lib/util.ts']), { rust: false, web: true, deps: false, workflows: false });
  assert.deepEqual(classify(['scripts/build-all.mjs']), { rust: false, web: false, deps: false, workflows: false });
  assert.equal(classify(['core/Cargo.lock']).deps, true);
  assert.equal(classify(['apps/desktop/src-tauri/Cargo.toml']).deps, true);
  assert.equal(classify(['pnpm-lock.yaml']).deps, true);
  assert.equal(classify(['pnpm-lock.yaml']).web, true);
  assert.equal(classify(['core/supply-chain/audits.toml']).deps, true);
  assert.equal(classify(['.github/workflows/release.yml']).workflows, true);
  assert.equal(classify(['.github/workflows/release.yml']).rust, false);
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
