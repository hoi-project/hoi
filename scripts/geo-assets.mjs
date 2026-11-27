// Builds the data behind the IP country status (#152): IP range → country tables, a time zone → country
// table and a dotted world map. The app ships the output and looks addresses up on the device, so no
// service learns an IP from it (RFC-0007: observations remain local).
//
//   node scripts/geo-assets.mjs --fetch <dir>     download the pinned inputs into <dir> and verify them
//   node scripts/geo-assets.mjs --input <dir>     write apps/web/src/lib/geo/data/ from those inputs
//
// Node built-ins only. The output is committed; tests use small fixtures and never the network.
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync, existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { deflateSync, gunzipSync } from 'node:zlib';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const OUT_DIR = join(ROOT, 'apps/web/src/lib/geo/data');

// sapics/ip-location-db publishes `user-country` daily as GitHub release assets under PDDL-1.0, with SHA-256
// files under a separate release. `user-country` prefers the country an end user is in, which is closest to
// what other services' lookups report about an address.
const USER_COUNTRY = {
  name: 'ip-location-db user-country',
  licence: 'PDDL-1.0',
  home: 'https://github.com/sapics/ip-location-db',
  files: ['user-country-ipv4.csv', 'user-country-ipv6.csv'],
  url: (file) => `https://github.com/sapics/ip-location-db/releases/download/latest/${file}`,
  checksum: (file) => `https://github.com/sapics/ip-location-db/releases/download/checksum/${file}.sha256`,
};
// Natural Earth is public domain. 110m gives shapes; 50m adds label points for states too small for 110m.
const NATURAL_EARTH = {
  name: 'Natural Earth 5.1.2 admin 0 countries',
  licence: 'public domain',
  home: 'https://www.naturalearthdata.com',
  files: {
    'ne_110m_admin_0_countries.geojson': '6866c877d39cba9c357620878839b336d569f8c662d3cfab4cb1dbe2d39c977f',
    'ne_50m_admin_0_countries.geojson': '3e458fc036ad0a66411f2c1e6cac49c5d7bfb81cb1123bc513b22511a2b7fdeb',
  },
  url: (file) => `https://raw.githubusercontent.com/nvkelso/natural-earth-vector/v5.1.2/geojson/${file}`,
};
// The IANA time zone database is public domain.
const TZDATA = {
  name: 'IANA tzdata 2026f',
  licence: 'public domain',
  home: 'https://www.iana.org/time-zones',
  file: 'tzdata2026f.tar.gz',
  sha256: '01b4a91cec5167ed48c11e8ab78f2048cead78ce94b0cafcad0ae8c5a3044d3d',
  url: 'https://data.iana.org/time-zones/releases/tzdata2026f.tar.gz',
};
const TZ_FILES = ['zone.tab', 'backward', 'africa', 'antarctica', 'asia', 'australasia', 'europe', 'northamerica', 'southamerica'];

export const MAGIC = 'HOIC';
export const FORMAT_VERSION = 1;
const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex');

// ---------------------------------------------------------------- addresses

function v4ToInt(text) {
  const parts = text.split('.');
  if (parts.length !== 4 || parts.some(p => !/^\d{1,3}$/.test(p) || Number(p) > 255)) throw new Error(`Invalid IPv4 address: ${text}`);
  return parts.reduce((n, p) => n * 256 + Number(p), 0);
}

function v6ToBig(text) {
  if (!/^[0-9a-f:]+$/i.test(text) || (text.match(/::/g) || []).length > 1) throw new Error(`Invalid IPv6 address: ${text}`);
  const [head, tail] = text.includes('::') ? text.split('::') : [text, undefined];
  const left = head ? head.split(':') : [];
  const right = tail ? tail.split(':') : [];
  const fill = tail === undefined ? 0 : 8 - left.length - right.length;
  const words = [...left, ...Array(fill).fill('0'), ...right];
  if (words.length !== 8 || fill < 0 || words.some(w => !/^[0-9a-f]{1,4}$/i.test(w))) throw new Error(`Invalid IPv6 address: ${text}`);
  return words.reduce((n, w) => (n << 16n) | BigInt(parseInt(w, 16)), 0n);
}

// IANA special-purpose ranges, as refused by publicAddress() in apps/web/src/lib/geo/public-address.ts. A table
// never assigns them a country, even if upstream data lists one.
const SPECIAL_V4 = [
  ['0.0.0.0', 8], ['10.0.0.0', 8], ['100.64.0.0', 10], ['127.0.0.0', 8], ['169.254.0.0', 16], ['172.16.0.0', 12],
  ['192.0.0.0', 24], ['192.0.2.0', 24], ['192.88.99.0', 24], ['192.168.0.0', 16], ['198.18.0.0', 15],
  ['198.51.100.0', 24], ['203.0.113.0', 24], ['224.0.0.0', 3],
].map(([a, bits]) => { const s = v4ToInt(a); return [s, s + 2 ** (32 - bits) - 1]; });
// IPv6 in /64 prefix units: only 2000::/3 is global unicast, minus protocol assignments (2001::/23),
// documentation (2001:db8::/32, 3fff::/20) and 6to4 (2002::/16).
const P = (hex) => BigInt(`0x${hex}`) << 48n;
const SPECIAL_V6 = [
  [0n, P('2000') - 1n], [P('4000'), (1n << 64n) - 1n],
  [P('2001'), P('2001') + (0x200n << 32n) - 1n], [P('2001') + (0xdb8n << 32n), P('2001') + (0xdb9n << 32n) - 1n],
  [P('2002'), P('2003') - 1n], [P('3fff'), P('3fff') + (0x1000n << 32n) - 1n],
];

// ---------------------------------------------------------------- range tables

/** Parses `start,end,CC` lines. Rows must be sorted and must not overlap. */
export function parseRanges(csv, family) {
  const rows = [];
  let previous = -1n;
  for (const [i, line] of csv.split('\n').entries()) {
    if (!line.trim()) continue;
    const [a, b, code, extra] = line.trim().split(',');
    if (extra !== undefined || !/^[A-Z]{2}$/.test(code || '')) throw new Error(`Line ${i + 1}: expected start,end,CC`);
    const start = family === 4 ? BigInt(v4ToInt(a)) : v6ToBig(a);
    const end = family === 4 ? BigInt(v4ToInt(b)) : v6ToBig(b);
    if (end < start || start <= previous) throw new Error(`Line ${i + 1}: ranges must be sorted and must not overlap`);
    previous = end;
    rows.push({ start, end, code });
  }
  return rows;
}

/**
 * Segments over the whole key space: IPv4 addresses, or IPv6 /64 prefixes. A /64 takes the country of its
 * lowest address, so a range that does not contain any prefix's first address is dropped.
 */
function segments(rows, family) {
  const max = family === 4 ? 2n ** 32n - 1n : 2n ** 64n - 1n;
  const intervals = [];
  for (const r of rows) {
    let start = r.start, end = r.end;
    if (family === 6) {
      start = (r.start + (1n << 64n) - 1n) >> 64n;
      end = r.end >> 64n;
      if (start > end) continue;
    }
    intervals.push([start, end, r.code]);
  }
  const special = (family === 4 ? SPECIAL_V4.map(([s, e]) => [BigInt(s), BigInt(e)]) : SPECIAL_V6).sort((x, y) => (x[0] < y[0] ? -1 : 1));
  // Cut special ranges out of every interval.
  const cut = [];
  for (const [s, e, code] of intervals) {
    let pieces = [[s, e]];
    for (const [a, b] of special) {
      pieces = pieces.flatMap(([x, y]) => (b < x || a > y ? [[x, y]] : [...(a > x ? [[x, a - 1n]] : []), ...(b < y ? [[b + 1n, y]] : [])]));
    }
    for (const [x, y] of pieces) cut.push([x, y, code]);
  }
  // Fill gaps with "not listed" and merge neighbours with the same country.
  const out = [];
  let cursor = 0n;
  const push = (start, code) => { if (out.length && out[out.length - 1][1] === code) return; out.push([start, code]); };
  for (const [s, e, code] of cut) {
    if (s > cursor) push(cursor, '');
    push(s, code);
    cursor = e + 1n;
  }
  if (cursor <= max) push(cursor, '');
  return out;
}

function leb128(value, bytes) {
  let v = value;
  do {
    let byte = Number(v & 0x7fn);
    v >>= 7n;
    if (v) byte |= 0x80;
    bytes.push(byte);
  } while (v);
}

/** Encodes rows into the deflated table format read by apps/web/src/lib/geo/ip-table.ts. */
export function encodeTable(rows, family) {
  if (family !== 4 && family !== 6) throw new Error('family must be 4 or 6');
  const segs = segments(rows, family);
  const codes = [...new Set(segs.map(([, c]) => c).filter(Boolean))].sort();
  if (codes.length > 255) throw new Error('A table holds at most 255 codes');
  const index = new Map(codes.map((c, i) => [c, i + 1]));
  const header = Buffer.alloc(12);
  header.write(MAGIC, 0, 'ascii');
  header.writeUInt8(FORMAT_VERSION, 4);
  header.writeUInt8(family, 5);
  header.writeUInt16LE(codes.length, 6);
  header.writeUInt32LE(segs.length, 8);
  const deltas = [];
  let previous = 0n;
  for (const [start] of segs) { leb128(start - previous, deltas); previous = start; }
  const raw = Buffer.concat([header, Buffer.from(codes.join(''), 'ascii'), Buffer.from(deltas), Buffer.from(segs.map(([, c]) => (c ? index.get(c) : 0)))]);
  return { bytes: deflateSync(raw, { level: 9 }), segments: segs.length, codes };
}

// ---------------------------------------------------------------- time zones

/** Reads members of an uncompressed ustar archive. */
export function untar(buffer, wanted) {
  const files = {};
  for (let at = 0; at + 512 <= buffer.length;) {
    const name = buffer.toString('ascii', at, at + 100).replace(/\0.*$/s, '');
    if (!name) break;
    const size = parseInt(buffer.toString('ascii', at + 124, at + 136).replace(/\0.*$/s, '').trim() || '0', 8);
    if (wanted.includes(name)) files[name] = buffer.toString('utf8', at + 512, at + 512 + size);
    at += 512 + Math.ceil(size / 512) * 512;
  }
  return files;
}

/**
 * Maps every zone in zone.tab to its country, then each Link name to the country of the zone it resolves to.
 * zone.tab wins over links. `Etc/*` and other zones without a country stay unmapped.
 */
export function buildTzMap(zoneTab, linkTexts) {
  const map = new Map();
  for (const line of zoneTab.split('\n')) {
    if (!line.trim() || line.startsWith('#')) continue;
    const [code, , zone] = line.split('\t');
    if (!/^[A-Z]{2}$/.test(code || '') || !zone) throw new Error(`Invalid zone.tab line: ${line}`);
    if (!map.has(zone)) map.set(zone, code);
  }
  const links = new Map();
  for (const text of linkTexts) {
    for (const line of text.split('\n')) {
      const [kind, target, name] = line.replace(/#.*/, '').trim().split(/\s+/);
      if (kind === 'Link' && target && name) links.set(name, target);
    }
  }
  for (const name of links.keys()) {
    if (map.has(name) || name.startsWith('Etc/')) continue;
    let target = name;
    for (let hops = 0; hops < 8 && links.has(target) && !map.has(target); hops++) target = links.get(target);
    if (map.has(target)) map.set(name, map.get(target));
  }
  return Object.fromEntries([...map].sort(([a], [b]) => (a < b ? -1 : 1)));
}

// ---------------------------------------------------------------- world map

// Equal Earth (Šavrič, Patterson, Jenny 2018).
const A1 = 1.340264, A2 = -0.081106, A3 = 0.000893, A4 = 0.003796, M = Math.sqrt(3) / 2;
export function equalEarth(lon, lat) {
  const l = (lon * Math.PI) / 180;
  const t = Math.asin(M * Math.sin((lat * Math.PI) / 180));
  const t2 = t * t, t6 = t2 * t2 * t2;
  return [(l * Math.cos(t)) / (M * (A1 + 3 * A2 * t2 + t6 * (7 * A3 + 9 * A4 * t2))), t * (A1 + A2 * t2 + t6 * (A3 + A4 * t2))];
}

const polygonsOf = (geometry) => (geometry.type === 'Polygon' ? [geometry.coordinates] : geometry.type === 'MultiPolygon' ? geometry.coordinates : []);
const codeOf = (p) => (/^[A-Z]{2}$/.test(p.ISO_A2_EH || '') ? p.ISO_A2_EH : /^[A-Z]{2}$/.test(p.ISO_A2 || '') ? p.ISO_A2 : '');
const ringArea = (ring) => ring.reduce((a, [x, y], i) => { const [u, v] = ring[(i + 1) % ring.length]; return a + x * v - u * y; }, 0) / 2;

function inRing(x, y, ring) {
  let inside = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const [xi, yi] = ring[i], [xj, yj] = ring[j];
    if ((yi > y) !== (yj > y) && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi) inside = !inside;
  }
  return inside;
}

const round1 = (n) => Math.round(n * 10) / 10;

function pathOf(polygons) {
  let d = '';
  for (const polygon of polygons) {
    for (const ring of polygon) {
      let px = 0, py = 0, first = true;
      for (const [x, y] of ring) {
        const rx = Math.round(x * 10), ry = Math.round(y * 10);
        if (first) { d += `M${rx / 10} ${ry / 10}`; first = false; }
        else {
          const dx = rx - px, dy = ry - py;
          if (!dx && !dy) continue;
          d += `l${dx / 10} ${dy / 10}`;
        }
        px = rx; py = ry;
      }
      d += 'z';
    }
  }
  return d.replace(/ -/g, '-');
}

// Land entirely south of this latitude is left out: it would double the map's height for little use.
const SOUTH_LIMIT = -60;

/**
 * Projects countries into a 1000-unit-wide Equal Earth frame and samples land dots on a grid. Each country
 * gets its shape, the box of its largest polygon (for a close-up) and a label point.
 */
export function buildWorldMap(shapes, labels = { features: [] }, { width = 1000, step = 6, margin = 4 } = {}) {
  const [xMax] = equalEarth(180, 0);
  const scale = (width / 2) / xMax;
  const raw = (lon, lat) => { const [x, y] = equalEarth(lon, lat); return [width / 2 + x * scale, -y * scale]; };
  const countries = [];
  for (const f of shapes.features) {
    const code = codeOf(f.properties);
    const kept = polygonsOf(f.geometry).filter(poly => poly[0].some(([, lat]) => lat > SOUTH_LIMIT));
    const polygons = kept.map(poly => poly.map(ring => ring.map(([lon, lat]) => raw(lon, lat))));
    if (!polygons.length) continue;
    countries.push({ code, polygons, label: [f.properties.LABEL_X, f.properties.LABEL_Y] });
  }
  let top = Infinity, bottom = -Infinity;
  for (const c of countries) for (const poly of c.polygons) for (const [, y] of poly[0]) { top = Math.min(top, y); bottom = Math.max(bottom, y); }
  const shift = margin - top;
  const height = Math.ceil(bottom - top + 2 * margin);
  for (const c of countries) c.polygons = c.polygons.map(poly => poly.map(ring => ring.map(([x, y]) => [x, y + shift])));
  const point = (lon, lat) => { const [x, y] = raw(lon, lat); return [round1(x), round1(y + shift)]; };

  // Boxes for a fast reject before ray casting.
  for (const c of countries) {
    c.boxes = c.polygons.map(poly => poly[0].reduce((b, [x, y]) => [Math.min(b[0], x), Math.min(b[1], y), Math.max(b[2], x), Math.max(b[3], y)], [Infinity, Infinity, -Infinity, -Infinity]));
  }
  const codes = [''];
  const codeIndex = new Map([['', 0]]);
  const indexOf = (code) => { if (!codeIndex.has(code)) { codeIndex.set(code, codes.length); codes.push(code); } return codeIndex.get(code); };
  const rows = [];
  const dotCodes = [];
  const cols = Math.floor(width / step);
  for (let r = 0; r * step + step / 2 < height; r++) {
    const y = r * step + step / 2;
    const runs = [];
    let runStart = -1;
    for (let col = 0; col <= cols; col++) {
      const x = col * step + step / 2;
      let hit = null;
      if (col < cols) {
        for (const c of countries) {
          if (c.polygons.some((poly, i) => { const b = c.boxes[i]; return x >= b[0] && x <= b[2] && y >= b[1] && y <= b[3] && inRing(x, y, poly[0]) && !poly.slice(1).some(hole => inRing(x, y, hole)); })) { hit = c; break; }
        }
      }
      if (hit) {
        if (runStart < 0) runStart = col;
        dotCodes.push(indexOf(hit.code));
      } else if (runStart >= 0) {
        runs.push(runStart, col - runStart);
        runStart = -1;
      }
    }
    if (runs.length) rows.push([r, ...runs]);
  }

  const out = {};
  for (const c of countries) {
    if (!c.code || out[c.code]) continue;
    const main = c.polygons.map((poly, i) => [Math.abs(ringArea(poly[0])), i]).sort((a, b) => b[0] - a[0])[0][1];
    const box = c.boxes[main].map(round1);
    out[c.code] = { p: point(...c.label), b: box, d: pathOf(c.polygons) };
  }
  // States too small for the shape file get a label point only.
  for (const f of labels.features) {
    const code = codeOf(f.properties);
    if (!code || out[code] || typeof f.properties.LABEL_X !== 'number' || f.properties.LABEL_Y <= SOUTH_LIMIT) continue;
    out[code] = { p: point(f.properties.LABEL_X, f.properties.LABEL_Y) };
  }
  const sorted = Object.fromEntries(Object.entries(out).sort(([a], [b]) => (a < b ? -1 : 1)));
  return { v: 1, w: width, h: height, step, codes, rows, dots: dotCodes, countries: sorted };
}

// ---------------------------------------------------------------- CLI

async function download(url) {
  const response = await fetch(url, { redirect: 'follow' });
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  return { bytes: Buffer.from(await response.arrayBuffer()), modified: response.headers.get('last-modified') };
}

async function fetchInputs(dir) {
  mkdirSync(dir, { recursive: true });
  const record = { userCountry: {} };
  for (const file of USER_COUNTRY.files) {
    const { bytes, modified } = await download(USER_COUNTRY.url(file));
    const { bytes: sum } = await download(USER_COUNTRY.checksum(file));
    const expected = sum.toString('utf8').trim().split(/\s+/)[0];
    if (sha256(bytes) !== expected) throw new Error(`${file}: SHA-256 does not match the published checksum`);
    writeFileSync(join(dir, file), bytes);
    record.userCountry[file] = { sha256: expected, modified: modified ? new Date(modified).toISOString().slice(0, 10) : null };
  }
  for (const [file, expected] of Object.entries(NATURAL_EARTH.files)) {
    const { bytes } = await download(NATURAL_EARTH.url(file));
    if (sha256(bytes) !== expected) throw new Error(`${file}: SHA-256 does not match the pinned value`);
    writeFileSync(join(dir, file), bytes);
  }
  const { bytes } = await download(TZDATA.url);
  if (sha256(bytes) !== TZDATA.sha256) throw new Error(`${TZDATA.file}: SHA-256 does not match the pinned value`);
  writeFileSync(join(dir, TZDATA.file), bytes);
  writeFileSync(join(dir, 'fetched.json'), `${JSON.stringify(record, null, 2)}\n`);
  console.log(`Inputs saved to ${dir}`);
}

function writeOutputs(dir, out = OUT_DIR) {
  const read = (file) => readFileSync(join(dir, file));
  const fetched = existsSync(join(dir, 'fetched.json')) ? JSON.parse(read('fetched.json')) : { userCountry: {} };
  for (const [file, expected] of Object.entries(NATURAL_EARTH.files)) if (sha256(read(file)) !== expected) throw new Error(`${file}: unexpected SHA-256`);
  if (sha256(read(TZDATA.file)) !== TZDATA.sha256) throw new Error(`${TZDATA.file}: unexpected SHA-256`);
  mkdirSync(out, { recursive: true });
  const outputs = {};
  const save = (name, bytes, extra) => { writeFileSync(join(out, name), bytes); outputs[name] = { bytes: bytes.length, sha256: sha256(bytes), ...extra }; };

  for (const [family, file] of [[4, 'user-country-ipv4.csv'], [6, 'user-country-ipv6.csv']]) {
    const table = encodeTable(parseRanges(read(file).toString('utf8'), family), family);
    save(`ip-country-v${family}.bin`, table.bytes, { segments: table.segments, codes: table.codes.length });
  }
  const tar = gunzipSync(read(TZDATA.file));
  const tz = untar(tar, TZ_FILES);
  for (const name of TZ_FILES) if (!(name in tz)) throw new Error(`${TZDATA.file}: ${name} is missing`);
  const zones = buildTzMap(tz['zone.tab'], TZ_FILES.slice(1).map(name => tz[name]));
  save('tz-country.json', Buffer.from(`${JSON.stringify(zones)}\n`), { zones: Object.keys(zones).length });
  const map = buildWorldMap(JSON.parse(read('ne_110m_admin_0_countries.geojson')), JSON.parse(read('ne_50m_admin_0_countries.geojson')));
  save('world-map.json', Buffer.from(`${JSON.stringify(map)}\n`), { dots: map.dots.length, countries: Object.keys(map.countries).length });

  const dates = Object.values(fetched.userCountry).map(f => f.modified).filter(Boolean).sort();
  const manifest = {
    format: FORMAT_VERSION,
    date: dates[dates.length - 1] || null,
    sources: [
      { name: USER_COUNTRY.name, licence: USER_COUNTRY.licence, home: USER_COUNTRY.home, files: Object.fromEntries(USER_COUNTRY.files.map(f => [f, { url: USER_COUNTRY.url(f), sha256: sha256(read(f)), date: fetched.userCountry[f]?.modified ?? null }])) },
      { name: NATURAL_EARTH.name, licence: NATURAL_EARTH.licence, home: NATURAL_EARTH.home, files: Object.fromEntries(Object.entries(NATURAL_EARTH.files).map(([f, s]) => [f, { url: NATURAL_EARTH.url(f), sha256: s }])) },
      { name: TZDATA.name, licence: TZDATA.licence, home: TZDATA.home, files: { [TZDATA.file]: { url: TZDATA.url, sha256: TZDATA.sha256 } } },
    ],
    outputs,
  };
  writeFileSync(join(out, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
  console.log(Object.entries(outputs).map(([name, o]) => `${name}: ${o.bytes} bytes`).join('\n'));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [flag, dir] = process.argv.slice(2);
  if (flag === '--fetch' && dir) await fetchInputs(resolve(dir));
  else if (flag === '--input' && dir) writeOutputs(resolve(dir));
  else {
    console.error('Usage: node scripts/geo-assets.mjs --fetch <dir> | --input <dir>');
    process.exit(2);
  }
}
