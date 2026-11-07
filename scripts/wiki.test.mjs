// The wiki, its SBOM and the network playground: what can be checked without a browser.
// (The pages themselves: apps/site/scripts/wiki-check.mjs; in a browser: apps/site/scripts/e2e.mjs.)
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { test } from 'node:test';
import { renderBytes, renderSeq } from '../apps/site/plugins/wiki-blocks.ts';
import { headingsOf, loadWiki, textOf } from '../apps/site/plugins/wiki.ts';
import { blake3, hex, leadingZeroBits } from '../apps/site/src/wiki/blake3.ts';
import { check, parseCargoLock, parseYaml } from '../apps/site/scripts/sbom.mjs';
import { SCENARIOS, FOR_DEVICE, DEVICES } from '../apps/site/src/playground/scenarios.ts';
import { buildWorld, YOU_AT } from '../apps/site/src/playground/world.ts';

const root = resolve(import.meta.dirname, '..');
const site = resolve(root, 'apps/site') + '/';

test('every page in nav.json exists, once', () => {
  const pages = loadWiki(site);
  assert.ok(pages.length >= 40);
  assert.equal(new Set(pages.map((p) => p.href)).size, pages.length);
  for (const p of pages) assert.ok(existsSync(site + p.file), p.file);
});

test('sequence diagrams render lanes, arrows, notes and phases', () => {
  const html = renderSeq(' data-title="Hi"', `
lanes: Browser | Node
-- Connect --
Browser -> Node: <code>hello</code>
Node --> Browser @coral: reply
Node: checks it
Browser .. Node: both`, 'test');
  assert.match(html, /class="wk-seq lanes-2"/);
  assert.match(html, /class="wk-seq-row msg s1 e2" role="listitem"/);
  assert.match(html, /class="wk-seq-row msg s1 e2 back reply c-coral"/);
  assert.match(html, /class="wk-seq-row note n2"/);
  assert.match(html, /class="wk-seq-row note span"/);
  assert.match(html, /class="wk-seq-row phase"/);
  assert.throws(() => renderSeq('', 'lanes: A | B\nA -> C: x', 'test'), /unknown lane "C"/);
  assert.throws(() => renderSeq('', 'A -> B: x', 'test'), /must start with "lanes:/);
});

test('byte layouts render boxes and refuse unknown colors and widths', () => {
  const html = renderBytes(' data-title="T"', 'id | 32 B | sun | 4 | the id\nnonce | 8 B | coral | 1', 'test');
  assert.match(html, /<span class="c-sun w4"><b>id<\/b><i>32 B<\/i><em>the id<\/em><\/span>/);
  assert.throws(() => renderBytes('', 'x | 1 | pink | 1', 'test'), /colour/);
  assert.throws(() => renderBytes('', 'x | 1 | sun | 13', 'test'), /width/);
});

test('headings and text come out of a page for the search index', () => {
  const hs = headingsOf('<section class="wk-sec" id="a"><h2>One &amp; two</h2><h3 id="b">Three</h3></section>');
  assert.deepEqual(hs.map((h) => [h.level, h.id, h.text]), [[2, 'a', 'One & two'], [3, 'b', 'Three']]);
  assert.equal(textOf('<p>a <code>b</code>&nbsp;&rarr;</p>'), 'a b →');
});

test('the demos\' BLAKE3 matches the reference vectors', () => {
  const input = (n) => Uint8Array.from({ length: n }, (_, i) => i % 251);
  assert.equal(hex(blake3(new Uint8Array(0))), 'af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262');
  assert.equal(hex(blake3(input(1))), '2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213');
  assert.equal(hex(blake3(input(1023))), '10108970eeda3eb932baac1428c7a2163b0e924c9a9e25b35bba72b28f70bd11');
  assert.equal(hex(blake3(input(1024))), '42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7');
  assert.equal(leadingZeroBits(Uint8Array.from([0, 0, 0x1f, 0xff])), 19);
});

test('the SBOM parsers read Cargo.lock and pnpm-lock.yaml', () => {
  const lock = parseCargoLock('version = 4\n\n[[package]]\nname = "a"\nversion = "1.0.0"\nsource = "registry+x"\nchecksum = "ab"\ndependencies = [\n "b",\n]\n');
  assert.deepEqual(lock, [{ name: 'a', version: '1.0.0', source: 'registry+x', checksum: 'ab', deps: ['b'] }]);
  const y = parseYaml("importers:\n\n  apps/site:\n    dependencies:\n      animejs:\n        specifier: 4.5.0\n        version: 4.5.0\npackages:\n\n  '@x/y@1.0.0':\n    resolution: {integrity: sha512-abc}\n");
  assert.equal(y.importers['apps/site'].dependencies.animejs.version, '4.5.0');
  assert.equal(y.packages['@x/y@1.0.0'].resolution, '{integrity: sha512-abc}');
});

test('the SBOM matches every lockfile', () => {
  assert.deepEqual(check(), []);
});

test('every playground scenario names devices that exist and code that exists', () => {
  const { actors } = buildWorld();
  for (const d of DEVICES) assert.ok(FOR_DEVICE[d.id].length, d.id);
  for (const s of SCENARIOS) {
    assert.ok(YOU_AT[s.you], `${s.id}: where "you" stand`);
    assert.ok(Object.values(FOR_DEVICE).some((ids) => ids.includes(s.id)), `${s.id}: offered to some device`);
    for (const st of s.steps) {
      const ids = [st.local, ...(st.open ?? []).slice(0, 2), ...st.msgs.flatMap((m) => [m.from, m.to])].filter(Boolean);
      for (const id of ids) assert.ok(actors.has(id), `${s.id} / ${st.title}: no actor "${id}"`);
      assert.ok(st.src.length, `${s.id} / ${st.title}: cites no code`);
      for (const ref of st.src) {
        const [file, lines] = ref.split(':');
        assert.ok(existsSync(resolve(root, file)), `${s.id} / ${st.title}: ${file} doesn't exist`);
        if (lines) {
          const last = Number(lines.split('-').pop());
          const count = readFileSync(resolve(root, file), 'utf8').split('\n').length;
          assert.ok(last <= count, `${s.id} / ${st.title}: ${file} has ${count} lines, not ${last}`);
        }
      }
    }
  }
});
