import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { test } from 'node:test';
import { loadSbom } from '../apps/site/scripts/sbom.mjs';

const fromWeb = createRequire(new URL('../apps/web/package.json', import.meta.url));
const fromVite = createRequire(fromWeb.resolve('vite'));
const fromGlob = createRequire(fromVite.resolve('tinyglobby'));
const fromDirectoryReader = createRequire(fromGlob.resolve('fdir'));
const fromSvelteCheck = createRequire(fromWeb.resolve('svelte-check/package.json'));
const fromCheckReader = createRequire(fromSvelteCheck.resolve('fdir'));
const parser = new URL('../vendor/picomatch/lib/parse.js', import.meta.url);
const digest = (file) => createHash('sha256').update(readFileSync(file)).digest('hex');
const match = fromVite('picomatch');

test('Vite, tinyglobby, fdir and the Svelte checker all use the reviewed glob parser', () => {
  for (const loader of [fromVite, fromGlob, fromDirectoryReader, fromCheckReader]) {
    assert.equal(digest(loader.resolve('picomatch/lib/parse.js')), digest(parser));
  }
});

test('the build matcher selects normal source paths and handles repeated file-name components', () => {
  const sources = match('src/@(app|site)/*.{svelte,ts}');
  assert.ok(sources('src/site/Home.svelte'));
  assert.ok(sources('src/app/index.ts'));
  assert.equal(sources('src/dist/index.ts'), false);
  assert.equal(sources('src/site/.cache.ts'), false);
  assert.ok(match('icons/+(v)-logo.svg')('icons/vvv-logo.svg'));
  const icons = match('icons/+(icon*)-light.svg');
  assert.ok(icons('icons/icon-header-light.svg'));
  assert.equal(icons('icons/label-light.svg'), false);
});

test('one wildcard branch is compiled once, including for callers that change the recursion limit', () => {
  for (const opts of [{}, { maxExtglobRecursion: false }]) {
    const output = (glob) => match.parse(glob, opts).output;
    assert.equal(output('icons/+(icon*)-light.svg'), output('icons/@(icon*)-light.svg').replace('(icon', '(?:icon'));
    assert.equal(output('icons/*(icon*)-light.svg'), output('icons/?(icon*)-light.svg'));
  }
});

test('the frontend inventory identifies the vendored code used by both builds', () => {
  const data = loadSbom();
  const key = 'npm:picomatch@file:vendor/picomatch';
  assert.equal(data.packages[key].version, '4.0.7');
  assert.equal(data.packages[key].vendored, 'vendor/picomatch');
  for (const name of ['web-app', 'website']) {
    assert.ok(data.programs.find(p => p.id === name).npm.some(dep => dep.k === key));
  }
  assert.equal(data.packages['npm:picomatch@4.0.7'], undefined);
});
