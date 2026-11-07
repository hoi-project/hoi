import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { parseLabels } from './labels.mjs';

test('the committed labels parse', () => {
  const labels = parseLabels(readFileSync(new URL('../.github/labels.yml', import.meta.url), 'utf8'));
  assert.ok(labels.length >= 10);
  const names = labels.map((label) => label.name);
  // Issue templates and Dependabot apply these; they must exist.
  for (const name of ['bug', 'enhancement', 'needs-triage', 'dependencies', 'good first issue']) assert.ok(names.includes(name), name);
});

test('quoted names, comments and blank lines', () => {
  const labels = parseLabels('# c\n\n- { name: "area: core", color: 1D76DB, description: "Rust, node" }\n');
  assert.deepEqual(labels, [{ name: 'area: core', color: '1d76db', description: 'Rust, node' }]);
});

test('rejects bad colors, missing names and duplicates', () => {
  assert.throws(() => parseLabels('- { name: x, color: red, description: y }'), /hex color/);
  assert.throws(() => parseLabels('- { color: ffffff, description: y }'), /name/);
  assert.throws(() => parseLabels('- { name: a, color: ffffff }\n- { name: A, color: 000000 }'), /duplicate/);
  assert.throws(() => parseLabels('name: a'), /expected/);
});
