// Serves fixture tables built with user-assigned codes (XA, XB) in place of the bundled data.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { encodeTable, parseRanges } from '../../../../scripts/geo-assets.mjs';

const v4 = encodeTable(parseRanges('1.0.0.0,1.255.255.255,XA\n', 4), 4).bytes;
const v6 = encodeTable(parseRanges('2400::,2400:ffff:ffff:ffff:ffff:ffff:ffff:ffff,XB\n', 6), 6).bytes;
const map = { v: 1, w: 100, h: 50, step: 10, codes: ['', 'XA'], rows: [[1, 2, 1]], dots: [1], countries: { XA: { p: [25, 15] } } };

function serve(fail = false) {
  const call = vi.fn(async (url: string) => {
    if (fail) return new Response('', { status: 500 });
    if (url.includes('ip-country-v4')) return new Response(new Uint8Array(v4));
    if (url.includes('ip-country-v6')) return new Response(new Uint8Array(v6));
    if (url.includes('tz-country')) return new Response(JSON.stringify({ 'Test/Alpha': 'XA' }));
    if (url.includes('world-map')) return new Response(JSON.stringify(map));
    return new Response('', { status: 404 });
  });
  vi.stubGlobal('fetch', call);
  return call;
}

beforeEach(() => { vi.resetModules(); });
afterEach(() => { vi.unstubAllGlobals(); vi.restoreAllMocks(); });

describe('country lookups on this device', () => {
  it('loads nothing until asked, then each file once', async () => {
    const call = serve();
    const geo = await import('../lib/ip-location.svelte');
    expect(geo.lookupOf('1.2.3.4')).toEqual({ state: 'idle' });
    expect(call).not.toHaveBeenCalled();
    await geo.locate('1.2.3.4');
    await geo.locate('1.9.9.9');
    expect(call.mock.calls.map(([url]) => String(url).replace(/\?.*$/, '').replace(/.*\//, '')).sort()).toEqual(['ip-country-v4.bin', 'tz-country.json', 'world-map.json']);
    expect(geo.lookupOf('1.2.3.4')).toEqual({ state: 'ready', code: 'XA' });
    expect(geo.lookupOf('2.0.0.1')).toEqual({ state: 'ready', code: null });
    expect(geo.lookupOf('2400::1')).toEqual({ state: 'idle' });
    await geo.locate('2400::1');
    expect(geo.lookupOf('2400::1')).toEqual({ state: 'ready', code: 'XB' });
    expect(call).toHaveBeenCalledTimes(4);
    expect(geo.worldMap()?.countries.XA.p).toEqual([25, 15]);
  });

  it('never loads anything for local or special-use addresses', async () => {
    const call = serve();
    const geo = await import('../lib/ip-location.svelte');
    await geo.locate('192.168.1.20');
    await geo.locate('2001:db8::25');
    expect(call).not.toHaveBeenCalled();
    expect(geo.lookupOf('192.168.1.20')).toEqual({ state: 'local' });
  });

  it('compares with the device zone once the zone table is loaded', async () => {
    serve();
    vi.spyOn(Intl, 'DateTimeFormat').mockReturnValue({ resolvedOptions: () => ({ timeZone: 'Test/Alpha' }) } as unknown as Intl.DateTimeFormat);
    const geo = await import('../lib/ip-location.svelte');
    expect(geo.deviceZone()).toEqual({ zone: 'Test/Alpha', code: null });
    await geo.locate('1.2.3.4');
    expect(geo.deviceZone()).toEqual({ zone: 'Test/Alpha', code: 'XA' });
  });

  it('reports a failure and tries again only when asked again', async () => {
    const failing = serve(true);
    const geo = await import('../lib/ip-location.svelte');
    await geo.locate('1.2.3.4');
    expect(geo.lookupOf('1.2.3.4')).toEqual({ state: 'failed' });
    expect(geo.ipLocation.error).toMatch(/could not be loaded/);
    const calls = failing.mock.calls.length;
    expect(geo.lookupOf('1.2.3.4')).toEqual({ state: 'failed' });
    expect(failing).toHaveBeenCalledTimes(calls);
    serve();
    await geo.locate('1.2.3.4');
    expect(geo.lookupOf('1.2.3.4')).toEqual({ state: 'ready', code: 'XA' });
  });
});
