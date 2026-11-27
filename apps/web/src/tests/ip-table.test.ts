// Fixtures use ISO 3166-1 user-assigned codes (XA, XB, XC) only.
import { beforeAll, describe, expect, it } from 'vitest';
import { encodeTable, parseRanges } from '../../../../scripts/geo-assets.mjs';
import { addressKey, decodeTable, lookup, type CountryTable } from '../lib/geo/ip-table';
import { inflate } from '../lib/geo/load';

const raw = (csv: string, family: 4 | 6) => inflate(new Uint8Array(encodeTable(parseRanges(csv, family), family).bytes), 1 << 20);
const country = (t: CountryTable, ip: string) => { const key = addressKey(ip); return key ? lookup(t, key) : 'refused'; };

describe('IP country tables', () => {
  let v4: CountryTable, v6: CountryTable, good: Uint8Array;
  beforeAll(async () => {
    v4 = decodeTable(await raw('1.0.0.0,1.0.0.255,XA\n1.0.1.0,1.0.1.255,XB\n223.255.255.0,223.255.255.255,XC\n', 4));
    v6 = decodeTable(await raw('2400::,2400:0:0:1:ffff:ffff:ffff:ffff,XA\n2400:0:0:2::,2400:0:0:2:ffff:ffff:ffff:ffff,XB\n3ffe:ffff:ffff:ffff::,3ffe:ffff:ffff:ffff:ffff:ffff:ffff:ffff,XC\n', 6));
    good = await raw('1.0.0.0,1.0.0.255,XA\n', 4);
  });

  it('finds the segment that holds an address, including its edges', () => {
    expect(country(v4, '0.255.255.255')).toBe('refused');
    expect(country(v4, '1.0.0.0')).toBe('XA');
    expect(country(v4, '1.0.0.255')).toBe('XA');
    expect(country(v4, '1.0.1.0')).toBe('XB');
    expect(country(v4, '1.0.2.0')).toBeNull();
    expect(country(v4, '0.0.0.1')).toBe('refused');
    expect(country(v4, '223.255.254.255')).toBeNull();
    expect(country(v4, '223.255.255.255')).toBe('XC');
  });

  it('keys IPv6 by /64 and maps IPv4-mapped addresses to the IPv4 table', () => {
    expect(country(v6, '2400::1')).toBe('XA');
    expect(country(v6, '2400:0:0:1:ffff:ffff:ffff:ffff')).toBe('XA');
    expect(country(v6, '2400:0:0:2::')).toBe('XB');
    expect(country(v6, '2400:0:0:3::')).toBeNull();
    expect(country(v6, '3ffe:ffff:ffff:ffff:1::')).toBe('XC');
    expect(addressKey('::ffff:1.0.0.7')).toEqual({ family: 4, hi: 0, lo: 0x01000007 });
    expect(lookup(v6, addressKey('1.0.0.7')!)).toBeNull();
  });

  it.each(['10.0.0.1', '127.0.0.1', '100.64.0.1', '169.254.0.1', '192.168.1.1', '192.0.2.1', '198.51.100.1', '203.0.113.1',
    '224.0.0.1', '255.255.255.255', '::', '::1', 'fc00::1', 'fe80::1', '2001:db8::25', '2002::1', '3fff::1', '::ffff:10.0.0.1', 'nope', '1.2.3'])(
    'never looks up %s', ip => { expect(addressKey(ip)).toBeNull(); });

  it('refuses damaged tables', () => {
    const damaged = (edit: (b: Uint8Array) => Uint8Array) => () => decodeTable(edit(good.slice()));
    expect(damaged(b => b)).not.toThrow();
    expect(damaged(b => { b[0] = 0x58; return b; })).toThrow(/unknown format/);
    expect(damaged(b => { b[4] = 2; return b; })).toThrow(/version/);
    expect(damaged(b => { b[5] = 5; return b; })).toThrow(/family/);
    expect(damaged(b => b.slice(0, b.length - 1))).toThrow(/damaged/);
    expect(damaged(b => { b[12] = 0x61; return b; })).toThrow(/bad code/);
    expect(damaged(b => { b[b.length - 1] = 9; return b; })).toThrow(/bad index/);
    expect(damaged(b => new Uint8Array([...b, 0]))).toThrow(/length/);
  });

  it('refuses starts that do not increase', () => {
    // Header for one code and two segments, both starting at 0.
    const raw = new Uint8Array([0x48, 0x4f, 0x49, 0x43, 1, 4, 1, 0, 2, 0, 0, 0, 0x58, 0x41, 0, 0, 0, 1]);
    expect(() => decodeTable(raw)).toThrow(/order/);
  });
});
