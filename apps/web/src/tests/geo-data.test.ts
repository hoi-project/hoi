// Checks the committed data files. It never asserts which country an address is in: the data changes
// with each refresh, and tests name no real country.
import { describe, expect, it } from 'vitest';
import { readOutput } from '../../../../scripts/geo-assets.mjs';
import { addressKey, decodeTable, lookup } from '../lib/geo/ip-table';
import { inflate, LIMITS } from '../lib/geo/load';
import { parseZoneTable } from '../lib/geo/tz';
import { mapDots, parseWorldMap } from '../lib/geo/world-map';
import manifest from '../lib/geo/data/manifest.json';

const text = (name: string) => new TextDecoder().decode(readOutput(name));
const table = async (family: 4 | 6) => decodeTable(await inflate(readOutput(`ip-country-v${family}.bin`), LIMITS.inflated));

describe('the bundled geo data', () => {
  it.each([4, 6] as const)('IPv%i table decodes and matches its manifest entry', async family => {
    const t = await table(family);
    const entry = manifest.outputs[`ip-country-v${family}.bin`];
    expect(t.family).toBe(family);
    expect(t.hi.length).toBe(entry.segments);
    expect(t.codes.length).toBe(entry.codes);
    expect(t.codes.every(c => /^[A-Z]{2}$/.test(c))).toBe(true);
  });

  it('lists some country for ordinary public addresses', async () => {
    // Published resolver addresses, used only as synthetic public IPs (as in browser-egress.test.ts).
    for (const ip of ['1.1.1.1', '2606:4700:4700::1111']) {
      const key = addressKey(ip)!;
      expect(lookup(await table(key.family), key)).toMatch(/^[A-Z]{2}$/);
    }
  });

  it('the time zone table and map parse, and every map code is well formed', () => {
    const zones = parseZoneTable(JSON.parse(text('tz-country.json')));
    expect(Object.keys(zones).length).toBe(manifest.outputs['tz-country.json'].zones);
    expect(Object.keys(zones).some(z => z.startsWith('Etc/'))).toBe(false);
    const map = parseWorldMap(JSON.parse(text('world-map.json')));
    expect(mapDots(map).length).toBe(manifest.outputs['world-map.json'].dots);
    for (const c of Object.values(map.countries)) {
      expect(c.p[0]).toBeGreaterThanOrEqual(0);
      expect(c.p[0]).toBeLessThanOrEqual(map.w);
    }
  });

  it('records sources with licences and hashes', () => {
    expect(manifest.sources.map(s => s.licence)).toEqual(['PDDL-1.0', 'public domain', 'public domain']);
    for (const source of manifest.sources) for (const file of Object.values(source.files)) expect(file.sha256).toMatch(/^[0-9a-f]{64}$/);
  });
});
