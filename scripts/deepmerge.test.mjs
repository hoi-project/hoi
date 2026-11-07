import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { test } from 'node:test';

const fromWeb = createRequire(new URL('../apps/web/package.json', import.meta.url));
const fromPlugin = createRequire(fromWeb.resolve('@sveltejs/vite-plugin-svelte'));
const merge = fromPlugin('deepmerge');

test('the Svelte build uses the reviewed deepmerge patch', () => {
  assert.equal(readFileSync(fromPlugin.resolve('deepmerge'), 'utf8'),
    readFileSync(new URL('../vendor/deepmerge/index.cjs', import.meta.url), 'utf8'));
});

test('malicious source and target keys cannot poison merged object prototypes', () => {
  const malicious = JSON.parse('{"__proto__":{"polluted":true},"constructor":{"prototype":{"polluted":true}},"prototype":{"polluted":true}}');
  for (const target of [{}, Object.create(null), malicious]) {
    for (const source of [{}, Object.create(null), malicious]) {
      for (const clone of [true, false]) {
        const result = merge(target, source, { clone });
        assert.equal(Object.getPrototypeOf(result), Object.prototype);
        assert.equal(result.polluted, undefined);
        for (const key of ['__proto__', 'constructor', 'prototype']) assert.equal(Object.hasOwn(result, key), false);
      }
    }
  }
  const nested = merge({}, { nested: malicious, array: [malicious] });
  assert.deepEqual(nested, { nested: {}, array: [{}] });
  assert.equal({}.polluted, undefined);
});

test('Svelte config merges still preserve functions, symbols and array replacement', () => {
  const preprocess = () => {};
  const symbol = Symbol('option');
  const target = { compilerOptions: { dev: true }, preprocess: [() => {}] };
  const source = { compilerOptions: { css: 'external' }, preprocess: [preprocess], [symbol]: true };
  assert.deepEqual(merge(target, source, { arrayMerge: (_target, source) => source }), {
    compilerOptions: { dev: true, css: 'external' }, preprocess: [preprocess], [symbol]: true,
  });
  assert.deepEqual(merge.all([{ values: [1] }, { values: [2] }]), { values: [1, 2] });
});
