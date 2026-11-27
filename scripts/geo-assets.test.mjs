// Fixtures use ISO 3166-1 user-assigned codes (XA, XB, XC) and made-up zone names only.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { inflateSync } from 'node:zlib';
import { buildTzMap, buildWorldMap, encodeTable, equalEarth, parseRanges, untar, MAGIC } from './geo-assets.mjs';

function decode(bytes) {
  const raw = inflateSync(bytes);
  assert.equal(raw.toString('ascii', 0, 4), MAGIC);
  const family = raw[5], count = raw.readUInt16LE(6), n = raw.readUInt32LE(8);
  const codes = [];
  for (let i = 0; i < count; i++) codes.push(raw.toString('ascii', 12 + 2 * i, 14 + 2 * i));
  let at = 12 + 2 * count;
  const starts = [];
  let value = 0n;
  for (let i = 0; i < n; i++) {
    let delta = 0n, shift = 0n, byte;
    do { byte = raw[at++]; delta |= BigInt(byte & 0x7f) << shift; shift += 7n; } while (byte & 0x80);
    value += delta;
    starts.push(value);
  }
  const index = [...raw.subarray(at, at + n)];
  assert.equal(at + n, raw.length, 'no trailing bytes');
  return { family, segments: starts.map((s, i) => [s, index[i] ? codes[index[i] - 1] : '']) };
}

const v4 = (text) => BigInt(text.split('.').reduce((n, p) => n * 256 + Number(p), 0));

test('IPv4 tables merge neighbours, fill gaps and start at zero', () => {
  const rows = parseRanges('1.0.0.0,1.0.0.255,XA\n1.0.1.0,1.0.1.255,XA\n1.0.3.0,1.0.3.255,XB\n', 4);
  const { segments, family } = decode(encodeTable(rows, 4).bytes);
  assert.equal(family, 4);
  assert.deepEqual(segments.slice(0, 4), [[0n, ''], [v4('1.0.0.0'), 'XA'], [v4('1.0.2.0'), ''], [v4('1.0.3.0'), 'XB']]);
  assert.deepEqual(segments[4], [v4('1.0.4.0'), '']);
});

test('special-use IPv4 ranges never get a country', () => {
  const rows = parseRanges('10.0.0.0,10.255.255.255,XA\n198.51.100.0,198.51.100.255,XB\n203.0.112.0,203.0.114.255,XC\n', 4);
  const { segments } = decode(encodeTable(rows, 4).bytes);
  const at = (ip) => { let code = ''; for (const [s, c] of segments) if (s <= v4(ip)) code = c; return code; };
  assert.equal(at('10.1.2.3'), '');
  assert.equal(at('198.51.100.7'), '');
  assert.equal(at('203.0.112.255'), 'XC');
  assert.equal(at('203.0.113.1'), '');
  assert.equal(at('203.0.114.0'), 'XC');
});

test('an IPv6 /64 takes the country of its lowest address', () => {
  const rows = parseRanges([
    '2001:db8::,2001:db8:ffff:ffff:ffff:ffff:ffff:ffff,XC', // documentation
    '2400::,2400::ffff:ffff:ffff:ffff,XA', // exactly one /64
    '2400:0:0:1::8000,2400:0:0:2:ffff:ffff:ffff:ffff,XB', // starts inside a /64: only the next one counts
    '2400:0:0:5::1,2400:0:0:5::ff,XC', // inside one /64 but not at its start: dropped
  ].join('\n'), 6);
  const { segments, family } = decode(encodeTable(rows, 6).bytes);
  assert.equal(family, 6);
  const prefix = (hex) => BigInt(`0x${hex}`);
  assert.deepEqual(segments, [[0n, ''], [prefix('2400000000000000'), 'XA'], [prefix('2400000000000001'), ''], [prefix('2400000000000002'), 'XB'], [prefix('2400000000000003'), '']]);
});

test('encoding is deterministic', () => {
  const csv = '1.0.0.0,1.0.0.255,XA\n2.0.0.0,2.0.0.255,XB\n';
  assert.deepEqual(encodeTable(parseRanges(csv, 4), 4).bytes, encodeTable(parseRanges(csv, 4), 4).bytes);
});

test('unsorted, overlapping or malformed rows are refused', () => {
  assert.throws(() => parseRanges('1.0.1.0,1.0.1.255,XA\n1.0.0.0,1.0.0.255,XB', 4), /sorted/);
  assert.throws(() => parseRanges('1.0.0.0,1.0.1.255,XA\n1.0.1.0,1.0.2.255,XB', 4), /overlap/);
  assert.throws(() => parseRanges('1.0.0.0,1.0.0.255,xa', 4), /CC/);
  assert.throws(() => parseRanges('1.0.0.0,1.0.0.256,XA', 4), /Invalid IPv4/);
  assert.throws(() => parseRanges('2400::1::2,2400::3,XA', 6), /Invalid IPv6/);
});

test('time zones map through zone.tab first, then links', () => {
  const zoneTab = '# comment\nXA\t+0000+00000\tTest/Alpha\nXB\t+0000+00000\tTest/Beta\nXC\t+0000+00000\tTest/Gamma\n';
  const backward = 'Link\tTest/Alpha\tTest/Old_Alpha\t# renamed\nLink Test/Old_Alpha Test/Older_Alpha\nLink Test/Beta Test/Gamma\nLink Etc/UTC Etc/Universal\n';
  const map = buildTzMap(zoneTab, [backward]);
  assert.deepEqual(map, { 'Test/Alpha': 'XA', 'Test/Beta': 'XB', 'Test/Gamma': 'XC', 'Test/Old_Alpha': 'XA', 'Test/Older_Alpha': 'XA' });
});

test('untar reads named members of a ustar archive', () => {
  const header = (name, size) => {
    const h = Buffer.alloc(512);
    h.write(name, 0, 'ascii');
    h.write(size.toString(8).padStart(11, '0'), 124, 'ascii');
    return h;
  };
  const body = Buffer.alloc(512); body.write('hello', 0, 'ascii');
  const tar = Buffer.concat([header('zone.tab', 5), body, header('other', 0), Buffer.alloc(1024)]);
  assert.deepEqual(untar(tar, ['zone.tab']), { 'zone.tab': 'hello' });
});

test('the world map samples land dots and keeps each code once', () => {
  const square = (code, lon, lat, size) => ({
    type: 'Feature', properties: { ISO_A2_EH: code, LABEL_X: lon + size / 2, LABEL_Y: lat + size / 2 },
    geometry: { type: 'Polygon', coordinates: [[[lon, lat], [lon + size, lat], [lon + size, lat + size], [lon, lat + size], [lon, lat]]] },
  });
  const shapes = { features: [square('XA', -40, -10, 30), square('XB', 20, 10, 20), square('-99', 100, 0, 10)] };
  const labels = { features: [{ type: 'Feature', properties: { ISO_A2_EH: 'XC', LABEL_X: 60, LABEL_Y: -20 }, geometry: { type: 'Polygon', coordinates: [] } }] };
  const map = buildWorldMap(shapes, labels, { step: 6 });
  assert.deepEqual(Object.keys(map.countries), ['XA', 'XB', 'XC']);
  assert.ok(map.countries.XA.d.startsWith('M') && map.countries.XA.b.length === 4);
  assert.equal(map.countries.XC.d, undefined, 'label-only states have no shape');
  const counts = map.dots.reduce((c, i) => ((c[map.codes[i]] = (c[map.codes[i]] || 0) + 1), c), {});
  assert.ok(counts.XA > counts.XB && counts.XB > 0);
  assert.ok(counts[''] > 0, 'land without a code still draws');
  const ranLength = map.rows.reduce((n, [, ...runs]) => n + runs.filter((_, i) => i % 2).reduce((a, b) => a + b, 0), 0);
  assert.equal(ranLength, map.dots.length);
  assert.deepEqual(equalEarth(0, 0), [0, 0]);
});
